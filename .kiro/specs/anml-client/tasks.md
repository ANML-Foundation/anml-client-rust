# ANML Client for Rust — Implementation Tasks

## Phase 1: Foundation

### Task 1: Project scaffolding
- [x] Initialize `Cargo.toml` with package metadata, edition 2021, MSRV 1.80, ISC license
- [x] Add `anml` as git dependency: `anml = { git = "https://github.com/Life-Savor-AI/anml-server-rust.git", features = ["serde"] }`
- [x] Add core deps: `reqwest` (rustls-tls), `tokio`, `thiserror`, `url`, `sha2`, `hmac`, `base64`, `uuid`, `rand`, `tracing`, `async-trait`, `serde`, `serde_json`
- [x] Add dev-deps: `proptest`, `tokio` (test-util), `trybuild`, `axum`
- [x] Define feature flags: `dns-sd`, `html-discovery`, `cache`, `blocking`, `testing`, `serde` (default), `wasm` (reserved empty)
- [x] Create `src/lib.rs` with module declarations
- [x] Create `.gitignore`, skeleton `README.md`, `CHANGELOG.md`, `LICENSE`

### Task 2: Error types and Display (`src/error.rs`)
- [x] Define `AnmlClientError` enum with all 19 variants, each with contextual fields
- [x] Implement `Display` with actionable "expected X, got Y" / "field 'Z' requires W" phrasing
- [x] Implement `From<reqwest::Error>` and `From<anml::errors::AnmlError>`
- [x] Map RFC problem type URIs to error variants via `problem_types` module
- [x] Define `Result<T>` type alias
- [x] `#[non_exhaustive]` on the enum
- [x] 28 tests verifying error message format, From impls, Send+Sync, single-line enforcement

### Task 3: Configuration, traits, and timeouts (`src/config.rs`)
- [ ] Define `ClientConfig` struct with base_url, `TimeoutConfig`, TLS settings, default headers, `allow_plaintext_http` flag, `ResourceLimits`, `ActionBudget`
- [ ] Define `TimeoutConfig`: per_request (30s), per_action (60s), per_flow (5min), per_media_fetch (15s), parse (5s)
- [ ] Define `ResourceLimits` with all RFC defaults (1 MiB doc, 64 depth, 10k elements, 64 KiB attr, 256 attrs/element, 256 KiB text/element, 64:1 entity expansion, 5s parse, 1k disclosures, 1k knowledge primitives, 256 steps) — configurable but not disableable (minimum floor values)
- [ ] Define `ActionBudget`: max distinct origins (5), max requests (50), max media fetches (20), max media bandwidth (50 MiB)
- [ ] Implement `ClientConfigBuilder` with builder pattern
- [ ] Define `ConsentHandler` trait for principal consent prompts (sync callback, blocks until answered)
- [ ] Define `ConfirmHandler` trait for action confirmation prompts
- [ ] Define `ConditionEvaluator` trait for `<step condition="...">` evaluation
- [ ] Define `TrustPolicy` trait with `evaluate(origin, doc) -> TrustDecision`
- [ ] Implement `DenyAllTrustPolicy` (default) and `AllowListTrustPolicy`
- [ ] Define `AuthProvider` async trait with `credentials(origin)` and `on_unauthorized(origin) -> AuthRefreshResult`
- [ ] Define `HttpMiddleware` async trait with `on_request` and `on_response`
- [ ] `#[non_exhaustive]` on all config structs and enums

