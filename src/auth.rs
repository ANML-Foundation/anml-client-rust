//! Authentication provider trait and built-in implementations.
//!
//! This module provides concrete [`AuthProvider`](crate::config::AuthProvider)
//! implementations for common authentication patterns:
//!
//! - [`BearerTokenProvider`] — static `Authorization: Bearer <token>` header
//! - [`ApiKeyProvider`] — API key via header or query parameter
//!
//! For custom authentication (e.g., OAuth2 refresh), implement the
//! [`AuthProvider`](crate::config::AuthProvider) trait directly.

use async_trait::async_trait;

use crate::config::{AuthProvider, AuthRefreshResult, Origin};

// ---------------------------------------------------------------------------
// ApiKeyLocation
// ---------------------------------------------------------------------------

/// Where to place the API key in the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiKeyLocation {
    /// Send the key as a request header (e.g., `X-API-Key: <key>`).
    Header {
        /// The header name (e.g., `"X-API-Key"`).
        name: String,
    },
    /// Append the key as a query parameter (e.g., `?api_key=<key>`).
    ///
    /// Note: query-based API keys are sent via a custom header
    /// `X-ANML-ApiKey-Query` since `AuthProvider` returns headers.
    /// The middleware or HTTP layer should translate this to a query param.
    Query {
        /// The query parameter name (e.g., `"api_key"`).
        name: String,
    },
}

// ---------------------------------------------------------------------------
// BearerTokenProvider
// ---------------------------------------------------------------------------

/// A simple [`AuthProvider`] that attaches a static bearer token.
///
/// Sends `Authorization: Bearer <token>` on every request. Does not
/// support token refresh — `on_unauthorized` always returns `Failed`.
///
/// # Example
///
/// ```rust,no_run
/// use anml_client::auth::BearerTokenProvider;
/// use anml_client::client::AnmlClient;
///
/// let client = AnmlClient::builder()
///     .base_url("https://api.example.com")
///     .auth_provider(BearerTokenProvider::new("my-secret-token"))
///     .build()
///     .expect("build client");
/// ```
#[derive(Clone, Debug)]
pub struct BearerTokenProvider {
    token: String,
}

impl BearerTokenProvider {
    /// Create a new bearer token provider with the given token.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl AuthProvider for BearerTokenProvider {
    async fn credentials(&self, _origin: &Origin) -> Option<Vec<(String, String)>> {
        Some(vec![(
            "Authorization".to_string(),
            format!("Bearer {}", self.token),
        )])
    }

    async fn on_unauthorized(&self, _origin: &Origin) -> AuthRefreshResult {
        // Static token — no refresh capability.
        AuthRefreshResult::Failed
    }
}

// ---------------------------------------------------------------------------
// ApiKeyProvider
// ---------------------------------------------------------------------------

/// A simple [`AuthProvider`] that attaches an API key via header or query.
///
/// # Header mode
///
/// Sends the key as a custom header (e.g., `X-API-Key: <key>`).
///
/// # Query mode
///
/// Sends the key as a header `X-ANML-ApiKey-Query: <name>=<key>` which
/// middleware can translate to a query parameter. This avoids modifying
/// the URL directly in the auth provider.
///
/// # Example
///
/// ```rust,no_run
/// use anml_client::auth::{ApiKeyProvider, ApiKeyLocation};
/// use anml_client::client::AnmlClient;
///
/// let client = AnmlClient::builder()
///     .base_url("https://api.example.com")
///     .auth_provider(ApiKeyProvider::header("X-API-Key", "my-api-key"))
///     .build()
///     .expect("build client");
/// ```
#[derive(Clone, Debug)]
pub struct ApiKeyProvider {
    key: String,
    location: ApiKeyLocation,
}

impl ApiKeyProvider {
    /// Create an API key provider that sends the key as a request header.
    pub fn header(header_name: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            location: ApiKeyLocation::Header {
                name: header_name.into(),
            },
        }
    }

    /// Create an API key provider that sends the key as a query parameter.
    pub fn query(param_name: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            location: ApiKeyLocation::Query {
                name: param_name.into(),
            },
        }
    }
}

#[async_trait]
impl AuthProvider for ApiKeyProvider {
    async fn credentials(&self, _origin: &Origin) -> Option<Vec<(String, String)>> {
        match &self.location {
            ApiKeyLocation::Header { name } => {
                Some(vec![(name.clone(), self.key.clone())])
            }
            ApiKeyLocation::Query { name } => {
                // Encode as a special header for middleware translation
                Some(vec![(
                    "X-ANML-ApiKey-Query".to_string(),
                    format!("{}={}", name, self.key),
                )])
            }
        }
    }

    async fn on_unauthorized(&self, _origin: &Origin) -> AuthRefreshResult {
        // Static API key — no refresh capability.
        AuthRefreshResult::Failed
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "api.example.com".into(),
            port: None,
        }
    }

    #[tokio::test]
    async fn bearer_token_provides_auth_header() {
        let provider = BearerTokenProvider::new("test-token-123");
        let origin = test_origin();
        let creds = provider.credentials(&origin).await.unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].0, "Authorization");
        assert_eq!(creds[0].1, "Bearer test-token-123");
    }

    #[tokio::test]
    async fn bearer_token_on_unauthorized_fails() {
        let provider = BearerTokenProvider::new("token");
        let origin = test_origin();
        let result = provider.on_unauthorized(&origin).await;
        assert_eq!(result, AuthRefreshResult::Failed);
    }

    #[tokio::test]
    async fn api_key_header_mode() {
        let provider = ApiKeyProvider::header("X-API-Key", "my-key");
        let origin = test_origin();
        let creds = provider.credentials(&origin).await.unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].0, "X-API-Key");
        assert_eq!(creds[0].1, "my-key");
    }

    #[tokio::test]
    async fn api_key_query_mode() {
        let provider = ApiKeyProvider::query("api_key", "my-key");
        let origin = test_origin();
        let creds = provider.credentials(&origin).await.unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].0, "X-ANML-ApiKey-Query");
        assert_eq!(creds[0].1, "api_key=my-key");
    }

    #[tokio::test]
    async fn api_key_on_unauthorized_fails() {
        let provider = ApiKeyProvider::header("X-API-Key", "key");
        let origin = test_origin();
        let result = provider.on_unauthorized(&origin).await;
        assert_eq!(result, AuthRefreshResult::Failed);
    }

    #[test]
    fn bearer_token_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BearerTokenProvider>();
    }

    #[test]
    fn api_key_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ApiKeyProvider>();
    }

    #[test]
    fn bearer_token_debug() {
        let provider = BearerTokenProvider::new("secret");
        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("BearerTokenProvider"));
    }

    #[test]
    fn api_key_debug() {
        let provider = ApiKeyProvider::header("X-Key", "val");
        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("ApiKeyProvider"));
    }
}
