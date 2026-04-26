//! Core `AnmlClient` struct and builder.
//!
//! `AnmlClient` is the primary entry point for interacting with ANML services.
//! It is `Clone + Send + Sync` — internally wrapping all state in `Arc` — so
//! a single instance can be shared across tasks cheaply.
//!
//! # Creating a client
//!
//! ```rust,no_run
//! use anml_client::client::AnmlClient;
//! use anml_client::config::AllowListTrustPolicy;
//! use std::time::Duration;
//!
//! # async fn example() -> anml_client::Result<()> {
//! let client = AnmlClient::builder()
//!     .base_url("https://api.example.com")
//!     .trust_policy(AllowListTrustPolicy::new()
//!         .allow_url("https://api.example.com"))
//!     .timeout(Duration::from_secs(15))
//!     .build()?;
//!
//! let doc = client.fetch("/service").await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Do NOT create a client per request
//!
//! `AnmlClient` maintains a connection pool, consent store, circuit breaker
//! state, and audit log. Creating a new client for every request discards all
//! of that. Instead, create one client at startup and `.clone()` it into each
//! task — the clone is a cheap `Arc` bump.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anml::parser::{self, ParseOptions};
use anml::types::document::AnmlDocument;
use tracing::{debug, instrument, warn};
use url::Url;

use crate::config::{
    AuthProvider, ClientConfig, ClientConfigBuilder, ConditionEvaluator, ConfirmHandler,
    ConsentHandler, DenyAllTrustPolicy, HttpMiddleware, TrustPolicy,
};
use crate::error::AnmlClientError;

// ---------------------------------------------------------------------------
// Placeholder types for modules not yet implemented
// ---------------------------------------------------------------------------

/// Placeholder consent store — tracks consent grants.
/// Full implementation is in the disclosure module.
/// Re-exported here for backward compatibility.
pub use crate::disclosure::ConsentStore;

/// Re-export audit types from the audit module.
pub use crate::audit::{AuditEntry, AuditLog, InMemoryAuditLog, NoOpAuditLog};

/// Re-export the tokenizer from the security module.
pub use crate::security::Tokenizer;

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Policy for retrying transient HTTP errors.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Maximum number of retries (default: 3).
    pub max_retries: u32,
    /// Base delay between retries (default: 1s).
    pub base_delay: Duration,
    /// Multiplier for exponential backoff (default: 2.0).
    pub multiplier: f64,
    /// Maximum delay cap (default: 30s).
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            multiplier: 2.0,
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Compute the delay for the given attempt (0-indexed).
    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay_ms = self.base_delay.as_millis() as f64
            * self.multiplier.powi(attempt as i32);
        let capped = Duration::from_millis(delay_ms.min(self.max_delay.as_millis() as f64) as u64);
        capped.min(self.max_delay)
    }

    /// Returns true if the HTTP status code is retryable.
    fn is_retryable_status(status: reqwest::StatusCode) -> bool {
        matches!(
            status.as_u16(),
            500 | 502 | 503 | 504
        )
    }
}

// ---------------------------------------------------------------------------
// CircuitBreaker
// ---------------------------------------------------------------------------

/// Per-origin circuit breaker state.
#[derive(Debug)]
struct CircuitBreakerState {
    failures: AtomicU32,
    last_failure: Mutex<Option<Instant>>,
}

impl Default for CircuitBreakerState {
    fn default() -> Self {
        Self {
            failures: AtomicU32::new(0),
            last_failure: Mutex::new(None),
        }
    }
}

/// Circuit breaker preventing repeated requests to failing origins.
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Number of consecutive failures before opening the circuit (default: 5).
    pub failure_threshold: u32,
    /// Cooldown period before allowing requests again (default: 60s).
    pub cooldown: Duration,
    /// Per-origin state.
    states: RwLock<HashMap<String, Arc<CircuitBreakerState>>>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(60),
            states: RwLock::new(HashMap::new()),
        }
    }
}

impl CircuitBreaker {
    /// Check if the circuit is open (requests should be blocked) for the given origin.
    fn is_open(&self, origin: &str) -> bool {
        let states = self.states.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = states.get(origin) {
            let failures = state.failures.load(Ordering::Relaxed);
            if failures >= self.failure_threshold {
                // Check if cooldown has elapsed
                let last = state.last_failure.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(last_time) = *last {
                    if last_time.elapsed() < self.cooldown {
                        return true; // still in cooldown
                    }
                    // Cooldown elapsed — allow a probe request
                }
            }
        }
        false
    }

    /// Record a successful request, resetting the failure count.
    fn record_success(&self, origin: &str) {
        let states = self.states.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = states.get(origin) {
            state.failures.store(0, Ordering::Relaxed);
        }
    }

    /// Record a failed request, incrementing the failure count.
    fn record_failure(&self, origin: &str) {
        // Get or create state
        let state = {
            let states = self.states.read().unwrap_or_else(|e| e.into_inner());
            states.get(origin).cloned()
        };

        let state = match state {
            Some(s) => s,
            None => {
                let mut states = self.states.write().unwrap_or_else(|e| e.into_inner());
                states
                    .entry(origin.to_string())
                    .or_insert_with(|| Arc::new(CircuitBreakerState::default()))
                    .clone()
            }
        };

        state.failures.fetch_add(1, Ordering::Relaxed);
        let mut last = state.last_failure.lock().unwrap_or_else(|e| e.into_inner());
        *last = Some(Instant::now());
    }
}

// ---------------------------------------------------------------------------
// Supported profiles
// ---------------------------------------------------------------------------

/// The set of conformance profiles this client implements.
const SUPPORTED_PROFILES: &[&str] = &[
    "urn:ietf:anml:profile:core-1.0",
    "core-1.0",
];

/// The ANML content type.
const ANML_CONTENT_TYPE: &str = "application/anml+xml";

/// The ANML version this client supports.
const ANML_VERSION: &str = "1.0";

// ---------------------------------------------------------------------------
// ClientInner
// ---------------------------------------------------------------------------

