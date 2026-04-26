# ANML Client for Rust — Implementation Tasks

## Phase 1: Foundation

### Task 1: Project scaffolding
- [ ] Initialize `Cargo.toml` with package metadata, edition 2021, MSRV 1.80, ISC license
- [ ] Add `anml` as git dependency: `anml = { git = "https://github.com/Life-Savor-AI/anml-server-rust.git", features = ["serde"] }`
- [ ] Add core deps: `reqwest` (rustls-tls), `tokio`, `thiserror`, `url`, `sha2`, `hmac`, `base64`, `uuid`, `rand`, `tracing`, `async-trait`, `serde`, `serde_json`
- [ ] Add dev-deps: `proptest`, `tokio` (test-util), `trybuild`, `axum`
- [ ] Define feature flags: `dns-sd`, `html-discovery`, `cache`, `blocking`, `testing`, `serde` (default), `wasm` (reserved empty)
- [ ] Create `src/lib.rs` with module declarations
- [ ] Create `.gitignore`, skeleton `README.md`, `CHANGELOG.md`, `LICENSE`

### Task 2: Error types and Display (`src/error.rs`)
- [ ] Define `AnmlClientError` enum with all variants: Http, Parse, TransportInsecure, ContentTypeMismatch, UnsupportedVersion, UnsupportedProfile, UnsupportedExtension, MalformedDocument, FlowAborted, IntegrityMismatch, ResourceLimitExceeded, ConsentDenied, RateLimited, TrustInsufficient, ActionBudgetExceeded, SsrfBlocked, UnexpectedStateRegression, ParamValidation, Timeout
- [ ] Every variant includes contextual fields (field name, rule, expected/actual, action id, endpoint)
- [ ] Implement `Display` with actionable "expected X, got Y" / "field 'Z' requires W" phrasing
- [ ] Implement `From<reqwest::Error>` and `From<anml::errors::AnmlError>`
- [ ] Map RFC problem type URIs to error variants
- [ ] Define `Result<T>` type alias
- [ ] `#[non_exhaustive]` on the enum
- [ ] Write tests verifying error message format for each variant

### Task 3: Configuration, traits, and timeouts (`src/config.rs`)
- [ ] Define `ClientConfig` struct with base_url, `TimeoutConfig`, TLS settings, default headers, `allow_plaintext_http` flag, `ResourceLimits`, `ActionBudget`
- [ ] Define `TimeoutConfig`: per_request (30s), per_action (60s), per_flow (5min), per_media_fetch (15s), parse (5s)
- [ ] Define `ResourceLimits` with all RFC defaults (1 MiB doc, 64 depth, 10k elements, etc.) — configurable but not disableable
- [ ] Define `ActionBudget`: max distinct origins (5), max requests (50), max media fetches (20), max media bandwidth (50 MiB)
- [ ] Implement `ClientConfigBuilder` with builder pattern
- [ ] Define `ConsentHandler` trait for principal consent prompts
- [ ] Define `ConfirmHandler` trait for action confirmation prompts
- [ ] Define `TrustPolicy` trait with `evaluate(origin, doc) -> TrustDecision`
- [ ] Implement `DenyAllTrustPolicy` (default) and `AllowListTrustPolicy`
- [ ] Define `AuthProvider` async trait with `credentials(origin)` and `on_unauthorized(origin)`
- [ ] Define `HttpMiddleware` async trait with `on_request` and `on_response`
- [ ] `#[non_exhaustive]` on all config structs and enums

