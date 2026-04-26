//! # anml-client
//!
//! An RFC-compliant ANML 1.0 client library for Rust.
//!
//! This crate provides a high-level async client for discovering, fetching,
//! and interacting with ANML services over HTTP(S). It implements the full
//! ANML 1.0 protocol including disclosure evaluation, action execution,
//! flow navigation, and SRI verification.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use anml_client::prelude::*;
//!
//! # async fn example() -> anml_client::Result<()> {
//! let client = AnmlClient::builder()
//!     .base_url("https://api.example.com")
//!     .build()?;
//!
//! let doc = client.fetch("/service").await?;
//! # Ok(())
//! # }
//! ```

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
