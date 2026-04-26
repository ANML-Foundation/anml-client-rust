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
- [x] Define `ClientConfig` struct with base_url, `TimeoutConfig`, TLS settings, default headers, `allow_plaintext_http` flag, `ResourceLimits`, `ActionBudget`
- [x] Define `TimeoutConfig`: per_request (30s), per_action (60s), per_flow (5min), per_media_fetch (15s), parse (5s)
- [x] Define `ResourceLimits` with all RFC defaults (1 MiB doc, 64 depth, 10k elements, 64 KiB attr, 256 attrs/element, 256 KiB text/element, 64:1 entity expansion, 5s parse, 1k disclosures, 1k knowledge primitives, 256 steps) — configurable but not disableable (minimum floor values)
- [x] Define `ActionBudget`: max distinct origins (5), max requests (50), max media fetches (20), max media bandwidth (50 MiB)
- [x] Implement `ClientConfigBuilder` with builder pattern
- [x] Define `ConsentHandler` trait for principal consent prompts (sync callback, blocks until answered)
- [x] Define `ConfirmHandler` trait for action confirmation prompts
- [x] Define `ConditionEvaluator` trait for `<step condition="...">` evaluation
- [x] Define `TrustPolicy` trait with `evaluate(origin, doc) -> TrustDecision`
- [x] Implement `DenyAllTrustPolicy` (default) and `AllowListTrustPolicy`
- [x] Define `AuthProvider` async trait with `credentials(origin)` and `on_unauthorized(origin) -> AuthRefreshResult`
- [x] Define `HttpMiddleware` async trait with `on_request` and `on_response`
- [x] `#[non_exhaustive]` on all config structs and enums

### Task 4: Core `AnmlClient` struct (`src/client.rs`)
- [x] Define `AnmlClient` with Arc-wrapped internals (`Clone + Send + Sync`)
- [x] Hold: `reqwest::Client`, `ClientConfig`, `ConsentStore`, `AuditLog`, `TrustPolicy`, `AuthProvider`, middleware chain, `CircuitBreaker`, `RetryPolicy`, `Tokenizer`, `ConditionEvaluator`
- [x] Implement `AnmlClient::builder()` returning `ClientConfigBuilder`
- [x] Configure `reqwest` with `rustls-tls` (no native TLS)
- [x] Create isolated `reqwest::Client` for media fetches (no cookies, no auth)
- [x] Implement `fetch(path) -> Result<AnmlDocument>` — Accept header with version, Content-Type validation, version negotiation, resource limits, parse timeout
- [x] Implement `fetch_url(url) -> Result<AnmlDocument>` — absolute URL variant
- [x] Reject documents with `<constraints>`/`<interact>`/`<ask requires="explicit">` when fetched over HTTP
- [x] Handle HTTP 406 with structured problem parsing
- [x] Scan `<meta name="requires-ext">` on parse; refuse if unrecognized extension declared
- [x] Scan `<meta name="profile">` on parse; refuse if unsupported profile declared (client implements `core-1.0`)
- [x] Wrap all HTTP calls in per-request timeout
- [x] Apply middleware chain (request: first→last, response: last→first)
- [x] Apply retry policy for transient HTTP errors (5xx, connection reset, timeout)
- [x] Apply circuit breaker per origin
- [x] Doc comments warning against per-request client creation

### Task 5: Prelude module (`src/prelude.rs`)
- [x] Re-export core types: `AnmlClient`, `ClientConfig`, `AnmlClientError`, `Result`, `ConsentBasis`, `TrustPolicy`, `AllowListTrustPolicy`, `ActionRequestBuilder`, `FlowNavigator`
- [x] Re-export key `anml` crate types: `AnmlDocument`, `AnmlAction`, `AnmlAsk`, `AnmlAnswer`, `AnmlRefuse`, `AnmlInform`
- [x] Document in crate-level docs

## Phase 2: Protocol Implementation

### Task 6: Discovery (`src/discovery/`)
- [x] Define `DiscoveryResult` struct (endpoint URL, version, metadata)
- [x] `well_known.rs` — fetch `/.well-known/anml`, parse as ANML document
- [x] `link_header.rs` — parse `Link` response headers for `rel="alternate" type="application/anml+xml"`
- [x] `html_link.rs` (feature-gated `html-discovery`) — parse HTML for `<link rel="alternate">`
- [x] `dns_sd.rs` (feature-gated `dns-sd`) — resolve `_anml._tcp` SRV + TXT records via `hickory-resolver`
- [x] `discover(origin)` orchestrator that tries mechanisms in order: well-known → Link header → HTML link → DNS-SD
- [x] Add `discover()` method to `AnmlClient`

