//! Action execution, parameter binding, and validation.
//!
//! This module implements the full action execution pipeline:
//! 1. Resolve the `<action>` by id from `<interact>`
//! 2. Resolve the endpoint URI (relative → absolute, xml:base precedence)
//! 3. SSRF protection: reject private/loopback IPs
//! 4. Collect and validate parameters
//! 5. Run disclosure evaluation for answer values
//! 6. Serialize request body per enctype (urlencoded, multipart, JSON)
//! 7. Invoke confirm callback if `confirm="true"`
//! 8. Generate Idempotency-Key if needed
//! 9. Track action budget (request count, origin set)
//! 10. Integrate auth provider for `auth="required|optional"`
//! 11. Perform HTTP request with per-action timeout
//! 12. Parse response as ANML document

pub mod builder;
pub mod idempotency;
pub mod params;
pub mod validation;

#[cfg(test)]
mod params_property_test;

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use anml::types::document::AnmlDocument;
use anml::types::elements::AnmlAction;
use anml::types::enums::AuthType;
use tracing::{debug, instrument, warn};
use url::Url;

use crate::config::{
    ActionBudget, AuthProvider, AuthRefreshResult, ConfirmDecision, ConfirmHandler, Origin,
};
use crate::error::AnmlClientError;

use self::params::ParamValue;

// ---------------------------------------------------------------------------
// ActionBudgetTracker — per-document request/origin tracking
// ---------------------------------------------------------------------------

/// Tracks per-document request count and distinct origins against the
/// configured [`ActionBudget`].
#[derive(Debug)]
pub struct ActionBudgetTracker {
    budget: ActionBudget,
    request_count: AtomicU32,
    origins: Mutex<HashSet<String>>,
}

impl ActionBudgetTracker {
    /// Create a new tracker with the given budget.
    pub fn new(budget: ActionBudget) -> Self {
        Self {
            budget,
            request_count: AtomicU32::new(0),
            origins: Mutex::new(HashSet::new()),
        }
    }

    /// Check and record a request to the given origin.
    /// Returns an error if the budget would be exceeded.
    pub fn check_and_record(&self, origin: &str) -> crate::Result<()> {
        // Check request count
        let current = self.request_count.load(Ordering::Relaxed);
        if current >= self.budget.max_requests {
            return Err(AnmlClientError::ActionBudgetExceeded {
                budget_type: "max_requests".into(),
                limit: self.budget.max_requests,
            });
        }

        // Check distinct origins
        {
            let mut origins = self.origins.lock().unwrap_or_else(|e| e.into_inner());
            if !origins.contains(origin) {
                if origins.len() as u32 >= self.budget.max_distinct_origins {
                    return Err(AnmlClientError::ActionBudgetExceeded {
                        budget_type: "max_distinct_origins".into(),
                        limit: self.budget.max_distinct_origins,
                    });
                }
                origins.insert(origin.to_string());
            }
        }

        self.request_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Reset the tracker (e.g., for a new document).
    pub fn reset(&self) {
        self.request_count.store(0, Ordering::Relaxed);
        let mut origins = self.origins.lock().unwrap_or_else(|e| e.into_inner());
        origins.clear();
    }
}

// ---------------------------------------------------------------------------
// SSRF protection
// ---------------------------------------------------------------------------

/// Check if an IP address is private or loopback (SSRF protection).
///
/// Rejects: 127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1, fc00::/7
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                          // 127.0.0.0/8
                || v4.octets()[0] == 10               // 10.0.0.0/8
                || (v4.octets()[0] == 172 && (v4.octets()[1] & 0xF0) == 16) // 172.16.0.0/12
                || (v4.octets()[0] == 192 && v4.octets()[1] == 168) // 192.168.0.0/16
                || v4.is_unspecified()                // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                          // ::1
                || (v6.octets()[0] & 0xFE) == 0xFC    // fc00::/7
                || v6.is_unspecified()                // ::
        }
    }
}

