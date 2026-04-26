//! # anml-client
//!
//! An RFC-compliant ANML 1.0 client library for Rust.
//!
//! This crate provides a high-level async client for discovering, fetching,
//! and interacting with ANML services over HTTP(S). It implements the full
//! ANML 1.0 protocol as specified in `draft-jeskey-anml-00`, including:
//!
//! - **Discovery** — well-known URI, Link header, HTML link, DNS-SD
//! - **Disclosure evaluation** — the full 7-step RFC algorithm with consent,
//!   rate limiting, trust policy, and tokenization
//! - **Action execution** — parameter binding (urlencoded, multipart, JSON),
//!   SSRF protection, idempotency keys, auth integration
//! - **Flow navigation** — multi-step workflows with retry budgets and
//!   exponential backoff
//! - **SRI verification** — SHA-256/384/512 integrity checks on media
//! - **Pagination** — async iteration over `<nav>` next links
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use anml_client::prelude::*;
//!
//! # async fn example() -> anml_client::Result<()> {
//! let client = AnmlClient::builder()
//!     .base_url("https://api.example.com")
//!     .trust_policy(AllowListTrustPolicy::new()
//!         .allow_url("https://api.example.com"))
//!     .build()?;
//!
//! let doc = client.fetch("/service").await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Security
//!
//! The client enforces HTTPS by default, requires explicit trust policies
//! for all origins, runs mandatory disclosure evaluation before emitting
//! answers, blocks SSRF to private IPs, and verifies SRI digests on media.
//! All dependencies are pure Rust (TLS via `rustls`).
//!
//! ## Feature Flags
//!
//! | Flag | Default | Description |
//! |------|---------|-------------|
//! | `serde` | ✓ | Serde derives on state types |
//! | `dns-sd` | — | DNS-SD discovery |
//! | `html-discovery` | — | HTML `<link>` discovery |
//! | `cache` | — | In-memory TTL cache |
//! | `testing` | — | Mock server and test utilities |
//!
//! ## Prelude
//!
//! The [`prelude`] module provides a single glob import for the most
//! commonly used types: [`AnmlClient`](client::AnmlClient),
//! [`ClientConfig`](config::ClientConfig),
//! [`AnmlClientError`](error::AnmlClientError), [`Result`](error::Result),
//! [`TrustPolicy`](config::TrustPolicy),
//! [`AllowListTrustPolicy`](config::AllowListTrustPolicy), and key
//! `anml` crate types ([`AnmlDocument`](anml::types::document::AnmlDocument),
//! [`AnmlAction`](anml::types::elements::AnmlAction),
//! [`AnmlAsk`](anml::types::elements::AnmlAsk),
//! [`AnmlAnswer`](anml::types::elements::AnmlAnswer),
//! [`AnmlRefuse`](anml::types::elements::AnmlRefuse),
//! [`AnmlInform`](anml::types::elements::AnmlInform)).

pub mod error;
pub mod config;
pub mod client;
pub mod discovery;
pub mod disclosure;
pub mod action;
pub mod knowledge;
pub mod flow;
pub mod integrity;
pub mod pagination;
pub mod rights;
pub mod security;
pub mod audit;
pub mod middleware;
pub mod retry;
pub mod auth;
pub mod prelude;

#[cfg(feature = "cache")]
pub mod cache;

#[cfg(feature = "testing")]
pub mod testing;

pub use error::{AnmlClientError, Result};