### Task 7: Disclosure evaluation engine (`src/disclosure/`)
- [x] `matching.rs` — exact field, field-prefix, field-pattern, default matching with RFC precedence (exact > longest prefix > fewest metacharacters pattern > default; ties by document order)
- [x] `field-prefix` matching: S matches field names starting with S followed by `.` or exactly S; `contact` matches `contact`, `contact.email`, NOT `contacts`
- [x] Glob pattern matching for `field-pattern`: `*` (0+ chars except `.`), `**` (0+ chars including `.`), `?` (exactly 1 char except `.`); backslash escapes
- [x] Mutual exclusion validation: `<disclosure>` MUST NOT carry both `field` and `field-prefix`, nor `field` and `field-pattern`, nor `field-prefix` and `field-pattern`
- [x] `consent.rs` — `ConsentStore` with session/origin/global scoped grants; inspect, revoke, list
- [x] `rate_limit.rs` — per-field 24-hour sliding window rate limit tracker for (field, origin) pairs
- [x] `evaluate(doc, field, value) -> DisclosureDecision` — full 7-step RFC disclosure algorithm:
  - Step 1: Resolve rule via matching precedence; no match → synthesize `requires="explicit"` + `consent-scope="session"`, log warning
  - Step 2: Check rate limit for (field, origin) in 24h window; violation → refuse `rate-limited`
  - Step 3: Check consent via `ConsentHandler`; `explicit` → always callback; `implicit` → check store; `authentication` → verify via `AuthProvider`; `none` → skip; failure → refuse `user-denied`
  - Step 4: Check trust policy via `TrustPolicy::evaluate()`; denial → refuse `trust-insufficient`
  - Step 5: Validate value against `<ask>` type/pattern/one-of; failure → refuse `unsupported-field`
  - Step 6: Tokenize if `tokenize="true"` via `Tokenizer`
  - Step 7: Emit answer, record audit entry (field, origin, timestamp, consent basis, rule reference)

### Task 8: Tokenization (`src/security/tokenizer.rs`)
- [x] `Tokenizer` struct with per-client HMAC-SHA256 secret (generated via `OsRng`)
- [x] `tokenize(field, principal_id, origin) -> String` — hex-encoded, non-reversible, non-URL-safe, bound to (field, principal, origin) tuple
- [x] Integrate into disclosure algorithm step 6 (when `tokenize="true"`)

### Task 9: Audit logging (`src/audit.rs`)
- [x] Define `AuditLog` trait with `record(AuditEntry)` method
- [x] `AuditEntry` struct: field, origin, timestamp, consent_basis, disclosure_rule, action_id
- [x] `InMemoryAuditLog` — append-only Vec behind Mutex
- [x] Expose to principal: list entries, filter by origin/field
- [x] `Serialize`/`Deserialize` behind `serde` feature

### Task 10: Action execution (`src/action/`)
- [x] `params.rs` — parameter binding algorithm for all 3 enctypes with canonical forms:
  - urlencoded: space→`+`, unreserved `ALPHA/DIGIT/-._~`, `name=value` pairs joined by `&` in document order, typed values as XSD canonical forms
  - multipart: boundary = `anml-` + doc `@id` + 96-bit crypto random (base32-no-pad), parts in document order
  - JSON: keys in document order (MUST NOT sort), numbers as JSON numbers, booleans as JSON booleans, dateTime as RFC 3339, minified UTF-8 no BOM
- [x] `validation.rs` — validate param values against type, required, pattern, min, max, enum; errors reference param name + constraint + expected + actual
- [x] `idempotency.rs` — UUID v4 key generation (≥128 bits entropy), retry with same key on network failure, new key on app error unless `retry-after`/`transient` problem type
- [x] Endpoint URI resolution: relative URIs resolve against document origin (scheme+host+port); `xml:base` on ancestor takes precedence per XML Base; absolute URIs pass through but still checked
- [x] SSRF protection: reject endpoints resolving to private/loopback IPs (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1, fc00::/7)
- [x] Track per-document request count and origin set against `ActionBudget`
- [x] Run disclosure evaluation before sending answers
- [x] Invoke confirm callback when `confirm="true"`
- [x] Integrate auth provider for `auth="required|optional"` actions; on 401 call `on_unauthorized()` and retry once
- [x] Wrap in per-action timeout
- [x] Parse response as ANML document