### Task 4: Core `AnmlClient` struct (`src/client.rs`)
- [ ] Define `AnmlClient` with Arc-wrapped internals (`Clone + Send + Sync`)
- [ ] Hold: `reqwest::Client`, `ClientConfig`, `ConsentStore`, `AuditLog`, `TrustPolicy`, `AuthProvider`, middleware chain, `CircuitBreaker`, `RetryPolicy`
- [ ] Implement `AnmlClient::builder()` returning `ClientConfigBuilder`
- [ ] Configure `reqwest` with `rustls-tls` (no native TLS)
- [ ] Create isolated `reqwest::Client` for media fetches (no cookies, no auth)
- [ ] Implement `fetch(path) -> Result<AnmlDocument>` — Accept header, Content-Type validation, version negotiation, resource limits, parse timeout
- [ ] Implement `fetch_url(url) -> Result<AnmlDocument>` — absolute URL variant
- [ ] Reject documents with `<constraints>`/`<interact>`/`<ask requires="explicit">` when fetched over HTTP
- [ ] Handle HTTP 406 with structured problem parsing
- [ ] Scan `<meta name="requires-ext">` on parse; refuse if unrecognized extension declared
- [ ] Wrap all HTTP calls in per-request timeout
- [ ] Apply middleware chain (request: first→last, response: last→first)
- [ ] Apply retry policy for transient HTTP errors (5xx, connection reset, timeout)
- [ ] Apply circuit breaker per origin
- [ ] Doc comments warning against per-request client creation

### Task 5: Prelude module (`src/prelude.rs`)
- [ ] Re-export core types: `AnmlClient`, `ClientConfig`, `AnmlClientError`, `Result`, `ConsentBasis`, `TrustPolicy`, `AllowListTrustPolicy`, `ActionRequestBuilder`, `FlowNavigator`
- [ ] Re-export key `anml` crate types: `AnmlDocument`, `AnmlAction`, `AnmlAsk`, `AnmlAnswer`, `AnmlRefuse`, `AnmlInform`
- [ ] Document in crate-level docs

## Phase 2: Protocol Implementation

### Task 6: Discovery (`src/discovery/`)
- [ ] Define `DiscoveryResult` struct (endpoint URL, version, metadata)
- [ ] `well_known.rs` — fetch `/.well-known/anml`, parse as ANML document
- [ ] `link_header.rs` — parse `Link` response headers for `rel="alternate" type="application/anml+xml"`
- [ ] `html_link.rs` (feature-gated `html-discovery`) — parse HTML for `<link rel="alternate">`
- [ ] `dns_sd.rs` (feature-gated `dns-sd`) — resolve `_anml._tcp` SRV + TXT records via `hickory-resolver`
- [ ] `discover(origin)` orchestrator that tries mechanisms in order
- [ ] Add `discover()` method to `AnmlClient`

### Task 7: Disclosure evaluation engine (`src/disclosure/`)
- [ ] `matching.rs` — exact field, field-prefix, field-pattern, default matching with RFC precedence
- [ ] Glob pattern matching for `field-pattern` (`*`, `**`, `?`)
- [ ] `consent.rs` — `ConsentStore` with session/origin/global scoped grants; inspect, revoke, list
- [ ] `rate_limit.rs` — per-field 24-hour sliding window rate limit tracker
- [ ] `evaluate(doc, field) -> DisclosureDecision` — full 7-step RFC disclosure algorithm
- [ ] Missing rules default to `requires="explicit"` + `consent-scope="session"`
- [ ] Integrate consent handler callback for explicit consent
- [ ] Integrate trust policy at step 4
- [ ] Integrate audit logging at step 7

### Task 8: Tokenization (`src/security/tokenizer.rs`)
- [ ] `Tokenizer` struct with per-client HMAC-SHA256 secret (generated via `OsRng`)
- [ ] `tokenize(field, principal_id, origin) -> String` — hex-encoded, non-reversible, non-URL-safe
- [ ] Integrate into disclosure algorithm step 6 (when `tokenize="true"`)

### Task 9: Audit logging (`src/audit.rs`)
- [ ] Define `AuditLog` trait with `record(AuditEntry)` method
- [ ] `AuditEntry` struct: field, origin, timestamp, consent_basis, disclosure_rule, action_id
- [ ] `InMemoryAuditLog` — append-only Vec behind Mutex
- [ ] Expose to principal: list entries, filter by origin/field
- [ ] `Serialize`/`Deserialize` behind `serde` feature

