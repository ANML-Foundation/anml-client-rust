//! Client configuration, traits, and timeout settings.
//!
//! This module defines the configuration structures, callback traits, and
//! policy interfaces used by [`AnmlClient`](crate::client::AnmlClient).
//! All config structs and enums are `#[non_exhaustive]` per semver policy.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use anml::types::document::AnmlDocument;
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Origin (lightweight wrapper for scheme + host + port)
// ---------------------------------------------------------------------------

/// An HTTP origin (scheme + host + optional port) as defined in RFC 6454.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Origin {
    /// The scheme (e.g. `"https"`).
    pub scheme: String,
    /// The host (e.g. `"api.example.com"`).
    pub host: String,
    /// The port, if explicitly specified.
    pub port: Option<u16>,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(p) => write!(f, "{}://{}:{}", self.scheme, self.host, p),
            None => write!(f, "{}://{}", self.scheme, self.host),
        }
    }
}

// ---------------------------------------------------------------------------
// TimeoutConfig
// ---------------------------------------------------------------------------

/// Timeout settings for different operation scopes.
///
/// Each timeout is enforced independently via `tokio::time::timeout`.
/// The per-flow timeout wraps the entire flow execution — exceeding it
/// cancels all in-progress steps.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TimeoutConfig {
    /// Timeout for a single HTTP request (default: 30s).
    pub per_request: Duration,
    /// Timeout for a complete action including retries (default: 60s).
    pub per_action: Duration,
    /// Timeout for an entire multi-step flow (default: 5min).
    pub per_flow: Duration,
    /// Timeout for a single media resource fetch (default: 15s).
    pub per_media_fetch: Duration,
    /// Timeout for XML parsing (default: 5s).
    pub parse: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            per_request: Duration::from_secs(30),
            per_action: Duration::from_secs(60),
            per_flow: Duration::from_secs(300),
            per_media_fetch: Duration::from_secs(15),
            parse: Duration::from_secs(5),
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceLimits
// ---------------------------------------------------------------------------

/// RFC-mandated resource limits for ANML document parsing and processing.
///
/// These limits are configurable but enforce minimum floor values — they
/// cannot be disabled entirely. Attempting to set a value below the floor
/// clamps it to the floor.
///
/// See RFC Section 11.6 (Security Considerations — Resource Limits).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum document size in bytes after decompression (default: 1 MiB, floor: 1 KiB).
    pub max_document_size: usize,
    /// Maximum element nesting depth (default: 64, floor: 4).
    pub max_depth: usize,
    /// Maximum total element count (default: 10_000, floor: 10).
    pub max_elements: usize,
    /// Maximum attribute value length in bytes (default: 64 KiB, floor: 256).
    pub max_attribute_value_length: usize,
    /// Maximum attributes per element (default: 256, floor: 4).
    pub max_attributes_per_element: usize,
    /// Maximum text content length per element in bytes (default: 256 KiB, floor: 256).
    pub max_text_per_element: usize,
    /// Maximum entity expansion ratio (default: 64, floor: 1).
    pub max_entity_expansion_ratio: usize,
    /// Maximum parse wall-clock time (default: 5s, floor: 1s).
    pub max_parse_time: Duration,
    /// Maximum `<disclosure>` rules per document (default: 1_000, floor: 1).
    pub max_disclosure_rules: usize,
    /// Maximum knowledge primitives (`<ask>` + `<answer>` + `<refuse>`) per document
    /// (default: 1_000, floor: 1).
    pub max_knowledge_primitives: usize,
    /// Maximum `<step>` elements per `<flow>` (default: 256, floor: 1).
    pub max_steps_per_flow: usize,
}

