# ANML Client for Rust — Requirements

## Overview

Build `anml-client`, an RFC-compliant Rust crate that makes it easy for developers building agents to interact with ANML documents served over HTTP(S). The client is the consumer-side counterpart to the `anml` server crate, reusing its types, parser, serializer, and builder rather than reimplementing them. All client code is self-contained in this repository.

Reference documents:
- #[[file:_reference/RFCs/ANML/draft-jeskey-anml-00.xml]] (the ANML 1.0 Internet-Draft)
- #[[file:_reference/RFCs/ANML/anml.xsd]] (normative XML Schema)
- #[[file:_reference/anml-server-rust/ARCHITECTURE.md]] (server crate architecture)

---

## 0. User Stories

- As a developer building an agent, I want to discover an ANML service from just a domain name, so I can start interacting without knowing the exact endpoint.
  - Acceptance: `client.discover("example.com")` returns a `DiscoveryResult` with the ANML endpoint URL, trying well-known URI, Link header, HTML link, and DNS-SD in order.
- As a developer, I want to fetch an ANML document and inspect what the service is asking for, what actions are available, and what constraints apply, before deciding what to disclose.
  - Acceptance: `client.fetch("/service")` returns an `AnmlDocument` with accessors for all sections. No side effects occur from fetching alone.
- As a developer, I want the client to automatically enforce disclosure rules so I can't accidentally leak user data without proper consent.
  - Acceptance: calling `build_answer()` or `execute_action()` with answer values runs the full 7-step disclosure algorithm. If any step fails, a `<refuse>` is emitted instead of an `<answer>`. There is no code path that bypasses disclosure evaluation.
- As a developer, I want to execute a multi-step flow (e.g., search → select → pay → confirm) and have the client track state for me.
  - Acceptance: `FlowNavigator` tracks current step, handles `next-on-error` transitions, enforces `retry-budget`, and updates state from service responses. `flow.advance(params)` executes the current step's action and returns the next document.
- As a developer, I want to answer an `<ask>` and have the client handle consent checking, tokenization, parameter encoding, and idempotency keys automatically.
  - Acceptance: `client.action(&doc, "submit-airline").param("airline", "Delta").execute()` runs disclosure, encodes params per the action's enctype, attaches Idempotency-Key if needed, and returns the parsed response.
- As a developer, I want clear compile-time errors when I forget a required parameter, not runtime surprises.
  - Acceptance: `client.action(&doc, "submit-reason").execute()` fails to compile if `<param name="reason" required="true">` has not been set via `.param("reason", ...)`.
- As a developer, I want to plug in my own auth provider, trust policy, and consent handler without forking the library.
  - Acceptance: `AnmlClient::builder().trust_policy(my_policy).auth_provider(my_auth).consent_handler(my_handler).build()` compiles and works with any implementation of the respective traits.
- As a developer, I want to write integration tests against a mock ANML server without spinning up real infrastructure.
  - Acceptance: `MockAnmlServer::new().document("/service", fixture).build()` starts an in-process HTTP server. Tests run in <1s with no network I/O.

---

## 1. Functional Requirements

### 1.1 Service Discovery
The client MUST support all four RFC-defined discovery mechanisms:
1. Well-known URI (`/.well-known/anml`) — fetch and parse the manifest document.
2. HTTP Link header (`rel="alternate" type="application/anml+xml"`) — extract ANML endpoint from any HTTP response.
3. HTML `<link>` element (feature-gated `html-discovery`) — parse HTML for `<link rel="alternate" type="application/anml+xml">`.
4. DNS-SD (feature-gated `dns-sd`) — resolve `_anml._tcp` SRV + TXT `v=anml1` via pure-Rust DNS resolver.

### 1.2 Document Fetching, Parsing & Negotiation
- Fetch ANML documents over HTTP(S) with `Accept: application/anml+xml; version="1.0"` (configurable).
- Validate `Content-Type: application/anml+xml` before parsing; reject non-ANML payloads with a typed error.
- Delegate parsing to the `anml` crate's `parser::parse()`.
- Support encoding detection per RFC precedence: HTTP charset > XML decl > BOM > UTF-8 default.
- Handle HTTP 406 (Not Acceptable) with structured `unsupported-version` problem parsing.
- Read `anml/@version` and `anml/@supported-versions` from fetched documents.

