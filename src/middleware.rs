//! HTTP middleware trait and composable interceptor chain.
//!
//! Middleware is applied in registration order for requests (first → last)
//! and reverse order for responses (last → first). Register middleware via
//! [`AnmlClient::builder().middleware(mw)`](crate::client::AnmlClientBuilder::middleware).
//!
//! # Example
//!
//! ```rust,no_run
//! use anml_client::middleware::LoggingMiddleware;
//! use anml_client::client::AnmlClient;
//!
//! let client = AnmlClient::builder()
//!     .base_url("https://api.example.com")
//!     .middleware(LoggingMiddleware)
//!     .build()
//!     .expect("build client");
//! ```

use async_trait::async_trait;
use tracing::{debug, info};

use crate::config::HttpMiddleware;

// ---------------------------------------------------------------------------
// LoggingMiddleware — example middleware that logs requests and responses
// ---------------------------------------------------------------------------

/// Example middleware that logs outgoing requests and incoming responses
/// using the `tracing` crate.
///
/// Logs at `INFO` level: method, URL, and response status code.
/// Does NOT log request/response bodies or sensitive headers.
#[derive(Clone, Debug, Default)]
pub struct LoggingMiddleware;

#[async_trait]
impl HttpMiddleware for LoggingMiddleware {
    async fn on_request(
        &self,
        req: reqwest::Request,
    ) -> crate::Result<reqwest::Request> {
        info!(
            method = %req.method(),
            url = %req.url(),
            "outgoing request"
        );
        debug!(
            headers = ?req.headers().keys().map(|k| k.as_str()).collect::<Vec<_>>(),
            "request header names (values redacted)"
        );
        Ok(req)
    }

    async fn on_response(
        &self,
        resp: reqwest::Response,
    ) -> crate::Result<reqwest::Response> {
        info!(
            status = %resp.status(),
            url = %resp.url(),
            "incoming response"
        );
        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_middleware_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LoggingMiddleware>();
    }

    #[test]
    fn logging_middleware_debug() {
        let mw = LoggingMiddleware;
        let debug_str = format!("{:?}", mw);
        assert_eq!(debug_str, "LoggingMiddleware");
    }
}