impl ResourceLimits {
    // Floor values — these cannot be set lower.
    const FLOOR_DOCUMENT_SIZE: usize = 1024;
    const FLOOR_DEPTH: usize = 4;
    const FLOOR_ELEMENTS: usize = 10;
    const FLOOR_ATTR_VALUE_LEN: usize = 256;
    const FLOOR_ATTRS_PER_ELEMENT: usize = 4;
    const FLOOR_TEXT_PER_ELEMENT: usize = 256;
    const FLOOR_ENTITY_EXPANSION: usize = 1;
    const FLOOR_PARSE_TIME: Duration = Duration::from_secs(1);
    const FLOOR_DISCLOSURE_RULES: usize = 1;
    const FLOOR_KNOWLEDGE_PRIMITIVES: usize = 1;
    const FLOOR_STEPS_PER_FLOW: usize = 1;

    /// Clamp all values to their minimum floors.
    pub fn clamp(&mut self) {
        self.max_document_size = self.max_document_size.max(Self::FLOOR_DOCUMENT_SIZE);
        self.max_depth = self.max_depth.max(Self::FLOOR_DEPTH);
        self.max_elements = self.max_elements.max(Self::FLOOR_ELEMENTS);
        self.max_attribute_value_length = self
            .max_attribute_value_length
            .max(Self::FLOOR_ATTR_VALUE_LEN);
        self.max_attributes_per_element = self
            .max_attributes_per_element
            .max(Self::FLOOR_ATTRS_PER_ELEMENT);
        self.max_text_per_element = self.max_text_per_element.max(Self::FLOOR_TEXT_PER_ELEMENT);
        self.max_entity_expansion_ratio = self
            .max_entity_expansion_ratio
            .max(Self::FLOOR_ENTITY_EXPANSION);
        self.max_parse_time = self.max_parse_time.max(Self::FLOOR_PARSE_TIME);
        self.max_disclosure_rules = self.max_disclosure_rules.max(Self::FLOOR_DISCLOSURE_RULES);
        self.max_knowledge_primitives = self
            .max_knowledge_primitives
            .max(Self::FLOOR_KNOWLEDGE_PRIMITIVES);
        self.max_steps_per_flow = self.max_steps_per_flow.max(Self::FLOOR_STEPS_PER_FLOW);
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_document_size: 1_048_576,       // 1 MiB
            max_depth: 64,
            max_elements: 10_000,
            max_attribute_value_length: 65_536,  // 64 KiB
            max_attributes_per_element: 256,
            max_text_per_element: 262_144,       // 256 KiB
            max_entity_expansion_ratio: 64,
            max_parse_time: Duration::from_secs(5),
            max_disclosure_rules: 1_000,
            max_knowledge_primitives: 1_000,
            max_steps_per_flow: 256,
        }
    }
}

// ---------------------------------------------------------------------------
// ActionBudget
// ---------------------------------------------------------------------------

/// Per-document budget for action execution and media fetching.
///
/// Prevents a single document from triggering excessive outbound traffic.
/// See RFC Section 11.8 (Malicious ANML Documents).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ActionBudget {
    /// Maximum distinct origins an agent may contact per document (default: 5).
    pub max_distinct_origins: u32,
    /// Maximum total HTTP requests per document (default: 50).
    pub max_requests: u32,
    /// Maximum media resource fetches per document (default: 20).
    pub max_media_fetches: u32,
    /// Maximum total media bandwidth in bytes per document (default: 50 MiB).
    pub max_media_bandwidth: u64,
}

impl Default for ActionBudget {
    fn default() -> Self {
        Self {
            max_distinct_origins: 5,
            max_requests: 50,
            max_media_fetches: 20,
            max_media_bandwidth: 50 * 1_048_576, // 50 MiB
        }
    }
}


// ---------------------------------------------------------------------------
// TrustDecision + TrustPolicy trait
// ---------------------------------------------------------------------------

/// The result of a trust policy evaluation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum TrustDecision {
    /// The origin is trusted; proceed with the operation.
    Allow,
    /// The origin is not trusted; the operation should be refused.
    Deny {
        /// Human-readable reason for the denial.
        reason: String,
    },
}