### Task 11: Fluent action builder with typestate (`src/action/builder.rs`)
- [x] `ActionRequestBuilder<State>` with generic typestate tracking required params
- [x] `.param(name, value)` accepting `impl Into<String>`, `i64`, `f64`, `u64`, `bool` with automatic canonical form conversion
- [x] `.with_consent(basis)` accepting enum or `&str` (with validation)
- [x] `.execute()` only available when all required param markers satisfied
- [x] Validation at execute time with actionable error messages referencing param name + constraint
- [x] NOT usable for deferred asks (asks without `action` attribute)

### Task 12: Knowledge exchange (`src/knowledge/`)
- [x] Convenience wrappers around `anml::builder::ResponseBuilder`
- [x] `build_answer(field, value, consent_basis)` with automatic disclosure check (runs full 7-step algorithm)
- [x] `build_answer()` works for deferred asks too (no HTTP submission, but disclosure still evaluated)
- [x] `build_refuse(field, reason)` with optional constraint reference and message
- [x] `build_ask(field, action_id, purpose)` and `build_inform(text)`
- [x] `build_response(...)` composite builder

### Task 13: Flow navigation (`src/flow/`)
- [x] `FlowNavigator` struct tracking current step, history, document state
- [x] `current()`, `pending()`, `completed()`, `is_complete()` accessors
- [x] `advance(params) -> Result<AnmlDocument>` — execute step action, update state from response
- [x] `next-on-error` transitions and `retry-budget` with exponential backoff (1s base, 2x multiplier, cap 60s or remaining TTL)
- [x] Budget exhaustion → follow `next-on-error` or abort with `FlowAborted`
- [x] Detect state regressions (step moving backward); surface as `UnexpectedStateRegression` warning
- [x] `ConditionEvaluator` integration: call before transitioning to a step with `condition`; if evaluator returns false, skip step; if no evaluator configured, treat as available with warning
- [x] Expose `condition` as `Option<String>` on step accessors
- [x] Wrap in per-flow timeout; exceeding aborts with `FlowAborted` regardless of step success
- [x] `Display` impl: current step, progress summary
- [ ] `Serialize`/`Deserialize` behind `serde` feature

### Task 14: SRI verification (`src/integrity.rs`)
- [x] `verify_integrity(bytes, integrity_attr) -> Result<()>` — sha256, sha384, sha512
- [x] Select strongest algorithm when multiple tokens present
- [x] `fetch_verified_resource(url, integrity) -> Result<Vec<u8>>` with per-media-fetch timeout
- [x] Enforce: `inference="required"` MUST have integrity verified; missing integrity on `inference="required"` = `MalformedDocument` error
- [x] Track media fetch count and bandwidth against `ActionBudget`
- [x] Use isolated media `reqwest::Client` (no credentials, no cookies)
- [x] Log security events on mismatch (element, expected digest, observed digest)

### Task 15: Pagination (`src/pagination.rs`)
- [x] `PaginatedStream` wrapping `<nav>` next/prev/cursor
- [x] `AsyncStream` yielding pages of `AnmlDocument`
- [x] `client.paginate(doc, data_id)` entry point
- [x] Handle end of pagination (no `next` in `<nav>`); expose `total` when available

### Task 16: Confidentiality and usage rights (`src/rights.rs`)
- [x] Expose `confidentiality` level on `<inform>` via accessors
- [x] Expose `<rights>` (holder, year, license, usage) and `<attribution>` (required, scope) via accessors
- [x] `usage_permitted(level)` helper checking against hierarchy (none < display < cache < store < train)

### Task 17: TTL cache (`src/cache.rs`, feature-gated `cache`)
- [x] In-memory cache keyed by URL, respecting `ttl` from `<anml>` root and `<inform>`
- [x] Cache invalidation on TTL expiry
- [x] `fetch()` checks cache first; cache bypass option

## Phase 3: Observability & Resilience

### Task 18: Tracing integration
- [x] Instrument `fetch()`, `execute_action()`, disclosure evaluation, flow transitions with `tracing` spans
- [x] Spans include: origin, URL, action_id, field, method, endpoint
- [x] Security events at WARN/ERROR: trust denials, integrity mismatches, SSRF blocks, transport rejections
- [x] Sensitive values (answer values, tokens, credentials) NOT logged above TRACE

### Task 19: HTTP middleware
- [x] Middleware chain in `AnmlClient` (request: first→last, response: last→first)
- [x] `AnmlClient::builder().middleware(mw)` registration
- [x] `LoggingMiddleware` example implementation

