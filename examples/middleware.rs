//! HTTP middleware for request logging and custom headers.
//!
//! ```no_run
//! cargo run --example middleware
//! ```

use anml_client::prelude::*;
use anml_client::config::HttpMiddleware;
use async_trait::async_trait;

/// Middleware that logs all outgoing requests and incoming responses.
struct RequestLogger;

#[async_trait]
impl HttpMiddleware for RequestLogger {
    async fn on_request(
        &self,
        req: reqwest::Request,
    ) -> anml_client::Result<reqwest::Request> {
        println!(
            "→ {} {}",
            req.method(),
            req.url()
        );
        Ok(req)
    }

    async fn on_response(
        &self,
        resp: reqwest::Response,
    ) -> anml_client::Result<reqwest::Response> {
        println!(
            "← {} {}",
            resp.status(),
            resp.url()
        );
        Ok(resp)
    }
}

/// Middleware that adds custom headers to every request.
struct CustomHeaders {
    headers: Vec<(String, String)>,
}

impl CustomHeaders {
    fn new() -> Self {
        Self {
            headers: vec![
                ("X-Client-Version".to_string(), "0.1.0".to_string()),
                ("X-Request-Source".to_string(), "anml-client".to_string()),
            ],
        }
    }
}

#[async_trait]
impl HttpMiddleware for CustomHeaders {
    async fn on_request(
        &self,
        mut req: reqwest::Request,
    ) -> anml_client::Result<reqwest::Request> {
        for (name, value) in &self.headers {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                req.headers_mut().insert(n, v);
            }
        }
        Ok(req)
    }

    async fn on_response(
        &self,
        resp: reqwest::Response,
    ) -> anml_client::Result<reqwest::Response> {
        Ok(resp)
    }
}

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    // Build client with middleware chain
    // Middleware is applied in registration order for requests (first→last)
    // and reverse order for responses (last→first)
    let client = AnmlClient::builder()
        .base_url("https://api.example.com")
        .trust_policy(
            AllowListTrustPolicy::new().allow_url("https://api.example.com"),
        )
        .middleware(RequestLogger)
        .middleware(CustomHeaders::new())
        .build()?;

    println!("Client configured with logging and custom header middleware");

    match client.fetch("/service").await {
        Ok(doc) => {
            let title = doc
                .head
                .as_ref()
                .and_then(|h| h.title.as_ref())
                .map(|t| t.text.as_str())
                .unwrap_or("(untitled)");
            println!("Fetched: {title}");
        }
        Err(e) => {
            println!("Fetch failed (expected without a real server): {e}");
        }
    }

    Ok(())
}