/// Policy consulted before acting on any document or executing any action.
///
/// The disclosure algorithm (RFC §8.5, step 4) invokes this trait. A denial
/// causes the client to emit `<refuse reason="trust-insufficient">`.
///
/// The default implementation ([`DenyAllTrustPolicy`]) denies all origins.
/// Use [`AllowListTrustPolicy`] or provide your own implementation.
pub trait TrustPolicy: Send + Sync {
    /// Evaluate whether the given origin should be trusted for the given document.
    fn evaluate(&self, origin: &Origin, doc: &AnmlDocument) -> TrustDecision;
}

/// Denies all origins unconditionally. This is the default trust policy.
///
/// Users must configure an explicit allow-list or custom policy to interact
/// with any ANML service.
#[derive(Clone, Debug, Default)]
pub struct DenyAllTrustPolicy;

impl TrustPolicy for DenyAllTrustPolicy {
    fn evaluate(&self, origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
        TrustDecision::Deny {
            reason: format!("DenyAllTrustPolicy: origin '{}' is not trusted", origin),
        }
    }
}

/// Allows a configured set of origins; denies everything else.
///
/// Origins are matched by (scheme, host, port). If the port is `None` in the
/// allow-list entry, it matches any port for that scheme+host.
#[derive(Clone, Debug, Default)]
pub struct AllowListTrustPolicy {
    allowed: HashSet<Origin>,
}

impl AllowListTrustPolicy {
    /// Create a new empty allow-list policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an origin to the allow list.
    pub fn allow(mut self, origin: Origin) -> Self {
        self.allowed.insert(origin);
        self
    }

    /// Add an origin from a URL string. Returns `Self` unchanged if the URL
    /// cannot be parsed.
    pub fn allow_url(mut self, url: &str) -> Self {
        if let Ok(parsed) = url::Url::parse(url) {
            let origin = Origin {
                scheme: parsed.scheme().to_string(),
                host: parsed.host_str().unwrap_or_default().to_string(),
                port: parsed.port(),
            };
            self.allowed.insert(origin);
        }
        self
    }
}

impl TrustPolicy for AllowListTrustPolicy {
    fn evaluate(&self, origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
        // Check exact match first.
        if self.allowed.contains(origin) {
            return TrustDecision::Allow;
        }
        // Check with port=None (wildcard port match).
        let wildcard = Origin {
            scheme: origin.scheme.clone(),
            host: origin.host.clone(),
            port: None,
        };
        if self.allowed.contains(&wildcard) {
            return TrustDecision::Allow;
        }
        TrustDecision::Deny {
            reason: format!("origin '{}' is not in the allow list", origin),
        }
    }
}

// ---------------------------------------------------------------------------
// ConsentDecision + ConsentHandler trait
// ---------------------------------------------------------------------------

/// The principal's decision on a consent prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsentDecision {
    /// The principal grants consent for this disclosure.
    Grant,
    /// The principal denies consent.
    Deny,
}

/// Callback trait for obtaining explicit consent from the principal.
///
/// This is a **synchronous** callback — it blocks the current async task
/// until the principal responds. Implementations should prompt the user
/// and wait for their answer.
///
/// The disclosure algorithm (RFC §8.5, step 3) invokes this when
/// `requires="explicit"`.
pub trait ConsentHandler: Send + Sync {
    /// Prompt the principal for consent to disclose `field` to `origin`
    /// under the given `purpose` (from the `<ask>` or `<disclosure>`).
    ///
    /// Returns [`ConsentDecision::Grant`] or [`ConsentDecision::Deny`].
    fn request_consent(
        &self,
        field: &str,
        origin: &Origin,
        purpose: Option<&str>,
    ) -> ConsentDecision;
}

// ---------------------------------------------------------------------------
// ConfirmDecision + ConfirmHandler trait
// ---------------------------------------------------------------------------