/// Resolve a hostname and check all resolved IPs for SSRF.
/// Returns an error if any resolved IP is private/loopback.
pub async fn check_ssrf(endpoint: &str) -> crate::Result<()> {
    let parsed = Url::parse(endpoint).map_err(|e| AnmlClientError::MalformedDocument {
        detail: format!("invalid endpoint URL '{}': {}", endpoint, e),
    })?;

    let host = parsed.host_str().ok_or_else(|| AnmlClientError::MalformedDocument {
        detail: format!("endpoint '{}' has no host", endpoint),
    })?;

    // Try parsing as IP directly
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            warn!(endpoint, "SSRF blocked: private IP detected");
            return Err(AnmlClientError::SsrfBlocked {
                endpoint: endpoint.to_string(),
            });
        }
        return Ok(());
    }

    // DNS resolution
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addr_str = format!("{}:{}", host, port);

    // Use tokio's DNS resolution
    let lookup_result = tokio::net::lookup_host(&addr_str).await;
    match lookup_result {
        Ok(addrs) => {
            for addr in addrs {
                if is_private_ip(&addr.ip()) {
                    warn!(endpoint, ip = %addr.ip(), "SSRF blocked: resolved to private IP");
                    return Err(AnmlClientError::SsrfBlocked {
                        endpoint: endpoint.to_string(),
                    });
                }
            }
            Ok(())
        }
        Err(_) => {
            // DNS resolution failed — we can't verify, so allow it
            // (the HTTP request will fail anyway if the host is unreachable)
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Endpoint URI resolution
// ---------------------------------------------------------------------------

/// Resolve an action endpoint URI against the document origin.
///
/// - Relative URIs resolve against the document's origin (scheme+host+port)
/// - `xml:base` on an ancestor takes precedence (passed as `xml_base`)
/// - Absolute URIs pass through but still need SSRF checks
pub fn resolve_endpoint(
    endpoint: &str,
    document_origin: &str,
    xml_base: Option<&str>,
) -> crate::Result<String> {
    // If xml:base is set, use it as the base
    let base_str = xml_base.unwrap_or(document_origin);

    // Try parsing as absolute URI first
    if let Ok(_url) = Url::parse(endpoint) {
        // Already absolute
        return Ok(endpoint.to_string());
    }

    // Relative URI — resolve against base
    let base = Url::parse(base_str).map_err(|e| AnmlClientError::MalformedDocument {
        detail: format!("invalid base URL '{}': {}", base_str, e),
    })?;

    let resolved = base.join(endpoint).map_err(|e| AnmlClientError::MalformedDocument {
        detail: format!("cannot resolve endpoint '{}' against base '{}': {}", endpoint, base_str, e),
    })?;

    Ok(resolved.to_string())
}

/// Extract the origin (scheme://host[:port]) from a URL string.
pub fn origin_from_url_str(url: &str) -> Option<Origin> {
    let parsed = Url::parse(url).ok()?;
    Some(Origin {
        scheme: parsed.scheme().to_string(),
        host: parsed.host_str().unwrap_or_default().to_string(),
        port: parsed.port(),
    })
}

// ---------------------------------------------------------------------------
// Action lookup
// ---------------------------------------------------------------------------

/// Find an `<action>` by id in the document's `<interact>` section.
pub fn find_action<'a>(doc: &'a AnmlDocument, action_id: &str) -> Option<&'a AnmlAction> {
    doc.interact
        .as_ref()
        .and_then(|interact| interact.actions.iter().find(|a| a.id == action_id))
}

// ---------------------------------------------------------------------------
// execute_action — the full pipeline
// ---------------------------------------------------------------------------

/// Result of executing an action.
#[derive(Debug)]
pub struct ActionResult {
    /// The parsed ANML response document, if the response was ANML.
    pub document: Option<AnmlDocument>,
    /// The HTTP status code.
    pub status: u16,
    /// The raw response body bytes.
    pub body: Vec<u8>,
    /// The idempotency key used, if any.
    pub idempotency_key: Option<String>,
}