### Task 10: Action execution (`src/action/`)
- [ ] `params.rs` — parameter binding algorithm for urlencoded, multipart, and JSON enctypes
- [ ] `validation.rs` — validate param values against type, required, pattern, min, max, enum; errors reference param name + constraint
- [ ] `idempotency.rs` — UUID v4 key generation, retry with same key on network failure, new key on app error
- [ ] SSRF protection: reject endpoints resolving to private/loopback IPs
- [ ] Track per-document request count and origin set against `ActionBudget`
- [ ] Run disclosure evaluation before sending answers
- [ ] Invoke confirm callback when `confirm="true"`
- [ ] Integrate auth provider for `auth="required|optional"` actions; on 401 call `on_unauthorized()` and retry once
- [ ] Wrap in per-action timeout
- [ ] Parse response as ANML document

### Task 11: Fluent action builder with typestate (`src/action/builder.rs`)
- [ ] `ActionRequestBuilder<State>` with generic typestate tracking required params
- [ ] `.param(name, value)` accepting `impl Into<String>`, `i64`, `f64`, `u64`, `bool`
- [ ] `.with_consent(basis)` accepting enum or `&str`
- [ ] `.execute()` only available when all required param markers satisfied
- [ ] Validation at execute time with actionable error messages

### Task 12: Knowledge exchange (`src/knowledge/`)
- [ ] Convenience wrappers around `anml::builder::ResponseBuilder`
- [ ] `build_answer(field, value, consent_basis)` with automatic disclosure check
- [ ] `build_refuse(field, reason)` with optional constraint reference and message
- [ ] `build_ask(field, action_id, purpose)` and `build_inform(text)`
- [ ] `build_response(...)` composite builder

### Task 13: Flow navigation (`src/flow/`)
- [ ] `FlowNavigator` struct tracking current step, history, document state
- [ ] `current()`, `pending()`, `completed()`, `is_complete()` accessors
- [ ] `advance(params) -> Result<AnmlDocument>` — execute step action, update state from response
- [ ] `next-on-error` transitions and `retry-budget` with exponential backoff (1s base, 2x, cap 60s or TTL)
- [ ] Budget exhaustion → follow `next-on-error` or abort with `FlowAborted`
- [ ] Detect state regressions (step moving backward); surface as warnings
- [ ] Wrap in per-flow timeout
- [ ] `Display` impl: current step, progress summary
- [ ] `Serialize`/`Deserialize` behind `serde` feature

### Task 14: SRI verification (`src/integrity.rs`)
- [ ] `verify_integrity(bytes, integrity_attr) -> Result<()>` — sha256, sha384, sha512
- [ ] Select strongest algorithm when multiple tokens present
- [ ] `fetch_verified_resource(url, integrity) -> Result<Vec<u8>>` with per-media-fetch timeout
- [ ] Enforce: `inference="required"` MUST have integrity verified; missing integrity = malformed document
- [ ] Track media fetch count and bandwidth against `ActionBudget`
- [ ] Use isolated media `reqwest::Client` (no credentials)
- [ ] Log security events on mismatch

### Task 15: Pagination (`src/pagination.rs`)
- [ ] `PaginatedStream` wrapping `<nav>` next/prev/cursor
- [ ] `AsyncStream` yielding pages of `AnmlDocument`
- [ ] `client.paginate(doc, data_id)` entry point
- [ ] Handle end of pagination; expose `total` when available

### Task 16: Confidentiality and usage rights (`src/rights.rs`)
- [ ] Expose `confidentiality` level on `<inform>` via accessors
- [ ] Expose `<rights>` and `<attribution>` via accessors
- [ ] `usage_permitted(level)` helper checking against hierarchy (none < display < cache < store < train)

### Task 17: TTL cache (`src/cache.rs`, feature-gated `cache`)
- [ ] In-memory cache keyed by URL, respecting `ttl` from `<anml>` root and `<inform>`
- [ ] Cache invalidation on TTL expiry
- [ ] `fetch()` checks cache first; cache bypass option

## Phase 3: Observability & Resilience

### Task 18: Tracing integration
- [ ] Instrument `fetch()`, `execute_action()`, disclosure evaluation, flow transitions with `tracing` spans
- [ ] Spans include: origin, URL, action_id, field, method, endpoint
- [ ] Security events at WARN/ERROR: trust denials, integrity mismatches, SSRF blocks, transport rejections
- [ ] Sensitive values (answer values, tokens, credentials) NOT logged above TRACE