/// The principal's decision on an action confirmation prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfirmDecision {
    /// The principal confirms the action should proceed.
    Confirm,
    /// The principal cancels the action.
    Cancel,
}

/// Callback trait for confirming action execution with the principal.
///
/// Invoked when an `<action>` has `confirm="true"` or when the client
/// determines confirmation is needed (e.g., state-modifying action at
/// an unfamiliar origin).
pub trait ConfirmHandler: Send + Sync {
    /// Prompt the principal to confirm execution of the given action.
    ///
    /// `action_id` is the `<action>` `id` attribute, `method` is the HTTP
    /// method, and `endpoint` is the resolved endpoint URI.
    fn request_confirmation(
        &self,
        action_id: &str,
        method: &str,
        endpoint: &str,
        description: Option<&str>,
    ) -> ConfirmDecision;
}

// ---------------------------------------------------------------------------
// ConditionEvaluator trait
// ---------------------------------------------------------------------------

/// Callback trait for evaluating `<step condition="...">` expressions.
///
/// The RFC defines a `condition` attribute on `<step>` but does not specify
/// an expression language. This trait lets the consuming application provide
/// its own evaluator.
///
/// If no evaluator is configured and a step has a condition, the client
/// treats the step as available (condition = true) and logs a warning.
pub trait ConditionEvaluator: Send + Sync {
    /// Evaluate the condition expression for a flow step.
    ///
    /// Returns `true` if the step should be available, `false` to skip it.
    fn evaluate(&self, condition: &str, step_id: &str) -> bool;
}

// ---------------------------------------------------------------------------
// AuthProvider trait
// ---------------------------------------------------------------------------

/// The result of an authentication refresh attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthRefreshResult {
    /// Credentials were refreshed; the request should be retried.
    Refreshed,
    /// Refresh failed; propagate the 401 as an error.
    Failed,
}

/// Async trait for providing authentication credentials.
///
/// Called before each request to an origin that requires auth
/// (`<action auth="required|optional">`). On 401, `on_unauthorized()`
/// is called and the request is retried once if `Refreshed`.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Return headers to attach to requests for the given origin.
    ///
    /// Returns `None` if no credentials are available for this origin.
    /// Each tuple is `(header_name, header_value)`.
    async fn credentials(&self, origin: &Origin) -> Option<Vec<(String, String)>>;

    /// Called when a 401 Unauthorized response is received.
    ///
    /// Implementations can refresh tokens, re-authenticate, etc.
    async fn on_unauthorized(&self, origin: &Origin) -> AuthRefreshResult;
}

// ---------------------------------------------------------------------------
// HttpMiddleware trait
// ---------------------------------------------------------------------------

/// Async trait for composable HTTP request/response interceptors.
///
/// Middleware is applied in registration order for requests (first → last)
/// and reverse order for responses (last → first).
#[async_trait]
pub trait HttpMiddleware: Send + Sync {
    /// Intercept and optionally modify an outgoing request.
    async fn on_request(
        &self,
        req: reqwest::Request,
    ) -> crate::Result<reqwest::Request>;

    /// Intercept and optionally modify an incoming response.
    async fn on_response(
        &self,
        resp: reqwest::Response,
    ) -> crate::Result<reqwest::Response>;
}

// ---------------------------------------------------------------------------
// ClientConfig
// ---------------------------------------------------------------------------

