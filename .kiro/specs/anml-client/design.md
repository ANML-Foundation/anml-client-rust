# ANML Client for Rust — Design

## Architecture Overview

```
anml-client/
├── src/
│   ├── lib.rs              → Public API re-exports
│   ├── client.rs           → AnmlClient struct (main entry point)
│   ├── config.rs           → ClientConfig builder
│   ├── discovery/
│   │   ├── mod.rs          → Discovery trait + orchestrator
│   │   ├── well_known.rs   → /.well-known/anml fetcher
│   │   ├── link_header.rs  → HTTP Link header parser
│   │   ├── html_link.rs    → HTML <link> parser (feature-gated)
│   │   └── dns_sd.rs       → DNS-SD resolver (feature-gated)
│   ├── disclosure/
│   │   ├── mod.rs          → Disclosure evaluation engine
│   │   ├── matching.rs     → field / field-prefix / field-pattern matching
│   │   ├── consent.rs      → Consent store (session/origin/global)
│   │   └── rate_limit.rs   → Per-field rate limit tracker
│   ├── action/
│   │   ├── mod.rs          → Action executor
│   │   ├── params.rs       → Parameter binding algorithm (3 enctypes)
│   │   ├── validation.rs   → Param type/pattern/range validation
│   │   └── idempotency.rs  → Idempotency-Key generation + retry logic
│   ├── flow/
│   │   ├── mod.rs          → Flow navigator
│   │   └── recovery.rs     → next-on-error + retry-budget with backoff
│   ├── knowledge/
│   │   ├── mod.rs          → Knowledge exchange helpers
│   │   └── response.rs     → Answer/Refuse/Ask/Inform builder wrappers
│   ├── integrity.rs        → SRI verification (sha256/sha384/sha512)
│   ├── pagination.rs       → Nav-based paginated iterator/stream
│   ├── cache.rs            → TTL-based document cache (feature-gated)
│   ├── security/
│   │   ├── mod.rs          → Security module re-exports
│   │   ├── trust.rs        → TrustPolicy trait + AllowListTrustPolicy + DenyAllTrustPolicy
│   │   ├── budget.rs       → ActionBudget, MediaBudget, per-document resource tracking
│   │   ├── transport.rs    → HTTPS enforcement, Content-Type validation, SSRF checks
│   │   └── tokenizer.rs    → HMAC-SHA256 tokenization (field, principal, origin binding)
│   ├── audit.rs            → AuditLog trait + InMemoryAuditLog + AuditEntry
│   ├── middleware.rs       → HttpMiddleware trait + composable interceptor chain
│   ├── retry.rs            → RetryPolicy, exponential backoff, circuit breaker
│   ├── auth.rs             → AuthProvider trait (bearer, API key, custom headers, async refresh)
│   ├── testing/            → (feature-gated `testing`)
│   │   ├── mod.rs          → Test module re-exports
│   │   ├── mock_server.rs  → MockAnmlServer (in-process HTTP, configurable responses)
│   │   ├── fixtures.rs     → Pre-built AnmlDocument fixtures for common patterns
│   │   └── assertions.rs   → Assertion helpers for disclosure, consent, action params
│   └── error.rs            → Client error types
├── Cargo.toml
├── README.md
└── examples/
    ├── basic_fetch.rs
    ├── action_execution.rs
    └── multi_step_flow.rs
```

## Key Design Decisions

### 1. Depend on `anml` crate via git, not path

The client depends on `anml` as a git dependency from the private repo. The `_reference/anml-server-rust` submodule exists for reading the source and understanding the API — it is NOT used as a path dependency. All client code is self-contained in this repo.

```toml
[dependencies]
anml = { git = "https://github.com/Life-Savor-AI/anml-server-rust.git", features = ["serde"] }
```

### 2. `AnmlClient` as the primary entry point

```rust
let client = AnmlClient::builder()
    .base_url("https://api.example.com")
    .timeout(Duration::from_secs(30))
    .build()?;

// Fetch and parse
let doc = client.fetch("/service").await?;

// Inspect
println!("Title: {:?}", doc.title());
for ask in doc.asks() {
    println!("Service asks for: {}", ask.field);
}

// Execute an action with disclosure check
let response = client.execute_action(&doc, "submit-airline", &[
    ("airline", "Delta"),
]).await?;
```

### 3. Disclosure evaluation is mandatory and automatic