### 1.3 Document Inspection
Provide ergonomic accessors for all top-level ANML sections: head, constraints, state, interact, knowledge, persona, aesthetic, body, footer, status.

### 1.4 Disclosure Evaluation

The client MUST implement the RFC disclosure evaluation algorithm exactly as specified in Section 8.5. The algorithm is executed for every `<answer>` the client prepares to emit. The 7 steps are:

1. **Resolve the rule.** Find the `<disclosure>` in `<constraints>` whose `field` matches the answer's field, using the matching precedence defined in 1.4.1. If no rule matches, synthesize `requires="explicit"` + `consent-scope="session"` and log a policy-quality warning.
2. **Check rate limit.** If the rule has `rate-limit`, verify the (field, origin) pair has not exceeded the limit in the trailing 24-hour sliding window. On violation, emit `<refuse reason="rate-limited" retry-after="...">` and stop.
3. **Check consent.** Consult the consent store for (field, origin, consent-scope):
   - `explicit`: invoke the `ConsentHandler` callback to obtain affirmative, field-specific authorization from the principal. Block until answered.
   - `implicit`: check the consent store for a prior standing grant matching (field, origin, scope). If none, treat as explicit.
   - `authentication`: verify an authenticated session exists with the origin (via `AuthProvider`).
   - `none`: no consent check, but trust policy still applies.
   - On failure: emit `<refuse reason="user-denied">` or `<refuse reason="constraint-violation">` and stop.
4. **Check trust policy.** Call `TrustPolicy::evaluate(origin, doc)`. On denial, emit `<refuse reason="trust-insufficient">` and stop.
5. **Validate value.** If the `<ask>` declared `type`, `pattern`, or `one-of`, validate the answer value. On failure, emit `<refuse reason="unsupported-field">` and stop.
6. **Tokenize if requested.** If the rule has `tokenize="true"`, replace the plaintext value with a structured token via the `Tokenizer` and set `tokenized="true"` on the `<answer>`.
7. **Emit and audit.** Construct the `<answer>` with `field`, `value`, and `consent` basis. Record an audit entry: (field, origin, timestamp, consent basis, rule reference). Submit to the endpoint resolved via action binding.

No step may be skipped. Additional checks (DLP, anomaly detection) MAY be inserted between steps.

#### 1.4.1 Disclosure Matching Precedence

When resolving which `<disclosure>` governs a field, the client MUST use this total order (ties broken by document order, first wins):

1. Exact `field` attribute match.
2. `field-prefix` match — longest prefix wins. `field-prefix="contact"` matches `contact`, `contact.email`, `contact.phone.home`, but NOT `contacts`.
3. `field-pattern` match — fewest metacharacters wins; further ties by longest literal prefix before first metacharacter. Supported metacharacters: `*` (zero+ chars except `.`), `**` (zero+ chars including `.`), `?` (exactly one char except `.`). Backslash escapes literals.
4. `default="true"` disclosure.
5. If nothing matches: treat as `requires="explicit"` + `consent-scope="session"`.

A `<disclosure>` MUST NOT carry both `field` and `field-prefix`, nor `field` and `field-pattern`, nor `field-prefix` and `field-pattern`.

### 1.5 Action Execution
- Execute actions from `<interact>` via the HTTP method and endpoint specified.
- Implement the parameter binding algorithm (Section 10.9) for all three enctypes, with the following canonical form requirements critical for idempotency key stability:

#### 1.5.1 Parameter Binding Canonical Forms

**Parameter collection:** Walk `<action>` children in document order, collect each `<param>` as (name, value, type). Omit params whose `include` resolves to false. Strip leading/trailing ASCII whitespace from text content values.

**`application/x-www-form-urlencoded`:** Percent-encode name and value (space → `+`, unreserved set: `ALPHA / DIGIT / "-" / "." / "_" / "~"`). Emit `name=value` pairs joined by `&` in document order. Typed values use canonical lexical forms: XSD decimal for numbers, XSD boolean (`true`/`false`) for booleans, XSD dateTime for dates. `Content-Length` MUST equal the octet length of the body.