/// Configuration for [`AnmlClient`](crate::client::AnmlClient).
///
/// Use [`ClientConfigBuilder`] (via `ClientConfig::builder()`) to construct.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ClientConfig {
    /// Base URL for the ANML service (e.g. `"https://api.example.com"`).
    pub base_url: Option<String>,
    /// Timeout settings for different operation scopes.
    pub timeouts: TimeoutConfig,
    /// Whether to allow plaintext HTTP (default: `false`).
    ///
    /// When `false`, the client refuses to fetch documents over HTTP and
    /// rejects documents containing `<constraints>`, `<interact>`, or
    /// `<ask requires="explicit">` when fetched over unencrypted transport.
    pub allow_plaintext_http: bool,
    /// Default headers to include on every request.
    pub default_headers: Vec<(String, String)>,
    /// RFC resource limits for document parsing.
    pub resource_limits: ResourceLimits,
    /// Per-document action execution budget.
    pub action_budget: ActionBudget,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            timeouts: TimeoutConfig::default(),
            allow_plaintext_http: false,
            default_headers: Vec::new(),
            resource_limits: ResourceLimits::default(),
            action_budget: ActionBudget::default(),
        }
    }
}

impl ClientConfig {
    /// Create a new [`ClientConfigBuilder`].
    pub fn builder() -> ClientConfigBuilder {
        ClientConfigBuilder::default()
    }
}


// ---------------------------------------------------------------------------
// ClientConfigBuilder
// ---------------------------------------------------------------------------

/// Builder for [`ClientConfig`].
///
/// ```rust
/// use anml_client::config::{ClientConfig, TimeoutConfig, AllowListTrustPolicy};
/// use std::time::Duration;
///
/// let config = ClientConfig::builder()
///     .base_url("https://api.example.com")
///     .allow_plaintext_http(false)
///     .build();
/// ```
#[derive(Default)]
pub struct ClientConfigBuilder {
    base_url: Option<String>,
    timeouts: Option<TimeoutConfig>,
    allow_plaintext_http: bool,
    default_headers: Vec<(String, String)>,
    resource_limits: Option<ResourceLimits>,
    action_budget: Option<ActionBudget>,
}

impl ClientConfigBuilder {
    /// Set the base URL for the ANML service.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set the timeout configuration.
    pub fn timeouts(mut self, timeouts: TimeoutConfig) -> Self {
        self.timeouts = Some(timeouts);
        self
    }

