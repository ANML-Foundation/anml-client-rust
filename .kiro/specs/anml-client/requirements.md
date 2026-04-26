# ANML Client for Rust — Requirements

## Overview

Build `anml-client`, an RFC-compliant Rust crate that makes it easy for users (developers building agents) to interact with ANML documents served over HTTP(S). The client is the consumer-side counterpart to the `anml` server crate (`anml-server-rust`), reusing its types, parser, serializer, and builder rather than reimplementing them.

Reference documents:
- #[[file:_reference/RFCs/ANML/draft-jeskey-anml-00.xml]] (the ANML 1.0 Internet-Draft)
- #[[file:_reference/RFCs/ANML/anml.xsd]] (normative XML Schema)
- #[[file:_reference/anml-server-rust/ARCHITECTURE.md]] (server crate architecture)

## Functional Requirements

### FR-1: Service Discovery
The client MUST support all four RFC-defined discovery mechanisms:
1. Well-known URI (`/.well-known/anml`) — fetch and parse the manifest document.
2. HTTP Link header (`rel="alternate" type="application/anml+xml"`) — extract ANML endpoint from any HTTP response.
3. HTML `<link>` element — parse an HTML page for `<link rel="alternate" type="application/anml+xml">`.
4. DNS-SD (`_anml._tcp` SRV + TXT `v=anml1`) — resolve service location from DNS.

### FR-2: Document Fetching & Parsing
- The client MUST fetch ANML documents over HTTP(S) with `Accept: application/anml+xml`.
- The client MUST delegate parsing to the `anml` crate's `parser::parse()`.
- The client MUST validate `Content-Type: application/anml+xml` on responses.
- The client MUST support encoding detection per RFC precedence: HTTP charset > XML decl > BOM > UTF-8 default.

### FR-3: Content & Version Negotiation
- The client MUST send `Accept: application/anml+xml; version="1.0"` (configurable).
- The client MUST handle HTTP 406 (Not Acceptable) with a structured `unsupported-version` problem.
- The client MUST read `anml/@version` and `anml/@supported-versions` from fetched documents.

### FR-4: Document Inspection
The client MUST provide ergonomic accessors for all top-level ANML sections:
- `head` (title, meta)
- `constraints` (disclosure rules)
- `state` (context, flow, steps)
- `interact` (actions, params, options, responses)
- `knowledge` (inform, ask)
- `persona`, `aesthetic`
- `body` (sections, data, items, fields, nav, media)
- `footer` (rights, attribution)
- `status` (code, result, problem)

### FR-5: Disclosure Evaluation
- The client MUST implement the RFC disclosure evaluation algorithm (Section 8.5).
- Before emitting any `<answer>`, the client MUST evaluate the `<constraints>` section.
- The client MUST support disclosure matching: exact `field`, `field-prefix`, `field-pattern`, and `default="true"` with the RFC-defined precedence.
- The client MUST support `requires` values: `explicit`, `implicit`, `authentication`, `none`.
- The client MUST support `consent-scope`: `session`, `origin`, `global`.
- The client MUST support `rate-limit` enforcement.
- The client MUST support `tokenize` flag.
- Missing disclosure rules for an `<ask>` MUST be treated as `requires="explicit"` with `consent-scope="session"`.

### FR-6: Action Execution
- The client MUST execute actions defined in `<interact>` by performing the HTTP request specified by `method` and `endpoint`.
- The client MUST implement the parameter binding algorithm (Section 10.9) for all three enctypes:
  - `application/x-www-form-urlencoded`
  - `multipart/form-data`
  - `application/json`
- The client MUST support parameter validation (type, required, pattern, min, max, enum options).
- The client MUST respect `auth` (none/required/optional) and `confirm` (prompt user before executing).
- The client MUST support `idempotent` actions with `Idempotency-Key` header generation and retry semantics per RFC Section 8.6.

### FR-7: Knowledge Exchange — Response Building
- The client MUST build agent response documents using the `anml` crate's `ResponseBuilder`.
- The client MUST support emitting `<answer>`, `<refuse>`, `<ask>`, and `<inform>` elements.
- `<answer>` MUST include `consent` attribute (explicit/implicit/delegated).
- `<refuse>` MUST include `reason` from the RFC-defined set.
- The client MUST support symmetric knowledge exchange (agent can also `<ask>` and `<inform>`).