### Task 19: HTTP middleware
- [ ] Middleware chain in `AnmlClient` (request: first→last, response: last→first)
- [ ] `AnmlClient::builder().middleware(mw)` registration
- [ ] `LoggingMiddleware` example implementation

### Task 20: Retry policy and circuit breaker (`src/retry.rs`)
- [ ] `RetryPolicy` struct: max_retries (3), base_delay (1s), multiplier (2.0), max_delay (30s), retryable_statuses
- [ ] Exponential backoff retry loop wrapping HTTP dispatch
- [ ] `CircuitBreaker`: failure_threshold (5), cooldown (60s), per-origin state
- [ ] Short-circuit during cooldown; reset on success
- [ ] Independent of ANML-level `retry-budget`

### Task 21: Authentication providers (`src/auth.rs`)
- [ ] `BearerTokenProvider` (static token)
- [ ] `ApiKeyProvider` (header-based or query-based)
- [ ] Integration into action execution pipeline

## Phase 4: State & Serialization

### Task 22: State persistence
- [ ] `Serialize`/`Deserialize` on `ConsentStore`, `ConsentGrant`, `ConsentKey` (behind `serde`)
- [ ] `ClientState` struct aggregating consent store, audit log, flow state
- [ ] `client.export_state()` and `builder.restore_state(state)`
- [ ] Round-trip serialization tests

### Task 23: Display implementations
- [ ] `Display` for `AnmlDocument` — concise summary: title, version, ask/action counts, flow step, status
- [ ] `Debug` for all public types
- [ ] `Display` for `FlowNavigator` — current step, progress
- [ ] Tests verifying output format

## Phase 5: Testing

### Task 24: Testing module (feature-gated `testing`)
- [ ] `MockAnmlServer` using `axum` — configurable document responses, action recording
- [ ] `fixtures.rs` — pre-built documents: simple service, multi-step flow, disclosure-gated, paginated, error/problem, extension-required
- [ ] `assertions.rs` — helpers: `assert_received(action_id, ...)`, `assert_disclosure_granted(field)`, `assert_param(name, value)`

### Task 25: Property-based tests — disclosure matching
- [ ] `proptest` Arbitrary impl for disclosure rule sets
- [ ] Properties: exact wins over prefix/pattern/default; longest prefix wins; fewest metacharacters wins; default only when no other match; missing rule synthesizes explicit/session

### Task 26: Property-based tests — parameter binding
- [ ] `proptest` Arbitrary impl for param sets
- [ ] Properties: urlencoded deterministic; JSON preserves order; multipart boundary has randomness; required params with include=false omitted; typed values use canonical forms

### Task 27: Property-based tests — consent store and rate limits
- [ ] Properties: session scope isolated; origin scope not cross-origin; global scope cross-origin; revoke precise; rate limit 24h window correct

### Task 28: Integration tests
- [ ] Happy path: discover → fetch → inspect → answer → execute → parse
- [ ] Multi-step flow: 3+ steps with state transitions
- [ ] Disclosure gate: explicit consent triggers callback
- [ ] Errors: 406, integrity mismatch, plaintext rejection, SSRF block, trust denial
- [ ] Pagination: 3 pages, all items collected

### Task 29: Compile-fail tests
- [ ] `trybuild` harness
- [ ] Missing required param → compile error
- [ ] All required params set → compiles
- [ ] Optional params omitted → compiles

## Phase 6: Documentation & Polish

### Task 30: Examples
- [ ] `examples/basic_fetch.rs` — discover, fetch, inspect
- [ ] `examples/action_execution.rs` — disclosure, execute, handle response
- [ ] `examples/multi_step_flow.rs` — flow with error recovery
- [ ] `examples/custom_auth.rs` — AuthProvider for OAuth2 refresh
- [ ] `examples/middleware.rs` — request logging, custom headers

### Task 31: Documentation and project hygiene
- [ ] Crate-level rustdoc with overview and quick start
- [ ] Complete `README.md`: installation, features, usage, security model
- [ ] `CONTRIBUTING.md` with dev setup, testing, PR guidelines
- [ ] Verify `Cargo.toml` metadata: description, repository, keywords, categories, license, rust-version