The `execute_action` and `answer` methods MUST run the disclosure algorithm before emitting any `<answer>`. The client maintains a `ConsentStore` that tracks consent grants by scope (session/origin/global).

```rust
// The consent callback is how the client asks the principal
let client = AnmlClient::builder()
    .consent_handler(|field, disclosure| {
        // Return ConsentDecision::Grant or ConsentDecision::Deny
    })
    .build()?;
```

For `requires="explicit"`, the consent handler is always called. For `requires="implicit"`, the client checks the consent store first. For `requires="none"`, no consent is needed (but trust policy may still block).

### 4. Action execution encapsulates the full RFC flow

`client.execute_action()` does:
1. Resolve the `<action>` by id from `<interact>`
2. Collect and validate parameters against `<param>` definitions
3. Run disclosure evaluation for any `<answer>` values being sent
4. Serialize the request body per the action's `enctype` using the parameter binding algorithm
5. If `confirm="true"`, invoke the confirm callback
6. If `idempotent="true"`, generate and attach `Idempotency-Key`
7. Perform the HTTP request
8. Parse the response as an ANML document
9. Return the parsed response

### 5. Flow navigation is stateful

```rust
let mut flow = client.flow(&doc)?;
println!("Current step: {:?}", flow.current());
println!("Next steps: {:?}", flow.pending());

// Execute the current step's action
let next_doc = flow.advance(&[("reason", "damaged")]).await?;
// flow automatically updates state from the response
```

The `FlowNavigator` tracks step state, handles `next-on-error` transitions, and enforces `retry-budget` with exponential backoff.

### 6. Pagination as an async stream

```rust
let mut pages = client.paginate(&doc, "flights").await;
while let Some(page) = pages.next().await {
    for item in page?.items() {
        println!("{:?}", item);
    }
}
```

### 7. Error types map to RFC problem URIs

```rust
pub enum AnmlClientError {
    // HTTP-level
    Http(reqwest::Error),
    // ANML parse/validation
    Parse(anml::errors::AnmlError),
    // Transport security
    TransportInsecure { url: String, reason: String },
    ContentTypeMismatch { expected: String, actual: String },
    // RFC problem types
    UnsupportedVersion { detail: String, supported: Vec<String> },
    UnsupportedProfile { profile_uri: String },
    UnsupportedExtension { namespace_uri: String },
    MalformedDocument { detail: String },
    FlowAborted { step_id: String, detail: String },
    IntegrityMismatch { element: String, expected: String, observed: String },
    ResourceLimitExceeded { limit: String, value: u64, max: u64 },
    // Disclosure
    ConsentDenied { field: String },
    RateLimited { field: String, retry_after: Option<u64> },
    // Trust
    TrustInsufficient { origin: String, reason: String },
    // Action safety
    ActionBudgetExceeded { budget_type: String, limit: u32 },
    SsrfBlocked { endpoint: String },
    // State
    UnexpectedStateRegression { step_id: String, from: String, to: String },
}
```

### 8. Feature flags

| Flag | Default | Adds |
|------|---------|------|
| `dns-sd` | — | `hickory-resolver` for DNS-SD discovery (pure Rust) |
| `html-discovery` | — | `scraper`/`html5ever` for HTML link parsing (pure Rust) |
| `cache` | — | In-memory TTL cache (`moka` or similar) |
| `blocking` | — | `tokio::runtime::Runtime` sync wrapper |
| `testing` | — | `MockAnmlServer`, fixtures, assertion helpers (`axum` + `tokio-test`) |
| `serde` | ✓ | Serde derives on client state types (consent store, audit log, flow state) |

Note: TLS is always `rustls` (pure Rust). The `reqwest` dependency uses `rustls-tls` feature, never `native-tls`. The `wasm` flag is reserved for future use.

### 9. Consent store design

The consent store is an in-memory structure tracking granted consents:

```rust
struct ConsentStore {
    // (origin, field) -> ConsentGrant for origin-scoped
    // field -> ConsentGrant for global-scoped
    // session consents are ephemeral per client instance
    grants: HashMap<ConsentKey, ConsentGrant>,
}

struct ConsentGrant {
    scope: ConsentScope,
    basis: ConsentBasis,  // explicit, implicit, delegated
    granted_at: Instant,
}
```

### 10. SRI verification