### FR-8: Flow Navigation
- The client MUST track multi-step workflow state from `<state>/<flow>`.
- The client MUST identify the current step, completed steps, and pending steps.
- The client MUST support step transitions via action execution.
- The client MUST support flow error recovery: `next-on-error` and `retry-budget` with exponential backoff.

### FR-9: Confidentiality Enforcement
- The client MUST honor `confidentiality` on `<inform>`: `public`, `restricted`, `private`.
- `private` content MUST NOT be forwarded to third parties.
- `restricted` content SHOULD gate forwarding behind principal approval.

### FR-10: Usage Rights
- The client MUST read and expose `<rights>` declarations (holder, year, license, usage level).
- The client MUST respect the usage hierarchy: `none < display < cache < store < train`.
- The client MUST expose `<attribution>` requirements.

### FR-11: TTL & Caching
- The client MUST respect `ttl` on `<anml>` root and `<inform>` elements.
- The client SHOULD provide a document cache keyed by URL with TTL-based expiration.

### FR-12: Error Handling
- The client MUST parse `<status>` responses including `<problem>` children.
- The client MUST map RFC problem type URIs to typed errors.
- The client MUST handle HTTP-level errors (4xx, 5xx) gracefully.

### FR-13: Subresource Integrity (SRI)
- The client MUST verify `integrity` attributes on `<img>`, `<audio>`, `<video>`, and `<link>` elements when `inference="required"`.
- The client MUST support `sha256` and `sha384` algorithms (minimum); `sha512` RECOMMENDED.
- Integrity mismatches MUST prevent the resource from being used.

### FR-14: Pagination
- The client MUST support `<nav>` elements for paginated results (next, prev, cursor, total).
- The client SHOULD provide an iterator/stream abstraction over paginated data.

## Non-Functional Requirements

### NFR-1: Async-First
- The client MUST be async (`tokio`-based) by default.
- A blocking convenience layer MAY be provided behind a feature flag.

### NFR-2: Dependency on `anml` Crate
- The client MUST depend on the `anml` crate as a git dependency from the private `Life-Savor-AI/anml-server-rust` GitHub repository.
- The dependency MUST be specified as `anml = { git = "https://github.com/Life-Savor-AI/anml-server-rust.git", features = ["serde"] }` (or SSH equivalent).
- All client code MUST be self-contained in this repository. The `_reference/anml-server-rust` submodule is for reading/understanding only — not for path dependencies.
- The client MUST NOT duplicate type definitions or parsing logic that the `anml` crate already provides.

### NFR-3: HTTP Client
- The client SHOULD use `reqwest` as the HTTP backend.
- The HTTP client MUST be configurable (timeouts, TLS, proxy, custom headers).

### NFR-4: Ergonomic API
- The client MUST provide a high-level `AnmlClient` struct as the primary entry point.
- Common workflows (discover → fetch → inspect → act → respond) MUST be achievable in a few method calls.
- Builder patterns for configuration.

### NFR-5: Feature Flags
- `dns-sd` — DNS service discovery (optional, adds DNS dependency).
- `html-discovery` — HTML link element parsing (optional, adds HTML parser dependency).
- `cache` — In-memory document/response caching.
- `blocking` — Synchronous API wrapper.

### NFR-6: Pure Rust — No External Binaries or Services
- The client MUST be implemented entirely in Rust. Third-party Rust crates are permitted.
- The client MUST NOT shell out to external binaries, invoke system commands, or depend on external services for any of its functionality (e.g., no calls to `openssl` CLI, `dig`, `curl`, or similar).
- DNS-SD resolution MUST use a pure-Rust DNS resolver (e.g., `hickory-resolver`), not system `dig`/`nslookup`.
- SRI hashing MUST use pure-Rust crypto (e.g., `sha2` crate), not system `openssl`.
- HTML parsing for discovery MUST use a pure-Rust parser (e.g., `scraper`/`html5ever`), not an external tool.

### NFR-7: MSRV
- Minimum Supported Rust Version: 1.80 (matching the `anml` crate).

## Security Requirements