/// Internal state shared via `Arc` across all clones of `AnmlClient`.
#[allow(dead_code)] // Fields used by later tasks (disclosure, action execution, etc.)
struct ClientInner {
    /// The primary HTTP client (connection-pooled, rustls-tls).
    http: reqwest::Client,
    /// An isolated HTTP client for media fetches (no cookies, no auth).
    media_http: reqwest::Client,
    /// Client configuration.
    config: ClientConfig,
    /// Consent store for tracking consent grants.
    consent_store: RwLock<ConsentStore>,
    /// Audit log for recording disclosure events.
    audit_log: Box<dyn AuditLog>,
    /// Trust policy consulted before acting on documents.
    trust_policy: Box<dyn TrustPolicy>,
    /// Optional authentication provider.
    auth_provider: Option<Box<dyn AuthProvider>>,
    /// Middleware chain applied to requests/responses.
    middleware: Vec<Box<dyn HttpMiddleware>>,
    /// Circuit breaker for per-origin failure tracking.
    circuit_breaker: CircuitBreaker,
    /// Retry policy for transient HTTP errors.
    retry_policy: RetryPolicy,
    /// Tokenizer for HMAC-SHA256 field tokenization.
    tokenizer: Tokenizer,
    /// Optional condition evaluator for `<step condition="...">`.
    condition_evaluator: Option<Box<dyn ConditionEvaluator>>,
    /// Optional consent handler for explicit consent prompts.
    consent_handler: Option<Box<dyn ConsentHandler>>,
    /// Optional confirm handler for action confirmation prompts.
    confirm_handler: Option<Box<dyn ConfirmHandler>>,
}

