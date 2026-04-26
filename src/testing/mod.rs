//! Testing utilities: mock server, fixtures, and assertion helpers
//! (feature-gated: `testing`).
//!
//! This module provides tools for testing ANML client integrations:
//!
//! - [`MockAnmlServer`] — an in-process HTTP server with configurable
//!   ANML document responses and request recording.
//! - [`fixtures`] — pre-built ANML document XML strings for common patterns.
//! - [`assertions`] — helper functions for verifying request parameters,
//!   headers, and disclosure grants.

pub mod mock_server;
pub mod fixtures;
pub mod assertions;

pub use mock_server::{MockAnmlServer, MockAnmlServerBuilder, RecordedRequest};
