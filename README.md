# anml-client

An RFC-compliant ANML 1.0 client library for Rust.

`anml-client` provides a high-level async client for discovering, fetching, and
interacting with ANML services over HTTP(S). It implements the full ANML 1.0
protocol as specified in `draft-jeskey-anml-00`, including disclosure evaluation,
action execution, flow navigation, SRI verification, and consent management.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
anml-client = { git = "https://github.com/Life-Savor-AI/anml-client-rust.git" }
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `serde` | ✓ | Serde derives on client state types |
| `dns-sd` | — | DNS-SD discovery via `hickory-resolver` |
| `html-discovery` | — | HTML `<link>` discovery via `scraper` |
| `cache` | — | In-memory TTL document cache |
| `testing` | — | Mock server, fixtures, assertion helpers |

## Quick Start

```rust,no_run
use anml_client::prelude::*;

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    let client = AnmlClient::builder()
        .base_url("https://api.example.com")
        .trust_policy(AllowListTrustPolicy::new()
            .allow_url("https://api.example.com"))
        .build()?;

    // Fetch an ANML document
    let doc = client.fetch("/service").await?;

    // Inspect asks
    if let Some(ref knowledge) = doc.knowledge {
        if let Some(ref asks) = knowledge.asks {
            for ask in asks {
                println!("Service asks for: {}", ask.field);
            }
        }
    }

    Ok(())
}
```

## Usage

### Discovery

```rust,no_run
# async fn example(client: anml_client::client::AnmlClient) -> anml_client::Result<()> {
let result = client.discover("https://api.example.com").await?;
println!("ANML endpoint: {}", result.endpoint);
# Ok(())
# }
```

### Action Execution

```rust,no_run
# use anml_client::prelude::*;
# fn example(doc: AnmlDocument) -> anml_client::Result<()> {
let builder = ActionRequestBuilder::new(&doc, "submit-airline", "https://api.example.com", None)?
    .param("airline", "Delta")
    .param_bool("flexible", true);
// builder.execute(&ctx).await?;
# Ok(())
# }
```

### Flow Navigation

```rust,no_run
# use anml_client::flow::FlowNavigator;
# use anml::types::document::AnmlDocument;
# fn example(doc: AnmlDocument) -> anml_client::Result<()> {
let nav = FlowNavigator::from_document(&doc)?;
println!("Current step: {:?}", nav.current().map(|s| &s.id));
println!("Progress: {nav}");
# Ok(())
# }
```

### Pagination

```rust,no_run
# async fn example(client: anml_client::client::AnmlClient, doc: anml::types::document::AnmlDocument) -> anml_client::Result<()> {
let mut pages = client.paginate(doc, Some("flights"));
while let Some(page) = pages.next_page().await {
    let _doc = page?;
    // Process items...
}
# Ok(())
# }
```

## Security Model

The client enforces a defense-in-depth security model:

- **HTTPS by default** — plaintext HTTP is rejected unless explicitly opted in.
  Documents fetched over HTTP that contain `<constraints>`, `<interact>`, or
  `<ask requires="explicit">` are always rejected.
- **Trust policy** — every origin must be explicitly trusted via `TrustPolicy`.
  The default `DenyAllTrustPolicy` blocks all origins.
- **Disclosure evaluation** — the full 7-step RFC disclosure algorithm runs
  before any `<answer>` is emitted, including consent checks, rate limiting,
  trust verification, and value validation.
- **SSRF protection** — action endpoints resolving to private/loopback IPs
  are blocked (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, ::1).
- **SRI verification** — media resources with `integrity` attributes are
  verified against SHA-256/384/512 digests before use.
- **Resource limits** — configurable but not disableable limits on document
  size, depth, element count, and parse time.
- **Action budgets** — per-document limits on request count, distinct origins,
  and media bandwidth.
- **Tokenization** — HMAC-SHA256 tokenization for fields with `tokenize="true"`.
- **Audit logging** — every disclosure is recorded with field, origin,
  timestamp, consent basis, and governing rule.
- **Pure Rust** — all dependencies are Rust crates. TLS via `rustls`, no
  system OpenSSL.

## Examples

See the `examples/` directory:

- `basic_fetch.rs` — discover, fetch, inspect
- `action_execution.rs` — disclosure, execute, handle response
- `multi_step_flow.rs` — flow navigation with error recovery
- `custom_auth.rs` — OAuth2 token refresh via `AuthProvider`
- `middleware.rs` — request logging and custom headers

## MSRV

Rust 1.80 or later.

## License

ISC