For media elements with `inference="required"`, the client fetches the resource, computes the digest, and compares against the `integrity` attribute before returning the bytes. The strongest supported algorithm is selected when multiple tokens are present.

## HTTP Middleware

```rust
#[async_trait]
pub trait HttpMiddleware: Send + Sync {
    async fn on_request(&self, req: reqwest::Request) -> Result<reqwest::Request>;
    async fn on_response(&self, resp: reqwest::Response) -> Result<reqwest::Response>;
}
```

Middleware is composable — `AnmlClient::builder().middleware(auth_mw).middleware(logging_mw)` applies them in order (request: first→last, response: last→first). This covers auth injection, request signing, metrics, and custom header rewriting without baking any of those into the core client.

## Retry & Circuit Breaker

```rust
pub struct RetryPolicy {
    pub max_retries: u32,       // default 3
    pub base_delay: Duration,   // default 1s
    pub multiplier: f64,        // default 2.0
    pub max_delay: Duration,    // default 30s
    pub retryable_statuses: HashSet<StatusCode>,  // default: 500, 502, 503, 504
}

pub struct CircuitBreaker {
    pub failure_threshold: u32,  // default 5
    pub cooldown: Duration,      // default 60s
    // per-origin state tracked internally
}
```

The retry policy handles transient HTTP failures. The circuit breaker is per-origin and prevents hammering a failing service. Both are independent of ANML-level `retry-budget` (which governs flow step retries).

## Authentication Provider

```rust
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Called before each request to an origin that requires auth.
    /// Returns headers to attach (e.g., Authorization: Bearer ...).
    async fn credentials(&self, origin: &Origin) -> Option<Vec<(String, String)>>;

    /// Called when a 401 is received. Implementations can refresh tokens.
    async fn on_unauthorized(&self, origin: &Origin) -> AuthRefreshResult;
}

pub enum AuthRefreshResult {
    Refreshed,   // retry the request with new credentials
    Failed,      // propagate the 401 as an error
}
```

The client calls `credentials()` before dispatching any request to an origin where `<action auth="required|optional">`. On 401, it calls `on_unauthorized()` and retries once if `Refreshed`.

## Fluent Action Builder

```rust
// Typestate pattern — required params tracked at compile time
let response = client.action(&doc, "submit-reason")
    .param("reason", "damaged")       // required param — must be set
    .param("notes", "Box was crushed") // optional param
    .execute()
    .await?;

// This won't compile if "reason" (required) is missing:
// client.action(&doc, "submit-reason").execute().await?;  // ERROR
```

The typestate is implemented via a generic `ActionRequestBuilder<State>` where `State` is a tuple of marker types tracking which required params have been set. When all required params are present, `.execute()` becomes available. This gives compile-time enforcement without runtime overhead.

Param setters accept `impl Into<String>` for strings, `i64`/`f64`/`u64` for numbers, and `bool` for booleans — automatic canonical form conversion happens internally.

## Prelude

```rust
pub mod prelude {
    pub use crate::{
        AnmlClient, ClientConfig, AnmlClientError, Result,
        ConsentBasis, TrustPolicy, AllowListTrustPolicy,
        ActionRequestBuilder, FlowNavigator,
    };
    // Re-exports from anml crate
    pub use anml::types::document::AnmlDocument;
    pub use anml::types::elements::{
        AnmlAction, AnmlAsk, AnmlAnswer, AnmlRefuse, AnmlInform,
    };
}
```

## Timeout Architecture

```rust
pub struct TimeoutConfig {
    pub per_request: Duration,      // default 30s — single HTTP call
    pub per_action: Duration,       // default 60s — including retries
    pub per_flow: Duration,         // default 5min — entire multi-step flow
    pub per_media_fetch: Duration,  // default 15s — single resource fetch
    pub parse: Duration,            // default 5s — XML parsing
}
```

Each timeout is enforced independently via `tokio::time::timeout`. The per-flow timeout wraps the entire `FlowNavigator::run()` loop — if the flow exceeds it, all in-progress steps are cancelled and a `FlowAborted` error is returned regardless of individual step success.

## Client Sharing

`AnmlClient` is `Clone + Send + Sync`. Internally it wraps all mutable state in `Arc<...>`:

```rust
pub struct AnmlClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,           // connection-pooled
    config: ClientConfig,
    consent_store: RwLock<ConsentStore>,
    audit_log: Box<dyn AuditLog>,
    trust_policy: Box<dyn TrustPolicy>,
    // ...
}
```