/// Context for action execution, bundling all dependencies.
pub struct ActionContext<'a> {
    /// The HTTP client for making requests.
    pub http: &'a reqwest::Client,
    /// The document origin URL (scheme://host[:port]).
    pub document_origin: &'a str,
    /// Optional xml:base override.
    pub xml_base: Option<&'a str>,
    /// The action budget tracker.
    pub budget_tracker: &'a ActionBudgetTracker,
    /// Optional confirm handler.
    pub confirm_handler: Option<&'a dyn ConfirmHandler>,
    /// Optional auth provider.
    pub auth_provider: Option<&'a dyn AuthProvider>,
    /// Per-action timeout.
    pub action_timeout: std::time::Duration,
    /// Per-request timeout.
    pub request_timeout: std::time::Duration,
    /// Disclosure context for running the 7-step algorithm.
    pub disclosure_ctx: Option<&'a crate::disclosure::DisclosureContext<'a>>,
}

/// Execute an action from an ANML document.
///
/// This is the full action execution pipeline:
/// 1. Resolve the `<action>` by id
/// 2. Resolve endpoint URI
/// 3. SSRF check
/// 4. Collect and validate parameters
/// 5. Run disclosure evaluation for answer values (if disclosure context provided)
/// 6. Serialize request body per enctype
/// 7. Invoke confirm callback if `confirm="true"`
/// 8. Generate Idempotency-Key if needed
/// 9. Track action budget
/// 10. Integrate auth provider
/// 11. Perform HTTP request with per-action timeout
/// 12. Parse response as ANML document
#[instrument(skip(doc, user_params, ctx), fields(action_id))]
pub async fn execute_action(
    doc: &AnmlDocument,
    action_id: &str,
    user_params: &[(String, String)],
    ctx: &ActionContext<'_>,
) -> crate::Result<ActionResult> {
    // Step 1: Find the action
    let action = find_action(doc, action_id).ok_or_else(|| AnmlClientError::MalformedDocument {
        detail: format!("action '{}' not found in <interact>", action_id),
    })?;

    debug!(action_id, method = %action.method, "executing action");

    // Step 2: Resolve endpoint
    let endpoint = resolve_endpoint(&action.endpoint, ctx.document_origin, ctx.xml_base)?;
    debug!(action_id, %endpoint, "resolved endpoint");

    // Step 3: SSRF check
    check_ssrf(&endpoint).await?;

    // Step 4: Collect and validate parameters
    let param_defs = action.params.as_deref().unwrap_or(&[]);
    let param_values = params::collect_params(param_defs, user_params);

    for def in param_defs {
        let value = param_values.iter().find(|p| p.name == def.name).map(|p| p.value.as_str());
        validation::validate_param(def, value)?;
    }

    // Step 5: Run disclosure evaluation (if context provided and we have answer values)
    if let Some(disclosure_ctx) = ctx.disclosure_ctx {
        let rules = crate::disclosure::extract_rules(doc);
        for pv in &param_values {
            let decision = crate::disclosure::evaluate(doc, &rules, &pv.name, &pv.value, disclosure_ctx);
            match decision {
                crate::disclosure::DisclosureDecision::Allow { .. } => {
                    debug!(field = %pv.name, "disclosure allowed");
                }
                crate::disclosure::DisclosureDecision::Deny { field, reason, .. } => {
                    debug!(field = %field, %reason, "disclosure denied");
                    // Disclosure denied — we don't block the action entirely,
                    // but the caller should handle this. For now, log and continue.
                    // The RFC says to emit <refuse> instead of <answer>.
                }
            }
        }
    }

    // Step 6: Serialize request body per enctype
    let enctype = action
        .enctype
        .as_deref()
        .unwrap_or("application/x-www-form-urlencoded");

    let doc_id = doc
        .head
        .as_ref()
        .and_then(|h| h.title.as_ref())
        .map(|t| t.text.as_str())
        .unwrap_or("anml");

    let (content_type, body) = encode_body(enctype, &param_values, doc_id);

    // Step 7: Invoke confirm callback if confirm="true"
    if action.confirm == Some(true) {
        if let Some(handler) = ctx.confirm_handler {
            let decision = handler.request_confirmation(
                &action.id,
                &action.method,
                &endpoint,
                action.description.as_deref(),
            );
            match decision {
                ConfirmDecision::Cancel => {
                    debug!(action_id, "action cancelled by user");
                    return Err(AnmlClientError::ConsentDenied {
                        field: action_id.to_string(),
                        rule: "confirm=true".into(),
                        consent_scope: "action".into(),
                    });
                }
                ConfirmDecision::Confirm => {
                    debug!(action_id, "action confirmed by user");
                }
            }
        } else {
            warn!(action_id, "action requires confirmation but no confirm handler configured");
            return Err(AnmlClientError::ConsentDenied {
                field: action_id.to_string(),
                rule: "confirm=true".into(),
                consent_scope: "action".into(),
            });
        }
    }

    // Step 8: Generate Idempotency-Key if needed
    let idempotency_key = if action.idempotent != Some(true) {
        Some(idempotency::generate_key())
    } else {
        None
    };

    // Step 9: Track action budget
    let endpoint_origin = origin_from_url_str(&endpoint)
        .map(|o| o.to_string())
        .unwrap_or_default();
    ctx.budget_tracker.check_and_record(&endpoint_origin)?;

    // Step 10 + 11: Build and execute HTTP request with per-action timeout
    let result = tokio::time::timeout(
        ctx.action_timeout,
        execute_http_request(
            ctx.http,
            &action.method,
            &endpoint,
            &content_type,
            &body,
            idempotency_key.as_deref(),
            action.auth.as_ref(),
            ctx.auth_provider,
            ctx.request_timeout,
        ),
    )
    .await
    .map_err(|_| AnmlClientError::Timeout {
        operation: "per_action".into(),
        timeout_secs: ctx.action_timeout.as_secs(),
    })??;

    // Step 12: Parse response as ANML document
    let status = result.0;
    let response_body = result.1;

    let parsed_doc = if !response_body.is_empty() {
        match String::from_utf8(response_body.clone()) {
            Ok(text) => anml::parser::parse(&text).ok(),
            Err(_) => None,
        }
    } else {
        None
    };

    Ok(ActionResult {
        document: parsed_doc,
        status,
        body: response_body,
        idempotency_key,
    })
}