### Task 4: Core `AnmlClient` struct (`src/client.rs`)
- [ ] Define `AnmlClient` with Arc-wrapped internals (`Clone + Send + Sync`)
- [ ] Hold: `reqwest::Client`, `ClientConfig`, `ConsentStore`, `AuditLog`, `TrustPolicy`, `AuthProvider`, middleware chain, `CircuitBreaker`, `RetryPolicy`, `Tokenizer`, `ConditionEvaluator`
- [ ] Implement `AnmlClient::builder()` returning `ClientConfigBuilder`
- [ ] Configure `reqwest` with `rustls-tls` (no native TLS)
- [ ] Create isolated `reqwest::Client` for media fetches (no cookies, no auth)
- [ ] Implement `fetch(path) -> Result<AnmlDocument>` — Accept header with version, Content-Type validation, version negotiation, resource limits, parse timeout
- [ ] Implement `fetch_url(url) -> Result<AnmlDocument>` — absolute URL variant
- [ ] Reject documents with `<constraints>`/`<interact>`/`<ask requires="explicit">` when fetched over HTTP
- [ ] Handle HTTP 406 with structured problem parsing
- [ ] Scan `<meta name="requires-ext">` on parse; refuse if unrecognized extension declared
- [ ] Scan `<meta name="profile">` on parse; refuse if unsupported profile declared (client implements `core-1.0`)
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
- [ ] `discover(origin)` orchestrator that tries mechanisms in order: well-known → Link header → HTML link → DNS-SD
- [ ] Add `discover()` method to `AnmlClient`

### Task 7: Disclosure evaluation engine (`src/disclosure/`)
- [ ] `matching.rs` — exact field, field-prefix, field-pattern, default matching with RFC precedence (exact > longest prefix > fewest metacharacters pattern > default; ties by document order)
- [ ] `field-prefix` matching: S matches field names starting with S followed by `.` or exactly S; `contact` matches `contact`, `contact.email`, NOT `contacts`
- [ ] Glob pattern matching for `field-pattern`: `*` (0+ chars except `.`), `**` (0+ chars including `.`), `?` (exactly 1 char except `.`); backslash escapes
- [ ] Mutual exclusion validation: `<disclosure>` MUST NOT carry both `field` and `field-prefix`, nor `field` and `field-pattern`, nor `field-prefix` and `field-pattern`
- [ ] `consent.rs` — `ConsentStore` with session/origin/global scoped grants; inspect, revoke, list
- [ ] `rate_limit.rs` — per-field 24-hour sliding window rate limit tracker for (field, origin) pairs
- [ ] `evaluate(doc, field, value) -> DisclosureDecision` — full 7-step RFC disclosure algorithm:
  - Step 1: Resolve rule via matching precedence; no match → synthesize `requires="explicit"` + `consent-scope="session"`, log warning
  - Step 2: Check rate limit for (field, origin) in 24h window; violation → refuse `rate-limited`
  - Step 3: Check consent via `ConsentHandler`; `explicit` → always callback; `implicit` → check store; `authentication` → verify via `AuthProvider`; `none` → skip; failure → refuse `user-denied`
  - Step 4: Check trust policy via `TrustPolicy::evaluate()`; denial → refuse `trust-insufficient`
  - Step 5: Validate value against `<ask>` type/pattern/one-of; failure → refuse `unsupported-field`
  - Step 6: Tokenize if `tokenize="true"` via `Tokenizer`
  - Step 7: Emit answer, record audit entry (field, origin, timestamp, consent basis, rule reference)

### Task 8: Tokenization (`src/security/tokenizer.rs`)
- [ ] `Tokenizer` struct with per-client HMAC-SHA256 secret (generated via `OsRng`)
- [ ] `tokenize(field, principal_id, origin) -> String` — hex-encoded, non-reversible, non-URL-safe, bound to (field, principal, origin) tuple
- [ ] Integrate into disclosure algorithm step 6 (when `tokenize="true"`)

### Task 9: Audit logging (`src/audit.rs`)
- [ ] Define `AuditLog` trait with `record(AuditEntry)` method
- [ ] `AuditEntry` struct: field, origin, timestamp, consent_basis, disclosure_rule, action_id
- [ ] `InMemoryAuditLog` — append-only Vec behind Mutex
- [ ] Expose to principal: list entries, filter by origin/field
- [ ] `Serialize`/`Deserialize` behind `serde` feature