### Task 20: Retry policy and circuit breaker (`src/retry.rs`)
- [x] `RetryPolicy` struct: max_retries (3), base_delay (1s), multiplier (2.0), max_delay (30s), retryable_statuses (500, 502, 503, 504)
- [x] Exponential backoff retry loop wrapping HTTP dispatch
- [x] `CircuitBreaker`: failure_threshold (5), cooldown (60s), per-origin state
- [x] Short-circuit during cooldown; reset on success
- [x] Independent of ANML-level `retry-budget`

### Task 21: Authentication providers (`src/auth.rs`)
- [x] `BearerTokenProvider` (static token)
- [x] `ApiKeyProvider` (header-based or query-based)
- [x] Integration into action execution pipeline

## Phase 4: State & Serialization

### Task 22: State persistence
- [x] `Serialize`/`Deserialize` on `ConsentStore`, `ConsentGrant`, `ConsentKey` (behind `serde`)
- [x] `ClientState` struct aggregating consent store, audit log, flow state
- [x] `client.export_state()` and `builder.restore_state(state)`
- [x] Round-trip serialization tests

### Task 23: Display implementations
- [x] `Display` for `AnmlDocument` — concise summary: title, version, ask/action counts, flow step, status
- [x] `Debug` for all public types
- [x] `Display` for `FlowNavigator` — current step, progress
- [x] Tests verifying output format

## Phase 5: Testing

### Task 24: Testing module (feature-gated `testing`)
- [x] `MockAnmlServer` using `axum` — configurable document responses, action recording
- [x] `fixtures.rs` — pre-built documents: simple service, multi-step flow, disclosure-gated, paginated, error/problem, extension-required, deferred ask (no action attr)
- [x] `assertions.rs` — helpers: `assert_received(action_id, ...)`, `assert_disclosure_granted(field)`, `assert_param(name, value)`

### Task 25: Property-based tests — disclosure matching
- [x] `proptest` Arbitrary impl for disclosure rule sets (field, field-prefix, field-pattern, default, requires, consent-scope)
- [x] Properties: exact wins over prefix/pattern/default; longest prefix wins; fewest metacharacters wins; default only when no other match; missing rule synthesizes explicit/session; mutual exclusion (field+prefix, field+pattern, prefix+pattern never coexist)

### Task 26: Property-based tests — parameter binding
- [x] `proptest` Arbitrary impl for param sets (name, value, type)
- [x] Properties: urlencoded deterministic (same input → same bytes); JSON preserves document order (keys not sorted); multipart boundary has cryptographic randomness; required params with include=false omitted; typed values use canonical forms (XSD decimal, boolean, dateTime)

### Task 27: Property-based tests — consent store and rate limits
- [x] Properties: session scope isolated per client instance; origin scope visible to same origin not siblings; global scope cross-origin; revoke removes exactly targeted grant; rate limit 24h sliding window correct; rate limit resets after 24h

### Task 28: Integration tests
- [x] Happy path: discover → fetch → inspect → answer → execute → parse
- [x] Multi-step flow: 3+ steps with state transitions and condition evaluation
- [x] Disclosure gate: explicit consent triggers callback; deferred ask runs disclosure but no HTTP
- [x] Errors: 406 version mismatch, integrity mismatch, plaintext rejection, SSRF block, trust denial, unsupported profile, unsupported extension
- [x] Pagination: 3 pages, all items collected
- [x] URI resolution: relative endpoint resolved against document origin; xml:base override

### Task 29: Compile-fail tests
- [x] `trybuild` harness
- [x] Missing required param → compile error
- [x] All required params set → compiles
- [x] Optional params omitted → compiles

## Phase 6: Documentation & Polish

### Task 30: Examples
- [x] `examples/basic_fetch.rs` — discover, fetch, inspect
- [x] `examples/action_execution.rs` — disclosure, execute, handle response
- [x] `examples/multi_step_flow.rs` — flow with error recovery and condition evaluation
- [x] `examples/custom_auth.rs` — AuthProvider for OAuth2 refresh
- [x] `examples/middleware.rs` — request logging, custom headers

### Task 31: Documentation and project hygiene
- [x] Crate-level rustdoc with overview and quick start
- [x] Complete `README.md`: installation, features, usage, security model
- [x] `CONTRIBUTING.md` with dev setup, testing, PR guidelines
- [x] Verify `Cargo.toml` metadata: description, repository, keywords, categories, license, rust-version
