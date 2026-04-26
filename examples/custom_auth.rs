//! Custom AuthProvider for OAuth2 token refresh.
//!
//! ```no_run
//! cargo run --example custom_auth
//! ```

use std::sync::Mutex;

use anml_client::prelude::*;
use anml_client::config::{AuthProvider, AuthRefreshResult, Origin};
use async_trait::async_trait;

/// An OAuth2 auth provider that supports token refresh.
struct OAuth2Provider {
    access_token: Mutex<String>,
    #[allow(dead_code)]
    refresh_token: String,
    token_endpoint: String,
}

impl OAuth2Provider {
    fn new(access_token: &str, refresh_token: &str, token_endpoint: &str) -> Self {
        Self {
            access_token: Mutex::new(access_token.to_string()),
            refresh_token: refresh_token.to_string(),
            token_endpoint: token_endpoint.to_string(),
        }
    }
}

#[async_trait]
impl AuthProvider for OAuth2Provider {
    async fn credentials(&self, _origin: &Origin) -> Option<Vec<(String, String)>> {
        let token = self.access_token.lock().unwrap().clone();
        Some(vec![(
            "Authorization".to_string(),
            format!("Bearer {}", token),
        )])
    }

    async fn on_unauthorized(&self, _origin: &Origin) -> AuthRefreshResult {
        println!(
            "Token expired, refreshing from {}...",
            self.token_endpoint
        );

        // In a real implementation, you'd make an HTTP request to the
        // token endpoint with the refresh token:
        //
        // let response = reqwest::Client::new()
        //     .post(&self.token_endpoint)
        //     .form(&[
        //         ("grant_type", "refresh_token"),
        //         ("refresh_token", &self.refresh_token),
        //     ])
        //     .send()
        //     .await;

        // Simulate a successful refresh
        let new_token = format!("refreshed-token-{}", rand::random::<u32>());
        *self.access_token.lock().unwrap() = new_token;
        println!("Token refreshed successfully");

        AuthRefreshResult::Refreshed
    }
}

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    let auth = OAuth2Provider::new(
        "initial-access-token",
        "my-refresh-token",
        "https://auth.example.com/token",
    );

    let client = AnmlClient::builder()
        .base_url("https://api.example.com")
        .trust_policy(
            AllowListTrustPolicy::new().allow_url("https://api.example.com"),
        )
        .auth_provider(auth)
        .build()?;

    println!("Client configured with OAuth2 auth provider");
    println!("On 401, the provider will automatically refresh the token");

    match client.fetch("/protected-resource").await {
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