### Task 10: Action execution (`src/action/`)
- [ ] `params.rs` — parameter binding algorithm for all 3 enctypes with canonical forms:
  - urlencoded: space→`+`, unreserved `ALPHA/DIGIT/-._~`, `name=value` pairs joined by `&` in document order, typed values as XSD canonical forms
  - multipart: boundary = `anml-` + doc `@id` + 96-bit crypto random (base32-no-pad), parts in document order
  - JSON: keys in document order (MUST NOT sort), numbers as JSON numbers, booleans as JSON booleans, dateTime as RFC 3339, minified UTF-8 no BOM
- [ ] `validation.rs` — validate param values against type, required, pattern, min, max, enum; errors reference param name + constraint + expected + actual
- [ ] `idempotency.rs` — UUID v4 key generation (≥128 bits entropy), retry with same key on network failure, new key on app error unless `retry-after`/`transient` problem type
- [ ] Endpoint URI resolution: relative URIs resolve against document origin (scheme+host+port); `xml:base` on ancestor takes precedence per XML Base; absolute URIs pass through but still checked
- [ ] SSRF protection: reject endpoints resolving to private/loopback IPs (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1, fc00::/7)
- [ ] Track per-document request count and origin set against `ActionBudget`
- [ ] Run disclosure evaluation before sending answers
- [ ] Invoke confirm callback when `confirm="true"`
- [ ] Integrate auth provider for `auth="required|optional"` actions; on 401 call `on_unauthorized()` and retry once
- [ ] Wrap in per-action timeout
- [ ] Parse response as ANML document

### Task 11: Fluent action builder with typestate (`src/action/builder.rs`)
- [ ] `ActionRequestBuilder<State>` with generic typestate tracking required params
- [ ] `.param(name, value)` accepting `impl Into<String>`, `i64`, `f64`, `u64`, `bool` with automatic canonical form conversion
- [ ] `.with_consent(basis)` accepting enum or `&str` (with validation)
- [ ] `.execute()` only available when all required param markers satisfied
- [ ] Validation at execute time with actionable error messages referencing param name + constraint
- [ ] NOT usable for deferred asks (asks without `action` attribute)

### Task 12: Knowledge exchange (`src/knowledge/`)
- [ ] Convenience wrappers around `anml::builder::ResponseBuilder`
- [ ] `build_answer(field, value, consent_basis)` with automatic disclosure check (runs full 7-step algorithm)
- [ ] `build_answer()` works for deferred asks too (no HTTP submission, but disclosure still evaluated)
- [ ] `build_refuse(field, reason)` with optional constraint reference and message
- [ ] `build_ask(field, action_id, purpose)` and `build_inform(text)`
- [ ] `build_response(...)` composite builder

### Task 13: Flow navigation (`src/flow/`)
- [ ] `FlowNavigator` struct tracking current step, history, document state
- [ ] `current()`, `pending()`, `completed()`, `is_complete()` accessors
- [ ] `advance(params) -> Result<AnmlDocument>` — execute step action, update state from response
- [ ] `next-on-error` transitions and `retry-budget` with exponential backoff (1s base, 2x multiplier, cap 60s or remaining TTL)
- [ ] Budget exhaustion → follow `next-on-error` or abort with `FlowAborted`
- [ ] Detect state regressions (step moving backward); surface as `UnexpectedStateRegression` warning
- [ ] `ConditionEvaluator` integration: call before transitioning to a step with `condition`; if evaluator returns false, skip step; if no evaluator configured, treat as available with warning
- [ ] Expose `condition` as `Option<String>` on step accessors
- [ ] Wrap in per-flow timeout; exceeding aborts with `FlowAborted` regardless of step success
- [ ] `Display` impl: current step, progress summary
- [ ] `Serialize`/`Deserialize` behind `serde` feature

### Task 14: SRI verification (`src/integrity.rs`)
- [ ] `verify_integrity(bytes, integrity_attr) -> Result<()>` — sha256, sha384, sha512
- [ ] Select strongest algorithm when multiple tokens present
- [ ] `fetch_verified_resource(url, integrity) -> Result<Vec<u8>>` with per-media-fetch timeout
- [ ] Enforce: `inference="required"` MUST have integrity verified; missing integrity on `inference="required"` = `MalformedDocument` error
- [ ] Track media fetch count and bandwidth against `ActionBudget`
- [ ] Use isolated media `reqwest::Client` (no credentials, no cookies)
- [ ] Log security events on mismatch (element, expected digest, observed digest)