**`multipart/form-data`:** One part per param with `Content-Disposition: form-data; name="NAME"`. Boundary MUST be `anml-` + document `@id` + 96-bit cryptographically random suffix in base32-no-pad. Parts in document order. File params carry `filename` and `Content-Type`.

**`application/json`:** JSON object with members in document order (keys MUST NOT be sorted — this is critical for idempotency key stability). Duplicate names collected into arrays. Numbers as JSON numbers (IEEE-754 double precision; out-of-range as strings), booleans as JSON booleans, dateTime as RFC 3339 strings. Minified UTF-8, no BOM.

#### 1.5.2 Endpoint URI Resolution

Relative `endpoint` URIs (e.g., `/airline`) MUST be resolved against the document's origin (scheme + host + port of the URL from which the document was fetched). If `xml:base` is declared on an ancestor element, it takes precedence per RFC 2396 / XML Base. Absolute URIs are used as-is but MUST still pass trust policy and SSRF checks.

#### 1.5.3 `<ask>` Without `action` Attribute

When an `<ask>` has no `action` attribute, the client MUST NOT attempt to submit an answer via HTTP. Instead:
- The client MUST expose the ask to the consuming application as a "deferred ask" — the application is responsible for including the answer in a subsequent ANML document sent to the service via any agreed channel.
- The `build_answer()` helper MUST still run disclosure evaluation for deferred asks.
- The `ActionRequestBuilder` MUST NOT be usable for deferred asks (no action to bind to).

- Validate parameters against type, required, pattern, min, max, and enum options.
- Respect `auth` (none/required/optional) via pluggable auth provider, and `confirm` (invoke confirmation callback).
- Support `Idempotency-Key` header generation (UUIDv4, ≥128 bits entropy) and RFC retry semantics.
- Provide a fluent `ActionRequestBuilder` with typestate enforcement of required params at compile time.
- Parameter setters accept `impl Into<String>`, `i64`, `f64`, `u64`, `bool` with automatic canonical form conversion.

### 1.6 Knowledge Exchange
- Build agent response documents using the `anml` crate's `ResponseBuilder`.
- Support emitting `<answer>` (with `consent`), `<refuse>` (with `reason`), `<ask>`, and `<inform>`.
- Support symmetric knowledge exchange (agent can also ask and inform).

### 1.7 Flow Navigation
- Track multi-step workflow state from `<state>/<flow>`.
- Provide `current()`, `pending()`, `completed()`, `is_complete()` accessors.
- Support step transitions via action execution.
- Support flow error recovery: `next-on-error` and `retry-budget` with exponential backoff (1s base, 2x multiplier, cap 60s or TTL).
- Detect state regressions (step moving backward) and surface as warnings.

#### 1.7.1 Step `condition` Attribute

The RFC defines a `condition` attribute on `<step>` but does not specify an expression language. The client MUST:
- Expose `condition` as a raw `Option<String>` on the step accessor.
- Provide a `ConditionEvaluator` callback trait that the consuming application can implement to evaluate conditions.
- If no `ConditionEvaluator` is configured and a step has a `condition`, the client MUST treat the step as available (condition = true) and log a warning that the condition was not evaluated.
- The `FlowNavigator` MUST call the evaluator before transitioning to a step with a condition; if the evaluator returns false, the step is skipped.

### 1.8 Subresource Integrity (SRI)
- Verify `integrity` attributes on `<img>`, `<audio>`, `<video>`, `<link>` when `inference="required"`.
- Support `sha256`, `sha384` (minimum); `sha512` recommended. Select strongest when multiple tokens present.
- Missing integrity on `inference="required"` = malformed document.
- Integrity mismatches block resource use and log security events.

### 1.9 Pagination
- Support `<nav>` elements (next, prev, cursor, total).
- Provide an async stream abstraction over paginated data.

### 1.10 Confidentiality & Usage Rights
- Honor `confidentiality` on `<inform>`: `private` MUST NOT be forwarded; `restricted` gates forwarding behind principal approval.
- Expose `<rights>` (holder, year, license, usage level) and `<attribution>` via accessors.
- Respect usage hierarchy: `none < display < cache < store < train`.

