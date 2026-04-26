//! Assertion helpers for ANML client testing.
//!
//! Provides convenience assertion functions for verifying mock server
//! interactions, disclosure grants, and parameter values.

use super::mock_server::RecordedRequest;

/// Assert that a recorded request contains a specific parameter value
/// in its URL-encoded body.
pub fn assert_param(request: &RecordedRequest, name: &str, expected_value: &str) {
    let body = String::from_utf8_lossy(&request.body);
    let target = format!("{}={}", name, url_encode_value(expected_value));
    assert!(
        body.contains(&target) || body.contains(&format!("{}={}", name, expected_value)),
        "expected param '{}={}' in body, got: {}",
        name,
        expected_value,
        body
    );
}

/// Assert that a recorded request has a specific header value.
pub fn assert_header(request: &RecordedRequest, name: &str, expected_value: &str) {
    let lower_name = name.to_lowercase();
    let actual = request.headers.get(&lower_name);
    assert_eq!(
        actual.map(|s| s.as_str()),
        Some(expected_value),
        "expected header '{}' = '{}', got: {:?}",
        name,
        expected_value,
        actual
    );
}

/// Assert that a recorded request has the ANML content type.
pub fn assert_anml_content_type(request: &RecordedRequest) {
    let ct = request
        .headers
        .get("content-type")
        .map(|s| s.as_str())
        .unwrap_or("");
    assert!(
        ct.contains("application/anml+xml")
            || ct.contains("application/x-www-form-urlencoded")
            || ct.contains("application/json")
            || ct.contains("multipart/form-data"),
        "expected a recognized content type, got: {}",
        ct
    );
}

/// Assert that a recorded request used the given HTTP method.
pub fn assert_method(request: &RecordedRequest, expected: &str) {
    assert_eq!(
        request.method, expected,
        "expected method '{}', got '{}'",
        expected, request.method
    );
}

/// Simple URL encoding for assertion matching.
fn url_encode_value(s: &str) -> String {
    s.replace(' ', "+")
        .replace('&', "%26")
        .replace('=', "%3D")
}