### Task 15: Pagination (`src/pagination.rs`)
- [ ] `PaginatedStream` wrapping `<nav>` next/prev/cursor
- [ ] `AsyncStream` yielding pages of `AnmlDocument`
- [ ] `client.paginate(doc, data_id)` entry point
- [ ] Handle end of pagination (no `next` in `<nav>`); expose `total` when available

### Task 16: Confidentiality and usage rights (`src/rights.rs`)
- [ ] Expose `confidentiality` level on `<inform>` via accessors
- [ ] Expose `<rights>` (holder, year, license, usage) and `<attribution>` (required, scope) via accessors
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
- [ ] `RetryPolicy` struct: max_retries (3), base_delay (1s), multiplier (2.0), max_delay (30s), retryable_statuses (500, 502, 503, 504)
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
- [ ] `fixtures.rs` — pre-built documents: simple service, multi-step flow, disclosure-gated, paginated, error/problem, extension-required, deferred ask (no action attr)
- [ ] `assertions.rs` — helpers: `assert_received(action_id, ...)`, `assert_disclosure_granted(field)`, `assert_param(name, value)`

### Task 25: Property-based tests — disclosure matching
- [ ] `proptest` Arbitrary impl for disclosure rule sets (field, field-prefix, field-pattern, default, requires, consent-scope)
- [ ] Properties: exact wins over prefix/pattern/default; longest prefix wins; fewest metacharacters wins; default only when no other match; missing rule synthesizes explicit/session; mutual exclusion (field+prefix, field+pattern, prefix+pattern never coexist)

### Task 26: Property-based tests — parameter binding
- [ ] `proptest` Arbitrary impl for param sets (name, value, type)
- [ ] Properties: urlencoded deterministic (same input → same bytes); JSON preserves document order (keys not sorted); multipart boundary has cryptographic randomness; required params with include=false omitted; typed values use canonical forms (XSD decimal, boolean, dateTime)

### Task 27: Property-based tests — consent store and rate limits
- [ ] Properties: session scope isolated per client instance; origin scope visible to same origin not siblings; global scope cross-origin; revoke removes exactly targeted grant; rate limit 24h sliding window correct; rate limit resets after 24h

### Task 28: Integration tests
- [ ] Happy path: discover → fetch → inspect → answer → execute → parse
- [ ] Multi-step flow: 3+ steps with state transitions and condition evaluation
- [ ] Disclosure gate: explicit consent triggers callback; deferred ask runs disclosure but no HTTP
- [ ] Errors: 406 version mismatch, integrity mismatch, plaintext rejection, SSRF block, trust denial, unsupported profile, unsupported extension
- [ ] Pagination: 3 pages, all items collected
- [ ] URI resolution: relative endpoint resolved against document origin; xml:base override

### Task 29: Compile-fail tests
- [ ] `trybuild` harness
- [ ] Missing required param → compile error
- [ ] All required params set → compiles
- [ ] Optional params omitted → compiles

## Phase 6: Documentation & Polish

### Task 30: Examples
- [ ] `examples/basic_fetch.rs` — discover, fetch, inspect
- [ ] `examples/action_execution.rs` — disclosure, execute, handle response
- [ ] `examples/multi_step_flow.rs` — flow with error recovery and condition evaluation
- [ ] `examples/custom_auth.rs` — AuthProvider for OAuth2 refresh
- [ ] `examples/middleware.rs` — request logging, custom headers

### Task 31: Documentation and project hygiene
- [ ] Crate-level rustdoc with overview and quick start
- [ ] Complete `README.md`: installation, features, usage, security model
- [ ] `CONTRIBUTING.md` with dev setup, testing, PR guidelines
- [ ] Verify `Cargo.toml` metadata: description, repository, keywords, categories, license, rust-version