/// Encode the request body per the action's enctype.
fn encode_body(enctype: &str, params: &[ParamValue], doc_id: &str) -> (String, Vec<u8>) {
    match enctype {
        "multipart/form-data" => params::encode_multipart(params, doc_id),
        "application/json" => {
            let body = params::encode_json(params);
            ("application/json".to_string(), body)
        }
        _ => {
            // Default: application/x-www-form-urlencoded
            let body = params::encode_urlencoded(params);
            ("application/x-www-form-urlencoded".to_string(), body)
        }
    }
}

/// Execute the HTTP request with auth integration and retry on 401.
async fn execute_http_request(
    http: &reqwest::Client,
    method: &str,
    endpoint: &str,
    content_type: &str,
    body: &[u8],
    idempotency_key: Option<&str>,
    auth_type: Option<&AuthType>,
    auth_provider: Option<&dyn AuthProvider>,
    request_timeout: std::time::Duration,
) -> crate::Result<(u16, Vec<u8>)> {
    let needs_auth = matches!(auth_type, Some(AuthType::Required) | Some(AuthType::Optional));

    // Build the request
    let mut builder = match method.to_uppercase().as_str() {
        "GET" => http.get(endpoint),
        "POST" => http.post(endpoint),
        "PUT" => http.put(endpoint),
        "DELETE" => http.delete(endpoint),
        "PATCH" => http.patch(endpoint),
        "HEAD" => http.head(endpoint),
        _ => http.request(
            reqwest::Method::from_bytes(method.as_bytes())
                .unwrap_or(reqwest::Method::POST),
            endpoint,
        ),
    };

    builder = builder
        .header("Content-Type", content_type)
        .body(body.to_vec());

    // Add idempotency key
    if let Some(key) = idempotency_key {
        builder = builder.header(idempotency::IDEMPOTENCY_KEY_HEADER, key);
    }

    // Add auth credentials
    if needs_auth {
        if let Some(provider) = auth_provider {
            let origin = origin_from_url_str(endpoint);
            if let Some(ref origin) = origin {
                if let Some(creds) = provider.credentials(origin).await {
                    for (name, value) in &creds {
                        builder = builder.header(name.as_str(), value.as_str());
                    }
                }
            }
        } else if matches!(auth_type, Some(AuthType::Required)) {
            return Err(AnmlClientError::TrustInsufficient {
                origin: endpoint.to_string(),
                reason: "action requires authentication but no auth provider configured".into(),
            });
        }
    }

    // Execute with timeout
    let response = tokio::time::timeout(request_timeout, builder.send())
        .await
        .map_err(|_| AnmlClientError::Timeout {
            operation: "per_request".into(),
            timeout_secs: request_timeout.as_secs(),
        })??;

    let status = response.status().as_u16();

    // Handle 401 — retry with refreshed credentials
    if status == 401 && needs_auth {
        if let Some(provider) = auth_provider {
            let origin = origin_from_url_str(endpoint);
            if let Some(ref origin) = origin {
                let refresh_result = provider.on_unauthorized(origin).await;
                if refresh_result == AuthRefreshResult::Refreshed {
                    debug!("auth refreshed, retrying request");
                    // Rebuild and retry once
                    let mut retry_builder = match method.to_uppercase().as_str() {
                        "GET" => http.get(endpoint),
                        "POST" => http.post(endpoint),
                        "PUT" => http.put(endpoint),
                        "DELETE" => http.delete(endpoint),
                        "PATCH" => http.patch(endpoint),
                        "HEAD" => http.head(endpoint),
                        _ => http.request(
                            reqwest::Method::from_bytes(method.as_bytes())
                                .unwrap_or(reqwest::Method::POST),
                            endpoint,
                        ),
                    };

                    retry_builder = retry_builder
                        .header("Content-Type", content_type)
                        .body(body.to_vec());

                    if let Some(key) = idempotency_key {
                        retry_builder = retry_builder.header(idempotency::IDEMPOTENCY_KEY_HEADER, key);
                    }

                    if let Some(creds) = provider.credentials(origin).await {
                        for (name, value) in &creds {
                            retry_builder = retry_builder.header(name.as_str(), value.as_str());
                        }
                    }

                    let retry_response = tokio::time::timeout(request_timeout, retry_builder.send())
                        .await
                        .map_err(|_| AnmlClientError::Timeout {
                            operation: "per_request".into(),
                            timeout_secs: request_timeout.as_secs(),
                        })??;

                    let retry_status = retry_response.status().as_u16();
                    let retry_body = retry_response.bytes().await?.to_vec();
                    return Ok((retry_status, retry_body));
                }
            }
        }
    }

    let response_body = response.bytes().await?.to_vec();
    Ok((status, response_body))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SSRF protection --

    #[test]
    fn private_ipv4_loopback() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"127.0.0.2".parse().unwrap()));
    }

    #[test]
    fn private_ipv4_10() {
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"10.255.255.255".parse().unwrap()));
    }

    #[test]
    fn private_ipv4_172_16() {
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.31.255.255".parse().unwrap()));
        assert!(!is_private_ip(&"172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn private_ipv4_192_168() {
        assert!(is_private_ip(&"192.168.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.255.255".parse().unwrap()));
    }

    #[test]
    fn public_ipv4() {
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn private_ipv6_loopback() {
        assert!(is_private_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn private_ipv6_ula() {
        assert!(is_private_ip(&"fc00::1".parse().unwrap()));
        assert!(is_private_ip(&"fd00::1".parse().unwrap()));
    }

    #[test]
    fn public_ipv6() {
        assert!(!is_private_ip(&"2001:db8::1".parse().unwrap()));
    }

    // -- Endpoint resolution --

    #[test]
    fn resolve_relative_endpoint() {
        let result = resolve_endpoint("/airline", "https://api.example.com", None).unwrap();
        assert_eq!(result, "https://api.example.com/airline");
    }

    #[test]
    fn resolve_relative_with_path() {
        let result = resolve_endpoint("/api/v1/submit", "https://api.example.com", None).unwrap();
        assert_eq!(result, "https://api.example.com/api/v1/submit");
    }

    #[test]
    fn resolve_absolute_endpoint() {
        let result = resolve_endpoint(
            "https://other.example.com/submit",
            "https://api.example.com",
            None,
        )
        .unwrap();
        assert_eq!(result, "https://other.example.com/submit");
    }

    #[test]
    fn resolve_with_xml_base() {
        let result = resolve_endpoint(
            "/submit",
            "https://api.example.com",
            Some("https://base.example.com"),
        )
        .unwrap();
        assert_eq!(result, "https://base.example.com/submit");
    }

    // -- Action lookup --

    #[test]
    fn find_action_found() {
        let doc = AnmlDocument {
            interact: Some(anml::types::elements::AnmlInteract {
                actions: vec![AnmlAction {
                    id: "submit-airline".into(),
                    method: "POST".into(),
                    endpoint: "/airline".into(),
                    enctype: None,
                    auth: None,
                    idempotent: None,
                    confirm: None,
                    description: None,
                    params: None,
                    response: None,
                }],
            }),
            ..Default::default()
        };
        assert!(find_action(&doc, "submit-airline").is_some());
    }

    #[test]
    fn find_action_not_found() {
        let doc = AnmlDocument::default();
        assert!(find_action(&doc, "nonexistent").is_none());
    }

    // -- Budget tracker --

    #[test]
    fn budget_tracker_allows_within_limits() {
        let tracker = ActionBudgetTracker::new(ActionBudget::default());
        assert!(tracker.check_and_record("https://example.com").is_ok());
    }

    #[test]
    fn budget_tracker_rejects_excess_requests() {
        let budget = ActionBudget {
            max_requests: 2,
            ..ActionBudget::default()
        };
        let tracker = ActionBudgetTracker::new(budget);
        assert!(tracker.check_and_record("https://example.com").is_ok());
        assert!(tracker.check_and_record("https://example.com").is_ok());
        assert!(tracker.check_and_record("https://example.com").is_err());
    }

    #[test]
    fn budget_tracker_rejects_excess_origins() {
        let budget = ActionBudget {
            max_distinct_origins: 2,
            ..ActionBudget::default()
        };
        let tracker = ActionBudgetTracker::new(budget);
        assert!(tracker.check_and_record("https://a.com").is_ok());
        assert!(tracker.check_and_record("https://b.com").is_ok());
        assert!(tracker.check_and_record("https://c.com").is_err());
    }

    #[test]
    fn budget_tracker_same_origin_doesnt_count_twice() {
        let budget = ActionBudget {
            max_distinct_origins: 2,
            ..ActionBudget::default()
        };
        let tracker = ActionBudgetTracker::new(budget);
        assert!(tracker.check_and_record("https://a.com").is_ok());
        assert!(tracker.check_and_record("https://a.com").is_ok());
        assert!(tracker.check_and_record("https://b.com").is_ok());
    }

    #[test]
    fn budget_tracker_reset() {
        let budget = ActionBudget {
            max_requests: 1,
            ..ActionBudget::default()
        };
        let tracker = ActionBudgetTracker::new(budget);
        assert!(tracker.check_and_record("https://a.com").is_ok());
        assert!(tracker.check_and_record("https://a.com").is_err());
        tracker.reset();
        assert!(tracker.check_and_record("https://a.com").is_ok());
    }

    // -- Origin extraction --

    #[test]
    fn origin_from_url_str_basic() {
        let origin = origin_from_url_str("https://api.example.com/path").unwrap();
        assert_eq!(origin.scheme, "https");
        assert_eq!(origin.host, "api.example.com");
        assert_eq!(origin.port, None);
    }

    #[test]
    fn origin_from_url_str_with_port() {
        let origin = origin_from_url_str("https://api.example.com:8443/path").unwrap();
        assert_eq!(origin.port, Some(8443));
    }
}