The following requirements are derived from RFC Section 11 (Security Considerations) and Section 11a (Privacy Considerations). They define the security posture of a conforming ANML client.

### SR-1: Transport Security (RFC §11.13)
- The client MUST default to HTTPS. HTTP MUST require explicit opt-in via configuration (`allow_plaintext_http`).
- The client MUST refuse to process documents containing `<constraints>`, `<interact>`, or `<ask requires="explicit">` when retrieved over unencrypted HTTP.
- The client MUST use `rustls` (pure Rust TLS) by default for TLS, not system OpenSSL.

### SR-2: Content-Type Downgrade Prevention (RFC §11.14)
- The client MUST verify that the response `Content-Type` begins with `application/anml+xml` before parsing.
- The client MUST NOT attempt to "upgrade-parse" non-ANML payloads (e.g., HTML, JSON) as ANML documents.
- On Content-Type mismatch, the client MUST return a typed error and MUST NOT pass the body to the XML parser.

### SR-3: XML Parsing Attack Mitigation (RFC §11.5)
- The client MUST disable external DTD subset resolution, external parameter entity resolution, external general entity resolution, and XInclude processing. (Inherited from the `anml` crate's parser, but the client MUST verify these are enforced and MUST NOT override them.)
- The client MUST NOT configure the `anml` parser in a way that relaxes these protections.

### SR-4: Resource Limits (RFC §11.6)
The client MUST enforce the following default limits (configurable upward/downward, but MUST NOT be disabled entirely):

| Limit | Default |
|-------|---------|
| Maximum document size (post-decompression) | 1 MiB |
| Maximum element nesting depth | 64 |
| Maximum element count | 10,000 |
| Maximum attribute value length | 64 KiB |
| Maximum attributes per element | 256 |
| Maximum text-content length per element | 256 KiB |
| Maximum entity expansion ratio | 64:1 |
| Maximum parse wall-clock time | 5 s |
| Maximum `<disclosure>` rules per document | 1,000 |
| Maximum `<ask>` + `<answer>` + `<refuse>` per document | 1,000 |
| Maximum `<step>` per `<flow>` | 256 |

- When a document is received compressed (Content-Encoding: gzip/br), the size limit applies to the decompressed length. The client MUST halt decompression at the limit.
- The client MUST NOT rely on `Content-Length` for security; limits are enforced on the actual byte stream.

### SR-5: No Streaming Consumption (RFC §11.7)
- The client MUST parse a document to end-of-document before executing any action, emitting any `<answer>`, `<refuse>`, or outbound `<ask>`.
- This holds even when the HTTP body arrives in chunks.
- Internal incremental parsing is permitted, but no externally observable action may occur before end-of-document.

### SR-6: Prompt Injection / Behavioral Manipulation Defense (RFC §11.3)
- The client MUST treat content in `<persona>`, `<instructions>`, `<vocabulary>`, `<inform>`, and free-text `<body>` as untrusted data, not as executable instructions.
- The client MUST NOT honor directives within these elements to ignore constraints, disclose secrets, modify security policy, or execute actions not defined in `<interact>`.
- The client MUST expose these elements as data to the consuming application, clearly labeled as service-supplied and untrusted.
- Principal (user) intent MUST always override document-supplied behavioral guidance.

### SR-7: Action Execution Safety (RFC §11.4)
- The client MUST resolve action `endpoint` URIs and verify them against the configured trust policy before executing.
- The client MUST require principal confirmation for actions with `confirm="true"`.
- The client SHOULD require confirmation for any state-modifying action (POST/PUT/PATCH/DELETE) at an unfamiliar origin.
- The client MUST NOT execute an action that would violate a constraint the principal has declared.
- The client MUST enforce a configurable limit on the number of distinct origins that actions in a single document may target (default: 5). Documents exceeding this limit MUST be refused.
- The client MUST enforce a configurable limit on the total number of HTTP requests generated from a single document (default: 50).

### SR-8: Trust Policy (RFC §11.9)
- The client MUST provide a `TrustPolicy` trait that is consulted before acting on any document.
- The default trust policy MUST deny all origins unless explicitly allowed.
- Trust policy inputs SHOULD include: origin (scheme/host/port), TLS certificate status, user allow/deny lists.
- The disclosure algorithm (step 4) MUST invoke the trust policy; if it denies, the client MUST emit `<refuse reason="trust-insufficient">`.

### SR-9: Cross-Origin Media Isolation (RFC §11.12)
- When fetching media referenced by `<img>`, `<audio>`, `<video>`, or `<link>`, the client MUST NOT attach credentials, cookies, or principal identifiers to cross-origin requests unless the principal has explicitly authorized it.
- The client MUST enforce a configurable limit on the number of media resources fetched per document (default: 20) and total media bandwidth per document (default: 50 MiB).

### SR-10: Tokenization Security (RFC §11.15)
- When `tokenize="true"` is set on a disclosure rule, the client MUST generate structured tokens that are:
  - Collision-resistant
  - Bound to the (field, principal, origin) tuple
  - Non-reversible outside the agent's trust boundary
- Tokens MUST be generated using a cryptographically secure mechanism (e.g., HMAC-SHA256 with a per-client secret key).
- Tokens MUST NOT encode persistent principal identifiers in a form observable to the receiving service.
- Tokens MUST NOT be URL-safe in a way that allows trivial reuse as identifiers in downstream calls.

### SR-11: Idempotency Key Security (RFC §8.6.1)
- Idempotency keys MUST be generated as UUIDv4 or equivalent with at least 128 bits of entropy.
- Keys MUST be scoped to the (action id, ask set) tuple.
- On network-level failure, the client MUST retry with the same key.
- On application-level error, the client MUST NOT retry with the same key unless the error is explicitly retriable (`retry-after` or `transient` problem type).

### SR-12: Disclosure Audit Logging (RFC §11.16 operational)
- The client MUST maintain an append-only audit log of disclosure events.
- Each audit entry MUST contain at minimum: field, service origin, timestamp, consent basis, and disclosure rule reference.
- The audit log MUST be accessible to the principal for inspection.
- The client SHOULD provide a callback/trait for custom audit log implementations.

### SR-13: Extension Namespace Safety (RFC §11.15)
- The client MUST silently ignore unknown extension elements and attributes per the RFC.
- If a document declares `<meta name="requires-ext" value="...">` for an extension the client does not support, the client MUST refuse the document with an `unsupported-profile` problem rather than silently degrading.

### SR-14: Replay and State Integrity
- The client MUST NOT blindly trust `<state>` values from a service response without validating them against the expected flow progression.
- The client SHOULD detect unexpected state regressions (e.g., a step moving from "completed" back to "pending") and surface them to the principal.

### SR-15: Denial of Service Protection (RFC §11.11)
- The client MUST enforce per-origin and per-document resource budgets (request count, bandwidth, parse time).
- The client MUST implement exponential backoff when encountering `rate-limited` refusals or `retry-after` status responses.
- The client MUST NOT enter unbounded retry loops; all retry paths MUST be bounded by `retry-budget` or a configurable maximum.

### SR-16: Privacy — Consent Management
- The client MUST provide the principal with the ability to inspect outstanding standing consents, revoke them individually or in bulk, and view a history of disclosures per origin and per field.
- `consent-scope="origin"` MUST be binding: disclosure to one origin MUST NOT authorize disclosure to a sibling origin.
- The client SHOULD prefer `tokenize="true"` when available for fields whose plaintext would be linkable across origins.

### SR-17: Privacy — Confidentiality Labels (RFC §11a.5)
- `confidentiality="private"` on `<inform>` MUST prevent the client from forwarding that content to any third party.
- `confidentiality="restricted"` SHOULD gate forwarding behind principal approval.
- The client MAY enforce tighter handling than the service requested.

## Operational Requirements

### OR-1: Observability
- The client MUST integrate with the `tracing` crate for structured logging at all key decision points: fetch, parse, disclosure evaluation, action execution, retry, flow transitions, and security events.
- Log events MUST include span context (origin, document URL, action id, field) for correlation.
- The client MUST NOT log sensitive values (answer values, tokens, credentials) at any level below `TRACE`.
- Security-relevant events (trust denials, integrity mismatches, SSRF blocks, transport rejections) MUST be logged at `WARN` or `ERROR`.

### OR-2: HTTP Middleware Hooks
- The client MUST support request/response interceptors via a middleware trait, allowing users to inspect or modify HTTP requests before dispatch and responses before parsing.
- Use cases: custom auth header injection, request signing, logging, metrics collection, header rewriting.
- Middleware MUST be composable (multiple interceptors in a chain).

### OR-3: Retry & Resilience
- The client MUST support a configurable retry policy for transient HTTP errors (5xx, connection reset, timeout) independent of ANML-level `retry-budget`.
- Default: 3 retries with exponential backoff (1s base, 2x multiplier, 30s cap).
- The client SHOULD implement per-origin circuit breaking: after N consecutive failures (configurable, default 5), the client MUST short-circuit requests to that origin for a cooldown period (configurable, default 60s) before attempting again.

### OR-4: Authentication Provider
- The client MUST provide a pluggable `AuthProvider` trait for supplying credentials to actions with `auth="required"` or `auth="optional"`.
- The trait MUST support: bearer tokens, API keys (header or query), and custom header injection.
- The trait SHOULD support async token refresh (e.g., OAuth2 token rotation).
- The client MUST NOT send credentials to origins not covered by the auth provider.

### OR-5: Testing Support
- The client crate MUST ship a `testing` module (feature-gated) providing:
  - `MockAnmlServer` — an in-process HTTP server that serves configurable ANML documents and records received requests/responses.
  - Pre-built fixture documents covering common patterns: simple fetch, multi-step flow, disclosure-gated ask, paginated data, error/problem responses.
  - Assertion helpers for verifying disclosure decisions, action parameters, and consent state.

### OR-6: State Persistence
- The `ConsentStore`, `AuditLog`, and `FlowNavigator` state MUST be serializable via `serde` (behind the `serde` feature flag).
- This enables users to persist client state across process restarts (e.g., standing consents, audit history, in-progress flow state).
- The client MUST support constructing from previously serialized state.

### OR-7: Ergonomic Action Builder
- The client MUST provide a fluent `ActionRequestBuilder` for constructing action executions:
  ```
  client.action(&doc, "submit-airline")
      .param("airline", "Delta")
      .param("seat", "12A")
      .execute().await?;
  ```
- Parameter validation (type, required, pattern, enum) MUST occur at `execute()` time with clear error messages referencing the `<param>` definition.

### OR-8: WASM Compatibility (Future)
- The client architecture MUST NOT take dependencies that preclude future `wasm32-unknown-unknown` compilation.
- A `wasm` feature flag SHOULD be reserved (not implemented in v1) that swaps `reqwest`'s native backend for its WASM backend and disables `tokio`-specific features.
- File I/O, system time, and OS-level randomness MUST be abstracted behind traits so WASM targets can provide alternatives.

## Developer Experience Requirements

### DX-1: Actionable Error Messages
- Every error variant MUST include enough context for the developer to understand what went wrong and what to do about it without reading source code.
- Disclosure errors MUST include: the field name, the governing disclosure rule, what the rule required, and the reason it failed.
- Parameter validation errors MUST include: the param name, the expected constraint (type, pattern, enum values, min/max), and the actual value provided.
- Action execution errors MUST include: the action id, the HTTP method, the endpoint, and the HTTP status or network error.
- All error messages MUST be phrased as "expected X, got Y" or "field 'Z' requires W" — never bare "validation failed" or "error occurred".

### DX-2: Compile-Time Safety via Typestate
- `ActionRequestBuilder` MUST use typestate pattern to enforce required parameters at compile time.
- If an action's `<param>` has `required="true"`, the builder MUST NOT expose `.execute()` until that param has been set.
- This is implemented via generic type parameters that track which required params have been provided, so missing a required param is a compile error, not a runtime error.

### DX-3: Useful Display and Debug Implementations
- `AnmlDocument` MUST implement `Display` with a human-readable summary: title, origin, version, number of asks, number of actions, current flow step (if any), and status.
- All public types MUST implement `Debug`.
- `Display` on error types MUST produce single-line messages suitable for log output.
- `Debug` on document types MAY be verbose (full tree), but `Display` MUST be concise.

### DX-4: Comprehensive Timeouts
- The client MUST support configurable timeouts at every level:
  - Per-request timeout (individual HTTP call, default 30s)
  - Per-action timeout (including retries, default 60s)
  - Per-flow total timeout (entire multi-step flow, default 5 minutes)
  - Per-media-fetch timeout (individual resource fetch, default 15s)
  - Parse timeout (already specified in SR-4, default 5s)
- A flow that exceeds its total timeout MUST be aborted with a `FlowAborted` error regardless of individual step success.

### DX-5: Ergonomic Type Conversions
- Parameter setters MUST accept `impl Into<String>` for string values, not just `&str`.
- Numeric parameters MUST accept `i64`, `f64`, and `u64` directly with automatic canonical form conversion.
- Boolean parameters MUST accept `bool` directly.
- Consent basis MUST be settable via the enum or via `&str` (with validation).
- All builder methods MUST accept the most natural Rust type for the value being set.

### DX-6: Prelude Module
- The crate MUST provide an `anml_client::prelude` module that re-exports the types needed for 90% of use cases.
- Prelude MUST include at minimum: `AnmlClient`, `ClientConfig`, `AnmlClientError`, `Result`, `ConsentBasis`, `TrustPolicy`, `AllowListTrustPolicy`, `ActionRequestBuilder`, `FlowNavigator`, and key `anml` crate re-exports (`AnmlDocument`, `AnmlAction`, `AnmlAsk`, `AnmlAnswer`, `AnmlRefuse`).

### DX-7: Connection Pooling and Client Sharing
- `AnmlClient` MUST be `Clone`, `Send`, and `Sync` (wrapping internals in `Arc` as needed).
- Documentation MUST explicitly state that users should create one `AnmlClient` and share it across tasks/threads, not create a new client per request.
- The underlying `reqwest::Client` connection pool MUST be shared across all requests from the same `AnmlClient` instance.

### DX-8: Semver, Non-Exhaustive, and Changelog
- The crate MUST follow strict semver.
- All public enums and config structs MUST use `#[non_exhaustive]` (matching the `anml` server crate convention) to allow adding variants/fields in minor versions.
- The crate MUST ship a `CHANGELOG.md` following Keep a Changelog format.
- Breaking changes MUST only occur in major version bumps.

### DX-9: `no_std` Core (Future)
- The disclosure engine, consent store, parameter validation, and disclosure matching logic MUST be implemented as pure functions with no I/O dependencies.
- The architecture MUST NOT prevent extracting these into a future `anml-client-core` crate that is `no_std` compatible for embedded agents.
- This is a design constraint, not a v1 deliverable.

## Testing Requirements

### TR-1: Property-Based Testing
- The client MUST use `proptest` for property-based testing of all algorithmic logic.
- The following MUST have property-based tests:
  - Disclosure matching (field, field-prefix, field-pattern, default, precedence) — arbitrary field names and disclosure rule sets must produce deterministic, correct matches per RFC precedence.
  - Parameter binding algorithm — arbitrary param sets must produce valid urlencoded, multipart, and JSON bodies that round-trip correctly.
  - Rate limit tracking — arbitrary sequences of disclosure events must correctly enforce 24-hour sliding windows.
  - Consent store scoping — arbitrary grant/revoke sequences across session/origin/global scopes must maintain correct state.
  - Glob pattern matching — arbitrary patterns and field names must match identically to a reference implementation.

### TR-2: Integration Tests with Mock Server
- The client MUST have integration tests using the `MockAnmlServer` from the `testing` module.
- Integration tests MUST cover the full lifecycle: discover → fetch → inspect → disclose → execute → parse response → advance flow.
- Integration tests MUST cover error paths: 406 version mismatch, 401 auth required, integrity mismatch, transport insecurity rejection, trust denial, rate limiting.

### TR-3: Compile-Fail Tests
- The client MUST use `trybuild` for compile-fail tests verifying typestate enforcement.
- Tests MUST verify that calling `.execute()` without setting required params produces a compile error.

### TR-4: Test Coverage
- The crate SHOULD target >90% line coverage on non-trivial logic (disclosure, matching, params, flow, security).
- Coverage MUST be measurable via `cargo-tarpaulin` or `cargo-llvm-cov`.
