//! Well-known URI discovery: fetch `/.well-known/anml` and parse as ANML.
//!
//! Per RFC Section 6.4.1, a service MAY publish a discovery document at
//! `/.well-known/anml`. This module fetches that endpoint and extracts
//! a [`DiscoveryResult`] from the parsed ANML document.

use std::collections::HashMap;

use crate::discovery::{DiscoveryMechanism, DiscoveryResult};
use crate::error::AnmlClientError;

/// The well-known path for ANML service discovery.
const WELL_KNOWN_PATH: &str = "/.well-known/anml";

/// The expected ANML content type.
const ANML_CONTENT_TYPE: &str = "application/anml+xml";

/// Attempt discovery via the well-known URI.
///
/// Fetches `{origin}/.well-known/anml`, validates the content type,
/// parses the response as an ANML document, and extracts the endpoint
/// URL, version, and metadata.
pub async fn discover_well_known(
    http_client: &reqwest::Client,
    origin: &str,
) -> crate::Result<DiscoveryResult> {
    let url = format!(
        "{}{}",
        origin.trim_end_matches('/'),
        WELL_KNOWN_PATH
    );

    let response = http_client
        .get(&url)
        .header("Accept", ANML_CONTENT_TYPE)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        return Err(AnmlClientError::MalformedDocument {
            detail: format!(
                "well-known discovery at '{}' returned HTTP {}",
                url, status
            ),
        });
    }

    // Validate content type
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.starts_with(ANML_CONTENT_TYPE) {
        return Err(AnmlClientError::ContentTypeMismatch {
            expected: ANML_CONTENT_TYPE.into(),
            actual: content_type.into(),
        });
    }

    let body = response.text().await?;
    let doc = anml::parser::parse(&body)?;

    // Extract metadata from the parsed document
    let mut metadata = HashMap::new();

    if let Some(ref head) = doc.head {
        if let Some(ref title) = head.title {
            metadata.insert("title".to_string(), title.text.clone());
        }
        if let Some(ref metas) = head.meta {
            for meta in metas {
                metadata.insert(meta.name.clone(), meta.value.clone());
            }
        }
    }

    let version = doc.version.clone();

    Ok(DiscoveryResult {
        endpoint: url,
        version,
        metadata,
        mechanism: DiscoveryMechanism::WellKnown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_path_is_correct() {
        assert_eq!(WELL_KNOWN_PATH, "/.well-known/anml");
    }

    #[test]
    fn url_construction_no_trailing_slash() {
        let origin = "https://example.com";
        let url = format!("{}{}", origin.trim_end_matches('/'), WELL_KNOWN_PATH);
        assert_eq!(url, "https://example.com/.well-known/anml");
    }

    #[test]
    fn url_construction_with_trailing_slash() {
        let origin = "https://example.com/";
        let url = format!("{}{}", origin.trim_end_matches('/'), WELL_KNOWN_PATH);
        assert_eq!(url, "https://example.com/.well-known/anml");
    }

    #[test]
    fn url_construction_with_port() {
        let origin = "https://example.com:8443";
        let url = format!("{}{}", origin.trim_end_matches('/'), WELL_KNOWN_PATH);
        assert_eq!(url, "https://example.com:8443/.well-known/anml");
    }
}