### 1.11 TTL & Caching
- Respect `ttl` on `<anml>` root and `<inform>` elements.
- Provide a document cache keyed by URL with TTL-based expiration (feature-gated `cache`).

### 1.12 Error Handling
- Parse `<status>` responses including `<problem>` children.
- Map RFC problem type URIs to typed error variants.
- Handle HTTP-level errors (4xx, 5xx) gracefully.

---

## 2. Security Requirements (RFC §11)

### 2.1 Transport Security
- Default to HTTPS; HTTP requires explicit `allow_plaintext_http` opt-in.
- Use `rustls` (pure Rust TLS), not system OpenSSL.
- Refuse documents with `<constraints>`, `<interact>`, or `<ask requires="explicit">` when fetched over HTTP.

### 2.2 XML Parsing Safety
- Disable external DTD/entity resolution and XInclude (inherited from `anml` crate; client MUST NOT relax).
- Parse to end-of-document before any externally observable action (no streaming consumption).

### 2.3 Resource Limits
Enforce RFC defaults (configurable but not disableable): 1 MiB doc size, 64 depth, 10k elements, 64 KiB attr value, 256 attrs/element, 256 KiB text/element, 64:1 entity expansion, 5s parse timeout, 1k disclosure rules, 1k knowledge primitives, 256 steps/flow. Decompression-aware; do not trust `Content-Length`.

### 2.4 Action Safety
- Verify endpoint URIs against trust policy before executing.
- SSRF protection: reject private/loopback IPs (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1, fc00::/7).
- Enforce per-document budgets: max distinct origins (5), max requests (50), max media fetches (20), max media bandwidth (50 MiB).
- Require confirmation for `confirm="true"` actions and state-modifying actions at unfamiliar origins.

### 2.5 Trust Policy
- Provide `TrustPolicy` trait consulted before acting on any document.
- Default: deny all origins unless explicitly allowed.
- Disclosure algorithm step 4 invokes trust policy; denial emits `<refuse reason="trust-insufficient">`.

### 2.6 Prompt Injection Defense
- Treat `<persona>`, `<instructions>`, `<vocabulary>`, `<inform>`, and `<body>` text as untrusted data.
- Expose as data, clearly labeled service-supplied. Principal intent always overrides.

### 2.7 Tokenization
- HMAC-SHA256 with per-client secret; tokens are hex-encoded, non-reversible, non-URL-safe, bound to (field, principal, origin).

### 2.8 Cross-Origin Media Isolation
- No credentials/cookies on cross-origin media fetches. Use isolated HTTP client.

### 2.9 Extension Namespace Safety
- Ignore unknown extensions per RFC.
- Refuse documents declaring `<meta name="requires-ext">` for unrecognized extensions.

### 2.10 Conformance Profile Handling
- On document parse, scan `<meta name="profile">` entries.
- The client MUST declare which profiles it implements (at minimum: `urn:ietf:anml:profile:core-1.0`).
- If a document declares a profile the client does not implement, the client MUST refuse the document with an `unsupported-profile` problem naming the missing profile URI.
- The client MUST NOT silently degrade when a required profile is missing.

### 2.11 Audit Logging
- Append-only audit log of disclosure events: field, origin, timestamp, consent basis, rule reference.
- `AuditLog` trait for custom implementations; `InMemoryAuditLog` default.

### 2.11 Privacy & Consent
- Consent store with session/origin/global scoping; inspect, revoke, list.
- `consent-scope="origin"` is binding across sibling origins.
- Prefer tokenization when available for linkable fields.

### 2.12 DoS Protection
- Per-origin/per-document resource budgets.
- Exponential backoff on `rate-limited`/`retry-after`.
- All retry paths bounded.

---

## 3. Non-Functional Requirements

### 3.1 Pure Rust
- Entirely Rust; no external binaries or system commands.
- DNS-SD via `hickory-resolver`, SRI via `sha2`, HTML parsing via `scraper`/`html5ever`, TLS via `rustls`.

### 3.2 Async-First
- `tokio`-based async by default. Blocking wrapper behind `blocking` feature flag.