Users create one client and share it via clone (cheap Arc bump). Documentation will explicitly warn against creating per-request clients.

## Display Implementations

```rust
// AnmlDocument Display — concise summary for logs
// Output: "ANML[Travel Booking Service] v1.0 | 2 asks, 1 action, flow@search (3 steps) | 200 OK"
impl fmt::Display for AnmlDocument { ... }

// Error Display — actionable single-line messages
// Output: "disclosure denied: field 'airline' requires explicit consent (consent-scope=session), but no consent was granted"
// Output: "param validation: 'reason' expected one of [damaged, wrong-item, not-needed], got 'broken'"
impl fmt::Display for AnmlClientError { ... }
```

## Testing Module (feature-gated)

```rust
// In tests:
let server = MockAnmlServer::new()
    .document("/service", fixtures::travel_booking())
    .action_response("submit-airline", fixtures::success_status())
    .build()
    .await;

let client = AnmlClient::builder()
    .base_url(&server.url())
    .trust_policy(AllowAllTrustPolicy)  // for testing only
    .build()?;

let doc = client.fetch("/service").await?;
assert_eq!(doc.title(), Some("Travel Booking Service"));

server.assert_received("submit-airline", |req| {
    req.has_param("airline", "Delta");
    req.has_consent_header();
});
```

`MockAnmlServer` wraps `axum` or `hyper` in-process, serves ANML documents, records requests, and provides assertion helpers. Fixture documents cover: simple service, multi-step flow, disclosure-gated asks, paginated data, error responses, extension-required documents.

## State Serialization

`ConsentStore`, `InMemoryAuditLog`, and `FlowNavigator` all derive `Serialize`/`Deserialize` behind the `serde` feature flag. This lets users persist state:

```rust
// Save
let state = client.export_state()?;
let json = serde_json::to_string(&state)?;
fs::write("client_state.json", json)?;

// Restore
let json = fs::read_to_string("client_state.json")?;
let state: ClientState = serde_json::from_str(&json)?;
let client = AnmlClient::builder()
    .restore_state(state)
    .build()?;
```

## Security Architecture

All security requirements are derived from RFC Section 11 and implemented as pure Rust — no external binaries or services.

### Transport layer
- `reqwest` with `rustls-tls` feature (pure Rust TLS, no system OpenSSL).
- Default: HTTPS only. `ClientConfig::allow_plaintext_http(true)` to opt in to HTTP.
- Documents fetched over HTTP that contain `<constraints>`, `<interact>`, or `<ask requires="explicit">` are rejected with `TransportInsecure` error.

### Content-Type validation
- Before passing any response body to the parser, the client checks `Content-Type` starts with `application/anml+xml`. Mismatch → `ContentTypeMismatch` error. No fallback parsing.

### Resource limits
- `ResourceLimits` struct with RFC defaults (1 MiB doc size, 64 depth, 10k elements, etc.), passed to the `anml` parser.
- Decompression-aware: the client wraps the response body in a counting reader that aborts at the limit, regardless of `Content-Length`.
- Parse timeout enforced via `tokio::time::timeout`.

### Trust policy

```rust
pub trait TrustPolicy: Send + Sync {
    fn evaluate(&self, origin: &Origin, doc: &AnmlDocument) -> TrustDecision;
}

pub enum TrustDecision {
    Allow,
    Deny { reason: String },
}
```

Default implementation: `DenyAllTrustPolicy` (deny unless origin is in an explicit allow-list). Users must provide their own or use `AllowListTrustPolicy`.

### Action safety
- `ActionBudget` struct: max distinct origins per document (default 5), max total requests per document (default 50), max media fetches (default 20), max media bandwidth (default 50 MiB).
- Endpoint URI validation: resolved against the document origin; private/loopback IPs rejected by default (SSRF protection).
- `confirm="true"` actions always invoke the confirm callback; no silent execution.

### Tokenization

```rust
pub struct Tokenizer {
    secret: [u8; 32],  // Per-client, generated at construction via OsRng
}

impl Tokenizer {
    pub fn tokenize(&self, field: &str, principal_id: &str, origin: &Origin) -> String {
        // HMAC-SHA256(secret, field || principal_id || origin)
        // Output: hex-encoded, not URL-safe, not reversible
    }
}
```

