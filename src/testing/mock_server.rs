//! Mock ANML server for integration testing.
//!
//! `MockAnmlServer` wraps an in-process `axum` HTTP server that serves
//! configurable ANML document responses and records incoming requests
//! for assertion.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// RecordedRequest
// ---------------------------------------------------------------------------

/// A recorded incoming request to the mock server.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    /// The request method (GET, POST, etc.).
    pub method: String,
    /// The request path.
    pub path: String,
    /// The request headers.
    pub headers: HashMap<String, String>,
    /// The request body bytes.
    pub body: Vec<u8>,
}

// ---------------------------------------------------------------------------
// MockState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MockState {
    /// Path → ANML XML response body.
    documents: Arc<HashMap<String, String>>,
    /// Action id → ANML XML response body.
    action_responses: Arc<HashMap<String, String>>,
    /// Recorded requests.
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
    /// Custom status codes per path.
    status_overrides: Arc<HashMap<String, u16>>,
}

// ---------------------------------------------------------------------------
// MockAnmlServer
// ---------------------------------------------------------------------------

/// An in-process mock ANML server for testing.
///
/// Serves configurable ANML document responses and records all incoming
/// requests for later assertion.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() {
/// use anml_client::testing::MockAnmlServer;
///
/// let server = MockAnmlServer::builder()
///     .document("/service", "<anml version=\"1.0\"><head><title>Test</title></head></anml>")
///     .build()
///     .await;
///
/// let url = server.url();
/// // Use url with AnmlClient...
///
/// let requests = server.recorded_requests();
/// # }
/// ```
pub struct MockAnmlServer {
    addr: SocketAddr,
    state: MockState,
    _handle: tokio::task::JoinHandle<()>,
}

impl MockAnmlServer {
    /// Create a new builder.
    pub fn builder() -> MockAnmlServerBuilder {
        MockAnmlServerBuilder::default()
    }

    /// The base URL of the mock server (e.g. `http://127.0.0.1:PORT`).
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// All recorded requests.
    pub fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.state
            .recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Recorded requests filtered by path.
    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        self.recorded_requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }

    /// Assert that a request was received for the given path.
    pub fn assert_received(&self, path: &str) {
        let reqs = self.requests_to(path);
        assert!(
            !reqs.is_empty(),
            "expected request to '{}' but none received; got: {:?}",
            path,
            self.recorded_requests()
                .iter()
                .map(|r| &r.path)
                .collect::<Vec<_>>()
        );
    }

    /// Assert that no request was received for the given path.
    pub fn assert_not_received(&self, path: &str) {
        let reqs = self.requests_to(path);
        assert!(
            reqs.is_empty(),
            "expected no request to '{}' but got {}",
            path,
            reqs.len()
        );
    }

    /// Clear all recorded requests.
    pub fn clear_recorded(&self) {
        self.state
            .recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for [`MockAnmlServer`].
#[derive(Default)]
pub struct MockAnmlServerBuilder {
    documents: HashMap<String, String>,
    action_responses: HashMap<String, String>,
    status_overrides: HashMap<String, u16>,
}

impl MockAnmlServerBuilder {
    /// Register an ANML document response for a path.
    pub fn document(mut self, path: &str, xml: &str) -> Self {
        self.documents.insert(path.to_string(), xml.to_string());
        self
    }

    /// Register an action response for a path (POST handler).
    pub fn action_response(mut self, path: &str, xml: &str) -> Self {
        self.action_responses
            .insert(path.to_string(), xml.to_string());
        self
    }

    /// Override the status code for a specific path.
    pub fn status(mut self, path: &str, code: u16) -> Self {
        self.status_overrides.insert(path.to_string(), code);
        self
    }

    /// Build and start the mock server.
    pub async fn build(self) -> MockAnmlServer {
        let state = MockState {
            documents: Arc::new(self.documents),
            action_responses: Arc::new(self.action_responses),
            recorded: Arc::new(Mutex::new(Vec::new())),
            status_overrides: Arc::new(self.status_overrides),
        };

        let app = Router::new()
            .fallback(handle_request)
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        MockAnmlServer {
            addr,
            state,
            _handle: handle,
        }
    }
}

// ---------------------------------------------------------------------------
// Request handler
// ---------------------------------------------------------------------------

async fn handle_request(
    State(state): State<MockState>,
    req: Request<Body>,
) -> impl IntoResponse {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let body_bytes = axum::body::to_bytes(req.into_body(), 1_048_576)
        .await
        .unwrap_or_default()
        .to_vec();

    // Record the request
    {
        let mut recorded = state.recorded.lock().unwrap_or_else(|e| e.into_inner());
        recorded.push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            headers,
            body: body_bytes,
        });
    }

    // Check for status override
    if let Some(&code) = state.status_overrides.get(&path) {
        let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if let Some(doc) = state.documents.get(&path) {
            return (
                status,
                [(header::CONTENT_TYPE, "application/anml+xml")],
                doc.clone(),
            )
                .into_response();
        }
        return (status, "").into_response();
    }

    // Check for document or action response
    if method == "POST" {
        if let Some(resp) = state.action_responses.get(&path) {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/anml+xml")],
                resp.clone(),
            )
                .into_response();
        }
    }

    if let Some(doc) = state.documents.get(&path) {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/anml+xml")],
            doc.clone(),
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}