    /// Set the per-request timeout (convenience shorthand).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        let mut t = self.timeouts.take().unwrap_or_default();
        t.per_request = timeout;
        self.timeouts = Some(t);
        self
    }

    /// Allow or disallow plaintext HTTP (default: `false`).
    pub fn allow_plaintext_http(mut self, allow: bool) -> Self {
        self.allow_plaintext_http = allow;
        self
    }

    /// Add a default header to include on every request.
    pub fn default_header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.default_headers.push((name.into(), value.into()));
        self
    }

    /// Set the resource limits for document parsing.
    pub fn resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Set the per-document action budget.
    pub fn action_budget(mut self, budget: ActionBudget) -> Self {
        self.action_budget = Some(budget);
        self
    }

    /// Build the [`ClientConfig`].
    ///
    /// Resource limits are clamped to their minimum floor values.
    pub fn build(self) -> ClientConfig {
        let mut resource_limits = self.resource_limits.unwrap_or_default();
        resource_limits.clamp();

        ClientConfig {
            base_url: self.base_url,
            timeouts: self.timeouts.unwrap_or_default(),
            allow_plaintext_http: self.allow_plaintext_http,
            default_headers: self.default_headers,
            resource_limits,
            action_budget: self.action_budget.unwrap_or_default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TimeoutConfig --

    #[test]
    fn timeout_config_defaults() {
        let tc = TimeoutConfig::default();
        assert_eq!(tc.per_request, Duration::from_secs(30));
        assert_eq!(tc.per_action, Duration::from_secs(60));
        assert_eq!(tc.per_flow, Duration::from_secs(300));
        assert_eq!(tc.per_media_fetch, Duration::from_secs(15));
        assert_eq!(tc.parse, Duration::from_secs(5));
    }

    // -- ResourceLimits --

    #[test]
    fn resource_limits_defaults() {
        let rl = ResourceLimits::default();
        assert_eq!(rl.max_document_size, 1_048_576);
        assert_eq!(rl.max_depth, 64);
        assert_eq!(rl.max_elements, 10_000);
        assert_eq!(rl.max_attribute_value_length, 65_536);
        assert_eq!(rl.max_attributes_per_element, 256);
        assert_eq!(rl.max_text_per_element, 262_144);
        assert_eq!(rl.max_entity_expansion_ratio, 64);
        assert_eq!(rl.max_parse_time, Duration::from_secs(5));
        assert_eq!(rl.max_disclosure_rules, 1_000);
        assert_eq!(rl.max_knowledge_primitives, 1_000);
        assert_eq!(rl.max_steps_per_flow, 256);
    }

    #[test]
    fn resource_limits_clamp_enforces_floors() {
        let mut rl = ResourceLimits {
            max_document_size: 0,
            max_depth: 0,
            max_elements: 0,
            max_attribute_value_length: 0,
            max_attributes_per_element: 0,
            max_text_per_element: 0,
            max_entity_expansion_ratio: 0,
            max_parse_time: Duration::from_millis(1),
            max_disclosure_rules: 0,
            max_knowledge_primitives: 0,
            max_steps_per_flow: 0,
        };
        rl.clamp();
        assert_eq!(rl.max_document_size, ResourceLimits::FLOOR_DOCUMENT_SIZE);
        assert_eq!(rl.max_depth, ResourceLimits::FLOOR_DEPTH);
        assert_eq!(rl.max_elements, ResourceLimits::FLOOR_ELEMENTS);
        assert_eq!(rl.max_attribute_value_length, ResourceLimits::FLOOR_ATTR_VALUE_LEN);
        assert_eq!(rl.max_attributes_per_element, ResourceLimits::FLOOR_ATTRS_PER_ELEMENT);
        assert_eq!(rl.max_text_per_element, ResourceLimits::FLOOR_TEXT_PER_ELEMENT);
        assert_eq!(rl.max_entity_expansion_ratio, ResourceLimits::FLOOR_ENTITY_EXPANSION);
        assert_eq!(rl.max_parse_time, ResourceLimits::FLOOR_PARSE_TIME);
        assert_eq!(rl.max_disclosure_rules, ResourceLimits::FLOOR_DISCLOSURE_RULES);
        assert_eq!(rl.max_knowledge_primitives, ResourceLimits::FLOOR_KNOWLEDGE_PRIMITIVES);
        assert_eq!(rl.max_steps_per_flow, ResourceLimits::FLOOR_STEPS_PER_FLOW);
    }

    #[test]
    fn resource_limits_clamp_preserves_above_floor() {
        let mut rl = ResourceLimits::default();
        let original = rl.clone();
        rl.clamp();
        assert_eq!(rl.max_document_size, original.max_document_size);
        assert_eq!(rl.max_depth, original.max_depth);
    }

    // -- ActionBudget --

    #[test]
    fn action_budget_defaults() {
        let ab = ActionBudget::default();
        assert_eq!(ab.max_distinct_origins, 5);
        assert_eq!(ab.max_requests, 50);
        assert_eq!(ab.max_media_fetches, 20);
        assert_eq!(ab.max_media_bandwidth, 50 * 1_048_576);
    }

    // -- TrustPolicy --

    #[test]
    fn deny_all_trust_policy_denies() {
        let policy = DenyAllTrustPolicy;
        let origin = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        let doc = AnmlDocument::default();
        let decision = policy.evaluate(&origin, &doc);
        assert!(matches!(decision, TrustDecision::Deny { .. }));
    }

    #[test]
    fn allow_list_trust_policy_allows_exact() {
        let origin = Origin {
            scheme: "https".into(),
            host: "api.example.com".into(),
            port: Some(443),
        };
        let policy = AllowListTrustPolicy::new().allow(origin.clone());
        let doc = AnmlDocument::default();
        assert!(matches!(policy.evaluate(&origin, &doc), TrustDecision::Allow));
    }

    #[test]
    fn allow_list_trust_policy_allows_wildcard_port() {
        let wildcard = Origin {
            scheme: "https".into(),
            host: "api.example.com".into(),
            port: None,
        };
        let policy = AllowListTrustPolicy::new().allow(wildcard);
        let specific = Origin {
            scheme: "https".into(),
            host: "api.example.com".into(),
            port: Some(8443),
        };
        let doc = AnmlDocument::default();
        assert!(matches!(policy.evaluate(&specific, &doc), TrustDecision::Allow));
    }

    #[test]
    fn allow_list_trust_policy_denies_unknown() {
        let policy = AllowListTrustPolicy::new().allow_url("https://good.example.com");
        let evil = Origin {
            scheme: "https".into(),
            host: "evil.example.com".into(),
            port: None,
        };
        let doc = AnmlDocument::default();
        assert!(matches!(policy.evaluate(&evil, &doc), TrustDecision::Deny { .. }));
    }

    #[test]
    fn allow_list_from_url() {
        let policy = AllowListTrustPolicy::new()
            .allow_url("https://api.example.com:8443/path");
        let origin = Origin {
            scheme: "https".into(),
            host: "api.example.com".into(),
            port: Some(8443),
        };
        let doc = AnmlDocument::default();
        assert!(matches!(policy.evaluate(&origin, &doc), TrustDecision::Allow));
    }

    #[test]
    fn allow_list_bad_url_is_noop() {
        let policy = AllowListTrustPolicy::new().allow_url("not a url");
        let origin = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        let doc = AnmlDocument::default();
        assert!(matches!(policy.evaluate(&origin, &doc), TrustDecision::Deny { .. }));
    }

    // -- ConsentDecision --

    #[test]
    fn consent_decision_variants() {
        let grant = ConsentDecision::Grant;
        let deny = ConsentDecision::Deny;
        assert_eq!(grant, ConsentDecision::Grant);
        assert_eq!(deny, ConsentDecision::Deny);
        assert_ne!(grant, deny);
    }

    // -- ConfirmDecision --

    #[test]
    fn confirm_decision_variants() {
        let confirm = ConfirmDecision::Confirm;
        let cancel = ConfirmDecision::Cancel;
        assert_eq!(confirm, ConfirmDecision::Confirm);
        assert_eq!(cancel, ConfirmDecision::Cancel);
        assert_ne!(confirm, cancel);
    }

    // -- AuthRefreshResult --

    #[test]
    fn auth_refresh_result_variants() {
        let refreshed = AuthRefreshResult::Refreshed;
        let failed = AuthRefreshResult::Failed;
        assert_eq!(refreshed, AuthRefreshResult::Refreshed);
        assert_eq!(failed, AuthRefreshResult::Failed);
        assert_ne!(refreshed, failed);
    }

    // -- Origin --

    #[test]
    fn origin_display_with_port() {
        let o = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: Some(8443),
        };
        assert_eq!(o.to_string(), "https://example.com:8443");
    }

    #[test]
    fn origin_display_without_port() {
        let o = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        assert_eq!(o.to_string(), "https://example.com");
    }

    // -- ClientConfig + Builder --

    #[test]
    fn client_config_defaults() {
        let config = ClientConfig::default();
        assert!(config.base_url.is_none());
        assert!(!config.allow_plaintext_http);
        assert!(config.default_headers.is_empty());
        assert_eq!(config.timeouts.per_request, Duration::from_secs(30));
        assert_eq!(config.resource_limits.max_document_size, 1_048_576);
        assert_eq!(config.action_budget.max_requests, 50);
    }

    #[test]
    fn builder_sets_base_url() {
        let config = ClientConfig::builder()
            .base_url("https://api.example.com")
            .build();
        assert_eq!(config.base_url.as_deref(), Some("https://api.example.com"));
    }

    #[test]
    fn builder_sets_timeout() {
        let config = ClientConfig::builder()
            .timeout(Duration::from_secs(10))
            .build();
        assert_eq!(config.timeouts.per_request, Duration::from_secs(10));
    }

    #[test]
    fn builder_sets_timeouts() {
        let tc = TimeoutConfig {
            per_request: Duration::from_secs(5),
            per_action: Duration::from_secs(10),
            per_flow: Duration::from_secs(60),
            per_media_fetch: Duration::from_secs(3),
            parse: Duration::from_secs(2),
        };
        let config = ClientConfig::builder().timeouts(tc).build();
        assert_eq!(config.timeouts.per_request, Duration::from_secs(5));
        assert_eq!(config.timeouts.per_action, Duration::from_secs(10));
    }

    #[test]
    fn builder_sets_allow_plaintext() {
        let config = ClientConfig::builder()
            .allow_plaintext_http(true)
            .build();
        assert!(config.allow_plaintext_http);
    }

    #[test]
    fn builder_adds_default_headers() {
        let config = ClientConfig::builder()
            .default_header("X-Custom", "value1")
            .default_header("X-Other", "value2")
            .build();
        assert_eq!(config.default_headers.len(), 2);
        assert_eq!(config.default_headers[0], ("X-Custom".into(), "value1".into()));
    }

    #[test]
    fn builder_sets_resource_limits() {
        let mut limits = ResourceLimits::default();
        limits.max_document_size = 2_097_152; // 2 MiB
        let config = ClientConfig::builder()
            .resource_limits(limits)
            .build();
        assert_eq!(config.resource_limits.max_document_size, 2_097_152);
    }

    #[test]
    fn builder_clamps_resource_limits() {
        let limits = ResourceLimits {
            max_document_size: 0,
            max_depth: 0,
            max_elements: 0,
            max_attribute_value_length: 0,
            max_attributes_per_element: 0,
            max_text_per_element: 0,
            max_entity_expansion_ratio: 0,
            max_parse_time: Duration::from_millis(1),
            max_disclosure_rules: 0,
            max_knowledge_primitives: 0,
            max_steps_per_flow: 0,
        };
        let config = ClientConfig::builder()
            .resource_limits(limits)
            .build();
        // Should be clamped to floors
        assert!(config.resource_limits.max_document_size >= ResourceLimits::FLOOR_DOCUMENT_SIZE);
        assert!(config.resource_limits.max_depth >= ResourceLimits::FLOOR_DEPTH);
    }

    #[test]
    fn builder_sets_action_budget() {
        let budget = ActionBudget {
            max_distinct_origins: 10,
            max_requests: 100,
            max_media_fetches: 40,
            max_media_bandwidth: 100 * 1_048_576,
        };
        let config = ClientConfig::builder()
            .action_budget(budget)
            .build();
        assert_eq!(config.action_budget.max_distinct_origins, 10);
        assert_eq!(config.action_budget.max_requests, 100);
    }

    // -- Send + Sync assertions --

    #[test]
    fn config_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ClientConfig>();
        assert_send_sync::<TimeoutConfig>();
        assert_send_sync::<ResourceLimits>();
        assert_send_sync::<ActionBudget>();
        assert_send_sync::<Origin>();
        assert_send_sync::<TrustDecision>();
        assert_send_sync::<ConsentDecision>();
        assert_send_sync::<ConfirmDecision>();
        assert_send_sync::<AuthRefreshResult>();
        assert_send_sync::<DenyAllTrustPolicy>();
        assert_send_sync::<AllowListTrustPolicy>();
    }

    #[test]
    fn trait_objects_are_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn TrustPolicy>();
        assert_send_sync::<dyn ConsentHandler>();
        assert_send_sync::<dyn ConfirmHandler>();
        assert_send_sync::<dyn ConditionEvaluator>();
        assert_send_sync::<dyn AuthProvider>();
        assert_send_sync::<dyn HttpMiddleware>();
    }
}