### Audit log

```rust
pub trait AuditLog: Send + Sync {
    fn record(&self, entry: AuditEntry);
}

pub struct AuditEntry {
    pub field: String,
    pub origin: Origin,
    pub timestamp: SystemTime,
    pub consent_basis: ConsentBasis,
    pub disclosure_rule: String,  // field attr of the governing <disclosure>
    pub action_id: Option<String>,
}
```

Default: `InMemoryAuditLog` (append-only `Vec` behind a `Mutex`). Users can provide custom implementations (file-backed, database, etc.).

### Cross-origin media isolation
- Media fetches use a separate `reqwest::Client` instance with no cookie jar, no default auth headers, and no credential forwarding.
- Configurable per-document media budget (count + bandwidth).

### Extension namespace safety
- On parse, the client scans `<meta name="requires-ext">` entries. If any declared extension URI is not in the client's recognized set, the document is refused with `UnsupportedExtension` error.

## Pure Rust Constraint

Every dependency MUST be a Rust crate. No external binaries, no system calls, no shelling out.

| Concern | Crate | Why pure Rust |
|---------|-------|---------------|
| TLS | `rustls` (via `reqwest`) | No system OpenSSL |
| DNS-SD | `hickory-resolver` | No system `dig`/`nslookup` |
| HTML parsing | `scraper` / `html5ever` | No external parser |
| Crypto (SRI) | `sha2` | No system `openssl` |
| Crypto (tokens) | `hmac`, `sha2` | No system `openssl` |
| UUID | `uuid` | Pure Rust |
| Base64 | `base64` | Pure Rust |

## Dependency Summary

| Crate | Purpose |
|-------|---------|
| `anml` | Types, parsing, serialization, building, validation |
| `reqwest` (with `rustls-tls`) | HTTP client (async, pure Rust TLS) |
| `tokio` | Async runtime |
| `thiserror` | Error types |
| `url` | URL parsing and manipulation |
| `sha2` | SHA-256/384/512 for SRI |
| `hmac` | HMAC for tokenization |
| `base64` | Base64 encoding for SRI |
| `uuid` | Idempotency-Key generation (v4) |
| `rand` | Cryptographic randomness for tokenizer secret |
| `tracing` | Structured logging / observability |
| `async-trait` | Async trait support for middleware, auth, trust policy |
| `serde` / `serde_json` | State serialization (feature-gated) |
| `hickory-resolver` | DNS-SD (optional, feature-gated) |
| `scraper` | HTML link parsing (optional, feature-gated) |
| `moka` | TTL cache (optional, feature-gated) |
| `axum` + `tokio-test` | Mock server for testing module (optional, feature-gated) |

## Test Strategy

### Property-Based Tests (`proptest`)
The server crate uses `proptest` extensively — we follow the same pattern. PBT targets:

- **Disclosure matching** — generate arbitrary `Vec<Disclosure>` rule sets and field names, verify the matching algorithm always selects the correct rule per RFC precedence (exact > prefix > pattern > default), and that no field ever matches two rules at the same precedence level.
- **Parameter binding** — generate arbitrary `Vec<Param>` with random types/values, serialize to all three enctypes, verify the output is well-formed and deterministic (same input → same bytes, critical for idempotency key stability).
- **Rate limit tracking** — generate arbitrary sequences of `(field, origin, timestamp)` disclosure events, verify the tracker correctly enforces the 24-hour sliding window.
- **Consent store** — generate arbitrary grant/check/revoke sequences across session/origin/global scopes, verify state consistency.
- **Glob pattern matching** — generate arbitrary `field-pattern` values and field names, verify against a naive reference implementation.

### Integration Tests (`MockAnmlServer`)
Full lifecycle tests using the in-process mock server:
- Happy path: discover → fetch → inspect → answer → execute → parse response
- Multi-step flow: advance through 3+ steps, verify state transitions
- Disclosure gate: service asks with `requires="explicit"`, verify consent callback is invoked
- Error paths: 406, 401, integrity mismatch, SSRF block, trust denial, rate limit
- Pagination: iterate through multiple pages of `<data>`

### Compile-Fail Tests (`trybuild`)
- Missing required param on `ActionRequestBuilder` → compile error
- Ensures typestate enforcement works as designed

### Dev Dependencies

```toml
[dev-dependencies]
proptest = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "test-util"] }
trybuild = "1"
```