### 3.3 Dependency on `anml` Crate
- Git dependency: `anml = { git = "https://github.com/Life-Savor-AI/anml-server-rust.git", features = ["serde"] }`.
- `_reference/anml-server-rust` submodule is read-only reference. No path dependencies.
- Do not duplicate types or parsing logic.

### 3.4 Feature Flags
- `dns-sd` — DNS service discovery.
- `html-discovery` — HTML link element parsing.
- `cache` — In-memory TTL document cache.
- `blocking` — Synchronous API wrapper.
- `testing` — Mock server, fixtures, assertion helpers.
- `serde` (default) — Serde derives on client state types.
- `wasm` (reserved, not v1) — Future WASM target support.

### 3.5 MSRV
- 1.80 (matching the `anml` crate).

---

## 4. Developer Experience Requirements

### 4.1 Ergonomic API
- `AnmlClient` as primary entry point; `Clone + Send + Sync` (Arc-wrapped internals).
- Common workflows (discover → fetch → inspect → act → respond) in a few method calls.
- Builder patterns for configuration.
- Prelude module re-exporting core types for 90% of use cases.
- Document that users should create one client and share it (connection pooling).

### 4.2 Actionable Errors
- Every error variant includes contextual fields. Messages phrased as "expected X, got Y" or "field 'Z' requires W".
- Disclosure errors: field, rule, requires, scope, failure reason.
- Param validation errors: param name, constraint, actual value.
- Action errors: action id, method, endpoint, HTTP status.

### 4.3 Display & Debug
- `AnmlDocument` `Display`: concise summary (title, version, ask/action counts, flow step, status).
- `AnmlClientError` `Display`: single-line actionable messages.
- All public types implement `Debug`.

### 4.4 Comprehensive Timeouts
- Per-request (30s), per-action (60s), per-flow (5min), per-media-fetch (15s), parse (5s).
- Flow exceeding total timeout aborts with `FlowAborted`.

### 4.5 Type Conversions
- Param setters accept `impl Into<String>`, `i64`, `f64`, `u64`, `bool`.
- Consent basis settable via enum or `&str`.

### 4.6 Semver & Non-Exhaustive
- Strict semver. `#[non_exhaustive]` on all public enums and config structs.
- `CHANGELOG.md` following Keep a Changelog format.

### 4.7 `no_std` Core (Future)
- Disclosure engine, consent store, param validation as pure functions with no I/O.
- Architecture must not prevent future `anml-client-core` extraction.

---

## 5. Operational Requirements

### 5.1 Observability
- `tracing` integration at all key decision points with span context.
- Sensitive values not logged above TRACE. Security events at WARN/ERROR.

### 5.2 HTTP Middleware
- Composable request/response interceptors via `HttpMiddleware` trait.

### 5.3 Retry & Circuit Breaker
- Configurable retry policy for transient HTTP errors (3 retries, 1s base, 2x, 30s cap).
- Per-origin circuit breaker (5 failures, 60s cooldown).
- Independent of ANML-level `retry-budget`.

### 5.4 Authentication Provider
- `AuthProvider` trait: bearer tokens, API keys, custom headers, async token refresh.
- Credentials never sent to uncovered origins. On 401, call `on_unauthorized()` and retry once.

### 5.5 State Persistence
- `ConsentStore`, `AuditLog`, `FlowNavigator` serializable via serde (behind `serde` flag).
- `export_state()` / `restore_state()` for cross-restart persistence.

### 5.6 Testing Support (feature-gated `testing`)
- `MockAnmlServer` — in-process HTTP server with configurable responses and request recording.
- Pre-built fixture documents for common patterns.
- Assertion helpers for disclosure, consent, and action params.

---

## 6. Testing Requirements

### 6.1 Property-Based Testing
`proptest` for: disclosure matching precedence, parameter binding determinism, rate limit sliding windows, consent store scoping, glob pattern matching.

### 6.2 Integration Tests
Full lifecycle via `MockAnmlServer`: happy path, multi-step flow, disclosure gates, error paths (406, integrity, SSRF, trust denial), pagination.

### 6.3 Compile-Fail Tests
`trybuild` verifying typestate enforcement: missing required param = compile error.

### 6.4 Coverage
Target >90% line coverage on non-trivial logic. Measurable via `cargo-tarpaulin` or `cargo-llvm-cov`.