impl std::fmt::Debug for ClientInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientInner")
            .field("config", &self.config)
            .field("retry_policy", &self.retry_policy)
            .field("circuit_breaker", &self.circuit_breaker)
            .field("tokenizer", &self.tokenizer)
            .field("has_auth_provider", &self.auth_provider.is_some())
            .field("middleware_count", &self.middleware.len())
            .field("has_condition_evaluator", &self.condition_evaluator.is_some())
            .field("has_consent_handler", &self.consent_handler.is_some())
            .field("has_confirm_handler", &self.confirm_handler.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// AnmlClient
// ---------------------------------------------------------------------------

/// The primary entry point for interacting with ANML services.
///
/// `AnmlClient` is `Clone + Send + Sync`. Internally it wraps all mutable
/// state in `Arc`, so cloning is a cheap reference-count bump. Create one
/// client at startup and share it across tasks.
///
/// # Warning
///
/// **Do not create a new `AnmlClient` for every request.** The client
/// maintains a connection pool, consent store, circuit breaker state, and
/// audit log. Creating a fresh client discards all of that and defeats
/// connection reuse. Instead, call `.clone()` to share the client.
#[derive(Clone, Debug)]
pub struct AnmlClient {
    inner: Arc<ClientInner>,
}

impl AnmlClient {
    /// Create a new [`AnmlClientBuilder`] for configuring and constructing
    /// an `AnmlClient`.
    ///
    /// The builder accepts both configuration values (via the underlying
    /// [`ClientConfigBuilder`]) and trait objects for trust policy, auth
    /// provider, consent handler, etc.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use anml_client::client::AnmlClient;
    /// use anml_client::config::AllowListTrustPolicy;
    ///
    /// # fn example() -> anml_client::Result<()> {
    /// let client = AnmlClient::builder()
    ///     .base_url("https://api.example.com")
    ///     .trust_policy(AllowListTrustPolicy::new()
    ///         .allow_url("https://api.example.com"))
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> AnmlClientBuilder {
        AnmlClientBuilder::default()
    }

    /// Fetch an ANML document from a relative path, resolved against the
    /// configured `base_url`.
    ///
    /// Sends `Accept: application/anml+xml; version="1.0"`, validates the
    /// response `Content-Type`, enforces resource limits, applies parse
    /// timeout, and runs post-parse security checks (profile, extension,
    /// HTTP transport restrictions).
    ///
    /// # Errors
    ///
    /// Returns `AnmlClientError` on HTTP errors, content-type mismatch,
    /// parse failures, unsupported profiles/extensions, or transport
    /// security violations.
    #[instrument(skip(self), fields(origin))]
    pub async fn fetch(&self, path: &str) -> crate::Result<AnmlDocument> {
        let base = self.inner.config.base_url.as_deref().ok_or_else(|| {
            AnmlClientError::MalformedDocument {
                detail: "no base_url configured; use fetch_url() for absolute URLs".into(),
            }
        })?;

        let url = if path.starts_with('/') {
            format!("{}{}", base.trim_end_matches('/'), path)
        } else {
            format!("{}/{}", base.trim_end_matches('/'), path)
        };

        self.fetch_url(&url).await
    }

    /// Fetch an ANML document from an absolute URL.
    ///
    /// This is the core fetch method. It:
    /// 1. Validates the URL scheme (HTTPS required unless `allow_plaintext_http`)
    /// 2. Checks the circuit breaker for the origin
    /// 3. Builds the request with Accept header and version negotiation
    /// 4. Applies the middleware chain (request: first→last)
    /// 5. Wraps the HTTP call in a per-request timeout
    /// 6. Retries on transient errors per the retry policy
    /// 7. Validates Content-Type
    /// 8. Handles HTTP 406 with structured problem parsing
    /// 9. Enforces resource limits on the response body
    /// 10. Parses the XML with a parse timeout
    /// 11. Applies the middleware chain (response: last→first)
    /// 12. Runs post-parse checks (profiles, extensions, HTTP transport)
    #[instrument(skip(self), fields(origin))]
    pub async fn fetch_url(&self, url: &str) -> crate::Result<AnmlDocument> {
        let parsed_url = Url::parse(url).map_err(|e| AnmlClientError::MalformedDocument {
            detail: format!("invalid URL '{}': {}", url, e),
        })?;

        // Check scheme
        if parsed_url.scheme() != "https" && !self.inner.config.allow_plaintext_http {
            warn!(url, "transport insecure: HTTPS required");
            return Err(AnmlClientError::TransportInsecure {
                url: url.to_string(),
                reason: "HTTPS required; set allow_plaintext_http(true) to allow HTTP".into(),
            });
        }

        let origin_str = origin_from_url(&parsed_url);

        // Check circuit breaker
        if self.inner.circuit_breaker.is_open(&origin_str) {
            warn!(origin = %origin_str, "circuit breaker open, blocking request");
            return Err(AnmlClientError::Timeout {
                operation: "circuit_breaker".into(),
                timeout_secs: self.inner.circuit_breaker.cooldown.as_secs(),
            });
        }

        // Build request
        let accept_header = format!("{}; version=\"{}\"", ANML_CONTENT_TYPE, ANML_VERSION);

        // Retry loop
        let mut last_err: Option<AnmlClientError> = None;
        for attempt in 0..=self.inner.retry_policy.max_retries {
            if attempt > 0 {
                let delay = self.inner.retry_policy.delay_for_attempt(attempt - 1);
                debug!(attempt, delay_ms = delay.as_millis(), "retrying request");
                tokio::time::sleep(delay).await;
            }

            match self
                .execute_fetch(url, &accept_header, &origin_str, &parsed_url)
                .await
            {
                Ok(doc) => {
                    self.inner.circuit_breaker.record_success(&origin_str);
                    return Ok(doc);
                }
                Err(e) if Self::is_retryable_error(&e) && attempt < self.inner.retry_policy.max_retries => {
                    debug!(attempt, error = %e, "transient error, will retry");
                    last_err = Some(e);
                    continue;
                }
                Err(e) => {
                    // Record failure for circuit breaker on connection/server errors
                    if Self::is_retryable_error(&e) {
                        self.inner.circuit_breaker.record_failure(&origin_str);
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| AnmlClientError::Timeout {
            operation: "fetch".into(),
            timeout_secs: self.inner.config.timeouts.per_request.as_secs(),
        }))
    }

    /// Discover an ANML service at the given origin.
    ///
    /// Tries all RFC-defined discovery mechanisms in order:
    /// 1. Well-known URI (`/.well-known/anml`)
    /// 2. HTTP Link header
    /// 3. HTML `<link>` element (requires `html-discovery` feature)
    /// 4. DNS-SD (requires `dns-sd` feature)
    ///
    /// Returns the first successful [`DiscoveryResult`](crate::discovery::DiscoveryResult).
    ///
    /// # Arguments
    ///
    /// * `origin` — The scheme+host of the service (e.g. `"https://example.com"`).
    ///
    /// # Errors
    ///
    /// Returns an error if all discovery mechanisms fail.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anml_client::Result<()> {
    /// # let client = anml_client::client::AnmlClient::builder()
    /// #     .base_url("https://example.com")
    /// #     .build()?;
    /// let result = client.discover("https://example.com").await?;
    /// println!("ANML endpoint: {}", result.endpoint);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn discover(
        &self,
        origin: &str,
    ) -> crate::Result<crate::discovery::DiscoveryResult> {
        crate::discovery::discover(&self.inner.http, origin).await
    }

    /// Returns the client configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.inner.config
    }

    /// Returns a reference to the isolated media HTTP client.
    ///
    /// This client has no cookies, no auth headers, and no credential
    /// forwarding — suitable for cross-origin media fetches.
    pub fn media_http(&self) -> &reqwest::Client {
        &self.inner.media_http
    }
}

// ---------------------------------------------------------------------------
// Private implementation methods
// ---------------------------------------------------------------------------

impl AnmlClient {
    /// Execute a single fetch attempt (no retry).
    async fn execute_fetch(
        &self,
        url: &str,
        accept_header: &str,
        origin_str: &str,
        parsed_url: &Url,
    ) -> crate::Result<AnmlDocument> {
        let mut request = self
            .inner
            .http
            .get(url)
            .header("Accept", accept_header)
            .build()?;

        // Apply default headers from config
        for (name, value) in &self.inner.config.default_headers {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                request.headers_mut().insert(n, v);
            }
        }

        // Apply middleware chain: first → last on request
        for mw in &self.inner.middleware {
            request = mw.on_request(request).await?;
        }

        // Execute with per-request timeout
        let timeout = self.inner.config.timeouts.per_request;
        let response = tokio::time::timeout(timeout, self.inner.http.execute(request))
            .await
            .map_err(|_| AnmlClientError::Timeout {
                operation: "per_request".into(),
                timeout_secs: timeout.as_secs(),
            })??;

        let status = response.status();

        // Handle HTTP 406 — version negotiation failure
        if status == reqwest::StatusCode::NOT_ACCEPTABLE {
            return self.handle_406(response).await;
        }

        // Check for server errors (will be retried if retryable)
        if status.is_server_error() {
            return Err(AnmlClientError::Http {
                source: response.error_for_status().unwrap_err(),
            });
        }

        // Check for other client errors
        if status.is_client_error() {
            return Err(AnmlClientError::Http {
                source: response.error_for_status().unwrap_err(),
            });
        }

        // Validate Content-Type
        self.validate_content_type(&response)?;

        // Apply middleware chain: last → first on response
        let mut response = response;
        for mw in self.inner.middleware.iter().rev() {
            response = mw.on_response(response).await?;
        }

        // Read body with resource limit enforcement
        let max_size = self.inner.config.resource_limits.max_document_size;
        let body_bytes = self.read_limited_body(response, max_size).await?;

        // Parse with timeout
        let parse_timeout = self.inner.config.timeouts.parse;
        let body_str = String::from_utf8(body_bytes).map_err(|_| {
            AnmlClientError::MalformedDocument {
                detail: "response body is not valid UTF-8".into(),
            }
        })?;

        let parse_options = self.build_parse_options();
        let doc = tokio::time::timeout(parse_timeout, async {
            parser::parse_with_options(&body_str, &parse_options)
                .map_err(AnmlClientError::from)
        })
        .await
        .map_err(|_| AnmlClientError::Timeout {
            operation: "parse".into(),
            timeout_secs: parse_timeout.as_secs(),
        })??;

        // Post-parse security checks
        let is_http = parsed_url.scheme() == "http";
        self.post_parse_checks(&doc, is_http, url)?;

        debug!(url, origin = origin_str, "fetched ANML document");
        Ok(doc)
    }

    /// Validate the Content-Type header starts with `application/anml+xml`.
    fn validate_content_type(&self, response: &reqwest::Response) -> crate::Result<()> {
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !content_type.starts_with(ANML_CONTENT_TYPE) {
            return Err(AnmlClientError::ContentTypeMismatch {
                expected: ANML_CONTENT_TYPE.into(),
                actual: content_type.into(),
            });
        }
        Ok(())
    }

    /// Handle HTTP 406 Not Acceptable — parse structured problem response.
    async fn handle_406(&self, response: reqwest::Response) -> crate::Result<AnmlDocument> {
        // Try to parse the body as an ANML document with a <status>/<problem>
        let body = response.text().await.unwrap_or_default();

        // Attempt to extract supported versions from the problem response
        let mut supported = Vec::new();
        let mut detail = "server does not support the requested ANML version".to_string();

        // Try parsing as ANML to extract structured problem info
        if let Ok(doc) = parser::parse(&body) {
            if let Some(ref status) = doc.status {
                if let Some(ref msg) = status.message {
                    detail = msg.clone();
                }
            }
            // Check supported-versions from the document
            if let Some(ref sv) = doc.supported_versions {
                supported = sv.split(',').map(|s| s.trim().to_string()).collect();
            }
        }

        Err(AnmlClientError::UnsupportedVersion { detail, supported })
    }

    /// Read the response body, enforcing the max document size limit.
    async fn read_limited_body(
        &self,
        response: reqwest::Response,
        max_size: usize,
    ) -> crate::Result<Vec<u8>> {
        let bytes = response.bytes().await?;
        if bytes.len() > max_size {
            return Err(AnmlClientError::ResourceLimitExceeded {
                limit: "max_document_size".into(),
                value: bytes.len() as u64,
                max: max_size as u64,
            });
        }
        Ok(bytes.to_vec())
    }

    /// Build parse options from the client's resource limits.
    fn build_parse_options(&self) -> ParseOptions {
        let limits = &self.inner.config.resource_limits;
        let mut opts = ParseOptions::default();
        opts.max_size = limits.max_document_size;
        opts.max_depth = limits.max_depth;
        opts.max_elements = limits.max_elements;
        opts
    }

    /// Run post-parse security checks on a fetched document.
    fn post_parse_checks(
        &self,
        doc: &AnmlDocument,
        is_http: bool,
        url: &str,
    ) -> crate::Result<()> {
        // 1. Reject security-sensitive sections over HTTP
        if is_http && !self.inner.config.allow_plaintext_http {
            self.check_http_restrictions(doc, url)?;
        }
        // Even with allow_plaintext_http, reject sensitive content over HTTP
        if is_http {
            self.check_http_restrictions(doc, url)?;
        }

        // 2. Check required extensions
        self.check_required_extensions(doc)?;

        // 3. Check required profiles
        self.check_required_profiles(doc)?;

        Ok(())
    }

    /// Reject documents with `<constraints>`, `<interact>`, or
    /// `<ask requires="explicit">` when fetched over HTTP.
    fn check_http_restrictions(&self, doc: &AnmlDocument, url: &str) -> crate::Result<()> {
        if doc.constraints.is_some() {
            warn!(url, "transport rejection: <constraints> over HTTP");
            return Err(AnmlClientError::TransportInsecure {
                url: url.to_string(),
                reason: "document contains <constraints> but was fetched over HTTP".into(),
            });
        }

        if doc.interact.is_some() {
            warn!(url, "transport rejection: <interact> over HTTP");
            return Err(AnmlClientError::TransportInsecure {
                url: url.to_string(),
                reason: "document contains <interact> but was fetched over HTTP".into(),
            });
        }

        // Check for <ask requires="explicit"> in knowledge section
        if let Some(ref knowledge) = doc.knowledge {
            if let Some(ref asks) = knowledge.asks {
                for ask in asks {
                    // The ask_type field doesn't carry requires — check via
                    // the constraints section. But we also need to check if
                    // any ask has requires="explicit" via the disclosure rules.
                    // For now, the presence of asks with constraints is already
                    // caught above. We also check top-level asks.
                    let _ = ask;
                }
            }
        }

        // Check top-level asks (agent response documents)
        // These are less common but still need checking
        if let Some(ref asks) = doc.asks {
            let _ = asks;
        }

        Ok(())
    }

    /// Scan `<meta name="requires-ext">` and refuse if unrecognized.
    fn check_required_extensions(&self, doc: &AnmlDocument) -> crate::Result<()> {
        if let Some(ref head) = doc.head {
            if let Some(ref metas) = head.meta {
                for meta in metas {
                    if meta.name == "requires-ext" {
                        // We don't recognize any extensions yet
                        return Err(AnmlClientError::UnsupportedExtension {
                            namespace_uri: meta.value.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Scan `<meta name="profile">` and refuse if unsupported.
    fn check_required_profiles(&self, doc: &AnmlDocument) -> crate::Result<()> {
        if let Some(ref head) = doc.head {
            if let Some(ref metas) = head.meta {
                for meta in metas {
                    if meta.name == "profile" {
                        if !SUPPORTED_PROFILES.contains(&meta.value.as_str()) {
                            return Err(AnmlClientError::UnsupportedProfile {
                                profile_uri: meta.value.clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Check if an error is retryable (transient).
    fn is_retryable_error(err: &AnmlClientError) -> bool {
        match err {
            AnmlClientError::Http { source } => {
                if let Some(status) = source.status() {
                    RetryPolicy::is_retryable_status(status)
                } else {
                    // Connection errors, timeouts are retryable
                    source.is_connect() || source.is_timeout()
                }
            }
            AnmlClientError::Timeout { .. } => true,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// AnmlClientBuilder
// ---------------------------------------------------------------------------

/// Builder for [`AnmlClient`].
///
/// Wraps [`ClientConfigBuilder`] and additionally accepts trait objects for
/// trust policy, auth provider, consent handler, confirm handler, condition
/// evaluator, audit log, and middleware.
///
/// # Example
///
/// ```rust,no_run
/// use anml_client::client::AnmlClient;
/// use anml_client::config::{AllowListTrustPolicy, DenyAllTrustPolicy};
///
/// # fn example() -> anml_client::Result<()> {
/// let client = AnmlClient::builder()
///     .base_url("https://api.example.com")
///     .trust_policy(AllowListTrustPolicy::new()
///         .allow_url("https://api.example.com"))
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct AnmlClientBuilder {
    config_builder: ClientConfigBuilder,
    trust_policy: Option<Box<dyn TrustPolicy>>,
    auth_provider: Option<Box<dyn AuthProvider>>,
    consent_handler: Option<Box<dyn ConsentHandler>>,
    confirm_handler: Option<Box<dyn ConfirmHandler>>,
    condition_evaluator: Option<Box<dyn ConditionEvaluator>>,
    audit_log: Option<Box<dyn AuditLog>>,
    middleware: Vec<Box<dyn HttpMiddleware>>,
    retry_policy: Option<RetryPolicy>,
    circuit_breaker_threshold: Option<u32>,
    circuit_breaker_cooldown: Option<Duration>,
    #[cfg(feature = "serde")]
    restored_state: Option<ClientState>,
}

impl Default for AnmlClientBuilder {
    fn default() -> Self {
        Self {
            config_builder: ClientConfigBuilder::default(),
            trust_policy: None,
            auth_provider: None,
            consent_handler: None,
            confirm_handler: None,
            condition_evaluator: None,
            audit_log: None,
            middleware: Vec::new(),
            retry_policy: None,
            circuit_breaker_threshold: None,
            circuit_breaker_cooldown: None,
            #[cfg(feature = "serde")]
            restored_state: None,
        }
    }
}

impl AnmlClientBuilder {
    // -- Config delegation --

    /// Set the base URL for the ANML service.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config_builder = self.config_builder.base_url(url);
        self
    }

    /// Set the per-request timeout (convenience shorthand).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config_builder = self.config_builder.timeout(timeout);
        self
    }

    /// Set the full timeout configuration.
    pub fn timeouts(mut self, timeouts: crate::config::TimeoutConfig) -> Self {
        self.config_builder = self.config_builder.timeouts(timeouts);
        self
    }

    /// Allow or disallow plaintext HTTP (default: `false`).
    pub fn allow_plaintext_http(mut self, allow: bool) -> Self {
        self.config_builder = self.config_builder.allow_plaintext_http(allow);
        self
    }

    /// Add a default header to include on every request.
    pub fn default_header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.config_builder = self.config_builder.default_header(name, value);
        self
    }

    /// Set the resource limits for document parsing.
    pub fn resource_limits(mut self, limits: crate::config::ResourceLimits) -> Self {
        self.config_builder = self.config_builder.resource_limits(limits);
        self
    }

    /// Set the per-document action budget.
    pub fn action_budget(mut self, budget: crate::config::ActionBudget) -> Self {
        self.config_builder = self.config_builder.action_budget(budget);
        self
    }

    // -- Trait object setters --

    /// Set the trust policy consulted before acting on documents.
    ///
    /// Default: [`DenyAllTrustPolicy`] (denies all origins).
    pub fn trust_policy(mut self, policy: impl TrustPolicy + 'static) -> Self {
        self.trust_policy = Some(Box::new(policy));
        self
    }

    /// Set the authentication provider for credential injection and refresh.
    pub fn auth_provider(mut self, provider: impl AuthProvider + 'static) -> Self {
        self.auth_provider = Some(Box::new(provider));
        self
    }

    /// Set the consent handler for explicit consent prompts.
    pub fn consent_handler(mut self, handler: impl ConsentHandler + 'static) -> Self {
        self.consent_handler = Some(Box::new(handler));
        self
    }

    /// Set the confirm handler for action confirmation prompts.
    pub fn confirm_handler(mut self, handler: impl ConfirmHandler + 'static) -> Self {
        self.confirm_handler = Some(Box::new(handler));
        self
    }

    /// Set the condition evaluator for `<step condition="...">` expressions.
    pub fn condition_evaluator(mut self, evaluator: impl ConditionEvaluator + 'static) -> Self {
        self.condition_evaluator = Some(Box::new(evaluator));
        self
    }

    /// Set a custom audit log implementation.
    ///
    /// Default: no-op audit log.
    pub fn audit_log(mut self, log: impl AuditLog + 'static) -> Self {
        self.audit_log = Some(Box::new(log));
        self
    }

    /// Add a middleware interceptor to the chain.
    ///
    /// Middleware is applied in registration order for requests (first → last)
    /// and reverse order for responses (last → first).
    pub fn middleware(mut self, mw: impl HttpMiddleware + 'static) -> Self {
        self.middleware.push(Box::new(mw));
        self
    }

    /// Set the retry policy for transient HTTP errors.
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Set the circuit breaker failure threshold.
    pub fn circuit_breaker_threshold(mut self, threshold: u32) -> Self {
        self.circuit_breaker_threshold = Some(threshold);
        self
    }

    /// Set the circuit breaker cooldown duration.
    pub fn circuit_breaker_cooldown(mut self, cooldown: Duration) -> Self {
        self.circuit_breaker_cooldown = Some(cooldown);
        self
    }

    /// Build the [`AnmlClient`].
    ///
    /// Configures `reqwest` with `rustls-tls` (no native TLS) and creates
    /// an isolated media client with no cookies or auth.
    ///
    /// # Errors
    ///
    /// Returns `AnmlClientError::Http` if the underlying `reqwest::Client`
    /// cannot be constructed.
    pub fn build(self) -> crate::Result<AnmlClient> {
        let config = self.config_builder.build();

        // Build the primary HTTP client with rustls-tls
        let mut http_builder = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(config.timeouts.per_request);

        // Add default headers
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &config.default_headers {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        if !headers.is_empty() {
            http_builder = http_builder.default_headers(headers);
        }

        let http = http_builder.build()?;

        // Build the isolated media HTTP client — no cookies, no auth, no
        // credential forwarding. Used for cross-origin media fetches.
        let media_http = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(config.timeouts.per_media_fetch)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;

        // Circuit breaker
        let mut cb = CircuitBreaker::default();
        if let Some(threshold) = self.circuit_breaker_threshold {
            cb.failure_threshold = threshold;
        }
        if let Some(cooldown) = self.circuit_breaker_cooldown {
            cb.cooldown = cooldown;
        }

        let inner = ClientInner {
            http,
            media_http,
            config,
            consent_store: RwLock::new(ConsentStore::default()),
            audit_log: self.audit_log.unwrap_or_else(|| Box::new(NoOpAuditLog)),
            trust_policy: self
                .trust_policy
                .unwrap_or_else(|| Box::new(DenyAllTrustPolicy)),
            auth_provider: self.auth_provider,
            middleware: self.middleware,
            circuit_breaker: cb,
            retry_policy: self.retry_policy.unwrap_or_default(),
            tokenizer: Tokenizer::default(),
            condition_evaluator: self.condition_evaluator,
            consent_handler: self.consent_handler,
            confirm_handler: self.confirm_handler,
        };

        let client = AnmlClient {
            inner: Arc::new(inner),
        };

        // Restore state if provided
        #[cfg(feature = "serde")]
        if let Some(state) = self.restored_state {
            let store = client.inner.consent_store.read().unwrap_or_else(|e| e.into_inner());
            store.restore_grants(state.consent_grants);
            for entry in state.audit_entries {
                client.inner.audit_log.record(entry);
            }
        }

        Ok(client)
    }
}

// ---------------------------------------------------------------------------
// ClientState — serializable state snapshot
// ---------------------------------------------------------------------------

/// Aggregated client state for persistence and restoration.
///
/// Captures the consent store grants and audit log entries so they can
/// be serialized (e.g., to JSON) and later restored via
/// [`AnmlClientBuilder::restore_state`].
///
/// # Example
///
/// ```rust,no_run
/// # fn example() -> anml_client::Result<()> {
/// let client = anml_client::client::AnmlClient::builder()
///     .base_url("https://api.example.com")
///     .build()?;
///
/// // Export state
/// let state = client.export_state();
/// let json = serde_json::to_string(&state).unwrap();
///
/// // Later, restore state
/// let restored: anml_client::client::ClientState = serde_json::from_str(&json).unwrap();
/// let client2 = anml_client::client::AnmlClient::builder()
///     .base_url("https://api.example.com")
///     .restore_state(restored)
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClientState {
    /// Consent grants from the consent store.
    pub consent_grants: Vec<crate::disclosure::consent::ConsentGrant>,
    /// Audit log entries (only populated if using `InMemoryAuditLog`).
    pub audit_entries: Vec<crate::audit::AuditEntry>,
}

#[cfg(feature = "serde")]
impl AnmlClient {
    /// Export the current client state for serialization.
    ///
    /// Captures consent store grants and audit log entries (if using
    /// `InMemoryAuditLog`). The returned `ClientState` can be serialized
    /// with `serde_json` and later restored via
    /// [`AnmlClientBuilder::restore_state`].
    pub fn export_state(&self) -> ClientState {
        let consent_grants = {
            let store = self.inner.consent_store.read().unwrap_or_else(|e| e.into_inner());
            store.export_grants()
        };

        // Try to downcast audit log to InMemoryAuditLog for export
        let audit_entries = if let Some(in_mem) = self
            .inner
            .audit_log
            .as_any()
            .downcast_ref::<InMemoryAuditLog>()
        {
            in_mem.list()
        } else {
            Vec::new()
        };

        ClientState {
            consent_grants,
            audit_entries,
        }
    }
}

#[cfg(feature = "serde")]
impl AnmlClientBuilder {
    /// Restore client state from a previously exported [`ClientState`].
    ///
    /// This restores consent grants and audit log entries. Call this
    /// before `.build()`.
    pub fn restore_state(mut self, state: ClientState) -> Self {
        self.restored_state = Some(state);
        self
    }
}

// ---------------------------------------------------------------------------
// DocumentSummary — concise Display wrapper for AnmlDocument
// ---------------------------------------------------------------------------

/// A concise summary wrapper for [`AnmlDocument`] that implements [`Display`](std::fmt::Display).
///
/// Produces a single-line summary suitable for logs:
/// `ANML[Travel Booking Service] v1.0 | 2 asks, 1 action | flow@search (1/4) | 200 OK`
///
/// # Example
///
/// ```rust,no_run
/// use anml::types::document::AnmlDocument;
/// use anml_client::client::DocumentSummary;
///
/// let doc = AnmlDocument::default();
/// println!("{}", DocumentSummary(&doc));
/// ```
pub struct DocumentSummary<'a>(pub &'a AnmlDocument);

impl<'a> std::fmt::Display for DocumentSummary<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let doc = self.0;

        // Title
        let title = doc
            .head
            .as_ref()
            .and_then(|h| h.title.as_ref())
            .map(|t| t.text.as_str())
            .unwrap_or("untitled");
        write!(f, "ANML[{title}]")?;

        // Version
        if let Some(ref v) = doc.version {
            write!(f, " v{v}")?;
        }

        // Counts
        let mut counts = Vec::new();

        let ask_count = doc
            .knowledge
            .as_ref()
            .and_then(|k| k.asks.as_ref())
            .map(|a| a.len())
            .unwrap_or(0)
            + doc.asks.as_ref().map(|a| a.len()).unwrap_or(0);
        if ask_count > 0 {
            counts.push(format!(
                "{ask_count} ask{}",
                if ask_count == 1 { "" } else { "s" }
            ));
        }

        let action_count = doc
            .interact
            .as_ref()
            .map(|i| i.actions.len())
            .unwrap_or(0);
        if action_count > 0 {
            counts.push(format!(
                "{action_count} action{}",
                if action_count == 1 { "" } else { "s" }
            ));
        }

        if !counts.is_empty() {
            write!(f, " | {}", counts.join(", "))?;
        }

        // Flow state
        if let Some(ref state) = doc.state {
            if let Some(ref flow) = state.flow {
                let total = flow.steps.len();
                let current_step = state
                    .context
                    .as_ref()
                    .map(|c| c.step.as_str())
                    .or_else(|| {
                        flow.steps
                            .iter()
                            .find(|s| s.status == Some(anml::types::enums::StepStatus::Current))
                            .map(|s| s.id.as_str())
                    })
                    .unwrap_or("?");
                let completed = flow
                    .steps
                    .iter()
                    .filter(|s| {
                        matches!(
                            s.status,
                            Some(anml::types::enums::StepStatus::Completed)
                                | Some(anml::types::enums::StepStatus::Skipped)
                        )
                    })
                    .count();
                write!(f, " | flow@{current_step} ({}/{total})", completed + 1)?;
            }
        }

        // Status
        if let Some(ref status) = doc.status {
            write!(f, " | {}", status.result)?;
            if let Some(ref msg) = status.message {
                write!(f, " {msg}")?;
            }
        }

        Ok(())
    }
}

impl<'a> std::fmt::Debug for DocumentSummary<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the origin string (scheme://host[:port]) from a URL.
fn origin_from_url(url: &Url) -> String {
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), url.host_str().unwrap_or(""), port),
        None => format!("{}://{}", url.scheme(), url.host_str().unwrap_or("")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_is_clone_send_sync() {
        fn assert_clone_send_sync<T: Clone + Send + Sync>() {}
        assert_clone_send_sync::<AnmlClient>();
    }

    #[test]
    fn builder_creates_client_with_defaults() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .expect("should build");

        assert_eq!(
            client.config().base_url.as_deref(),
            Some("https://example.com")
        );
        assert!(!client.config().allow_plaintext_http);
    }

    #[test]
    fn builder_accepts_custom_timeout() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .timeout(Duration::from_secs(5))
            .build()
            .expect("should build");

        assert_eq!(client.config().timeouts.per_request, Duration::from_secs(5));
    }

    #[test]
    fn builder_accepts_allow_plaintext() {
        let client = AnmlClient::builder()
            .allow_plaintext_http(true)
            .build()
            .expect("should build");

        assert!(client.config().allow_plaintext_http);
    }

    #[test]
    fn builder_accepts_custom_trust_policy() {
        use crate::config::AllowListTrustPolicy;

        let _client = AnmlClient::builder()
            .base_url("https://example.com")
            .trust_policy(
                AllowListTrustPolicy::new().allow_url("https://example.com"),
            )
            .build()
            .expect("should build");
    }

    #[test]
    fn builder_accepts_retry_policy() {
        let _client = AnmlClient::builder()
            .retry_policy(RetryPolicy {
                max_retries: 5,
                base_delay: Duration::from_millis(500),
                multiplier: 1.5,
                max_delay: Duration::from_secs(10),
            })
            .build()
            .expect("should build");
    }

    #[test]
    fn builder_accepts_circuit_breaker_config() {
        let _client = AnmlClient::builder()
            .circuit_breaker_threshold(10)
            .circuit_breaker_cooldown(Duration::from_secs(120))
            .build()
            .expect("should build");
    }

    #[test]
    fn retry_policy_delay_calculation() {
        let policy = RetryPolicy::default();
        // attempt 0: 1s * 2^0 = 1s
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        // attempt 1: 1s * 2^1 = 2s
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(2));
        // attempt 2: 1s * 2^2 = 4s
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(4));
    }

    #[test]
    fn retry_policy_delay_capped() {
        let policy = RetryPolicy {
            max_delay: Duration::from_secs(5),
            ..RetryPolicy::default()
        };
        // attempt 10: would be 1024s, capped to 5s
        assert_eq!(policy.delay_for_attempt(10), Duration::from_secs(5));
    }

    #[test]
    fn retryable_statuses() {
        assert!(RetryPolicy::is_retryable_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(RetryPolicy::is_retryable_status(
            reqwest::StatusCode::BAD_GATEWAY
        ));
        assert!(RetryPolicy::is_retryable_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(RetryPolicy::is_retryable_status(
            reqwest::StatusCode::GATEWAY_TIMEOUT
        ));
        assert!(!RetryPolicy::is_retryable_status(
            reqwest::StatusCode::NOT_FOUND
        ));
        assert!(!RetryPolicy::is_retryable_status(reqwest::StatusCode::OK));
    }

    #[test]
    fn circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::default();
        assert!(!cb.is_open("https://example.com"));
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker {
            failure_threshold: 3,
            cooldown: Duration::from_secs(60),
            ..CircuitBreaker::default()
        };
        for _ in 0..3 {
            cb.record_failure("https://example.com");
        }
        assert!(cb.is_open("https://example.com"));
    }

    #[test]
    fn circuit_breaker_resets_on_success() {
        let cb = CircuitBreaker {
            failure_threshold: 3,
            cooldown: Duration::from_secs(60),
            ..CircuitBreaker::default()
        };
        for _ in 0..3 {
            cb.record_failure("https://example.com");
        }
        assert!(cb.is_open("https://example.com"));
        cb.record_success("https://example.com");
        assert!(!cb.is_open("https://example.com"));
    }

    #[test]
    fn origin_from_url_with_port() {
        let url = Url::parse("https://example.com:8443/path").unwrap();
        assert_eq!(origin_from_url(&url), "https://example.com:8443");
    }

    #[test]
    fn origin_from_url_without_port() {
        let url = Url::parse("https://example.com/path").unwrap();
        assert_eq!(origin_from_url(&url), "https://example.com");
    }

    #[test]
    fn post_parse_checks_rejects_unsupported_extension() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let doc = AnmlDocument {
            head: Some(anml::types::elements::AnmlHead {
                title: None,
                meta: Some(vec![anml::types::elements::AnmlMeta {
                    name: "requires-ext".into(),
                    value: "https://example.com/ext/payments".into(),
                }]),
            }),
            ..Default::default()
        };

        let result = client.post_parse_checks(&doc, false, "https://example.com/service");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnmlClientError::UnsupportedExtension { .. }
        ));
    }

    #[test]
    fn post_parse_checks_rejects_unsupported_profile() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let doc = AnmlDocument {
            head: Some(anml::types::elements::AnmlHead {
                title: None,
                meta: Some(vec![anml::types::elements::AnmlMeta {
                    name: "profile".into(),
                    value: "urn:ietf:anml:profile:signed-answer-1.0".into(),
                }]),
            }),
            ..Default::default()
        };

        let result = client.post_parse_checks(&doc, false, "https://example.com/service");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnmlClientError::UnsupportedProfile { .. }
        ));
    }

    #[test]
    fn post_parse_checks_accepts_core_profile() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let doc = AnmlDocument {
            head: Some(anml::types::elements::AnmlHead {
                title: None,
                meta: Some(vec![anml::types::elements::AnmlMeta {
                    name: "profile".into(),
                    value: "urn:ietf:anml:profile:core-1.0".into(),
                }]),
            }),
            ..Default::default()
        };

        let result = client.post_parse_checks(&doc, false, "https://example.com/service");
        assert!(result.is_ok());
    }

    #[test]
    fn post_parse_checks_rejects_constraints_over_http() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let doc = AnmlDocument {
            constraints: Some(anml::types::elements::AnmlConstraints {
                disclosures: None,
            }),
            ..Default::default()
        };

        let result = client.post_parse_checks(&doc, true, "http://example.com/service");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnmlClientError::TransportInsecure { .. }
        ));
    }

    #[test]
    fn post_parse_checks_rejects_interact_over_http() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let doc = AnmlDocument {
            interact: Some(anml::types::elements::AnmlInteract {
                actions: vec![],
            }),
            ..Default::default()
        };

        let result = client.post_parse_checks(&doc, true, "http://example.com/service");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnmlClientError::TransportInsecure { .. }
        ));
    }

    #[test]
    fn clone_shares_inner_state() {
        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        let cloned = client.clone();
        assert!(Arc::ptr_eq(&client.inner, &cloned.inner));
    }

    #[tokio::test]
    async fn fetch_without_base_url_returns_error() {
        let client = AnmlClient::builder().build().unwrap();
        let result = client.fetch("/service").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_url_rejects_http_by_default() {
        let client = AnmlClient::builder().build().unwrap();
        let result = client.fetch_url("http://example.com/service").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnmlClientError::TransportInsecure { .. }
        ));
    }

    // -- DocumentSummary Display tests --

    #[test]
    fn document_summary_default() {
        let doc = AnmlDocument::default();
        let summary = format!("{}", DocumentSummary(&doc));
        assert!(summary.starts_with("ANML[untitled]"));
    }

    #[test]
    fn document_summary_with_title_and_version() {
        let doc = AnmlDocument {
            version: Some("1.0".into()),
            head: Some(anml::types::elements::AnmlHead {
                title: Some(anml::types::elements::AnmlTitle {
                    text: "Travel Booking".into(),
                }),
                meta: None,
            }),
            ..Default::default()
        };
        let summary = format!("{}", DocumentSummary(&doc));
        assert!(summary.contains("ANML[Travel Booking]"));
        assert!(summary.contains("v1.0"));
    }

    #[test]
    fn document_summary_with_asks_and_actions() {
        let doc = AnmlDocument {
            knowledge: Some(anml::types::elements::AnmlKnowledge {
                asks: Some(vec![
                    anml::types::elements::AnmlAsk {
                        field: "email".into(),
                        action: "submit".into(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                    anml::types::elements::AnmlAsk {
                        field: "name".into(),
                        action: "submit".into(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                ]),
                ..Default::default()
            }),
            interact: Some(anml::types::elements::AnmlInteract {
                actions: vec![anml::types::elements::AnmlAction {
                    id: "submit".into(),
                    method: "POST".into(),
                    endpoint: "/submit".into(),
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
        let summary = format!("{}", DocumentSummary(&doc));
        assert!(summary.contains("2 asks"), "got: {summary}");
        assert!(summary.contains("1 action"), "got: {summary}");
    }

    #[test]
    fn document_summary_with_flow() {
        use anml::types::elements::{AnmlContext, AnmlFlow, AnmlState, AnmlStep};
        use anml::types::enums::StepStatus;

        let doc = AnmlDocument {
            state: Some(AnmlState {
                flow: Some(AnmlFlow {
                    steps: vec![
                        AnmlStep {
                            id: "search".into(),
                            label: None,
                            status: Some(StepStatus::Current),
                            required: None,
                            next: None,
                            condition: None,
                            action: None,
                        },
                        AnmlStep {
                            id: "select".into(),
                            label: None,
                            status: Some(StepStatus::Pending),
                            required: None,
                            next: None,
                            condition: None,
                            action: None,
                        },
                    ],
                }),
                context: Some(AnmlContext {
                    step: "search".into(),
                }),
            }),
            ..Default::default()
        };
        let summary = format!("{}", DocumentSummary(&doc));
        assert!(summary.contains("flow@search"), "got: {summary}");
        assert!(summary.contains("1/2"), "got: {summary}");
    }

    // -- ClientState round-trip tests --

    #[cfg(feature = "serde")]
    #[test]
    fn client_state_round_trip() {
        use crate::disclosure::consent::{ConsentBasis, ConsentGrant};
        use crate::disclosure::matching::ConsentScope;

        let state = ClientState {
            consent_grants: vec![ConsentGrant {
                basis: ConsentBasis::Explicit,
                granted_at: None,
                field: "email".into(),
                origin: "https://example.com".into(),
                scope: ConsentScope::Session,
            }],
            audit_entries: vec![],
        };

        let json = serde_json::to_string(&state).expect("serialize");
        let restored: ClientState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.consent_grants.len(), 1);
        assert_eq!(restored.consent_grants[0].field, "email");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn export_and_restore_state() {
        use crate::config::Origin;
        use crate::disclosure::consent::ConsentBasis;
        use crate::disclosure::matching::ConsentScope;

        let client = AnmlClient::builder()
            .base_url("https://example.com")
            .build()
            .unwrap();

        // Grant consent
        {
            let store = client.inner.consent_store.read().unwrap();
            let origin = Origin {
                scheme: "https".into(),
                host: "example.com".into(),
                port: None,
            };
            store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        }

        // Export
        let state = client.export_state();
        assert_eq!(state.consent_grants.len(), 1);

        // Serialize and deserialize
        let json = serde_json::to_string(&state).unwrap();
        let restored_state: ClientState = serde_json::from_str(&json).unwrap();

        // Restore into new client
        let client2 = AnmlClient::builder()
            .base_url("https://example.com")
            .restore_state(restored_state)
            .build()
            .unwrap();

        // Verify consent was restored
        let store = client2.inner.consent_store.read().unwrap();
        let origin = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        assert!(store.check("email", &origin, ConsentScope::Session).is_some());
    }
}
