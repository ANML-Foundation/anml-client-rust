//! HTTP Link header discovery: parse `Link` response headers for ANML endpoints.
//!
//! Per RFC Section 6.4.2, a service MAY advertise ANML support via a `Link`
//! header with `rel="alternate"` and `type="application/anml+xml"`.
//!
//! This module sends a HEAD request to the origin and inspects the `Link`
//! header(s) in the response.

use std::collections::HashMap;

use crate::discovery::{DiscoveryMechanism, DiscoveryResult};
use crate::error::AnmlClientError;

/// The expected ANML content type for Link header matching.
const ANML_CONTENT_TYPE: &str = "application/anml+xml";

/// Attempt discovery via HTTP Link header.
///
/// Sends a HEAD request to `{origin}/` and parses the `Link` response
/// header(s) looking for `rel="alternate" type="application/anml+xml"`.
pub async fn discover_link_header(
    http_client: &reqwest::Client,
    origin: &str,
) -> crate::Result<DiscoveryResult> {
    let url = format!("{}/", origin.trim_end_matches('/'));

    let response = http_client
        .head(&url)
        .send()
        .await?;

    // We don't require a success status — the Link header may be present
    // even on redirects or other responses.

    // Collect all Link header values
    let link_values: Vec<&str> = response
        .headers()
        .get_all("link")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();

    if link_values.is_empty() {
        return Err(AnmlClientError::MalformedDocument {
            detail: format!("no Link header found in response from '{}'", url),
        });
    }

    // Parse each Link header value
    for header_value in &link_values {
        if let Some(result) = parse_link_header(header_value, origin) {
            return Ok(result);
        }
    }

    Err(AnmlClientError::MalformedDocument {
        detail: format!(
            "no ANML alternate link found in Link headers from '{}'",
            url
        ),
    })
}

/// Parse a single `Link` header value and extract an ANML alternate link.
///
/// A Link header may contain multiple comma-separated link entries.
/// Returns the first entry matching `rel="alternate" type="application/anml+xml"`.
pub(crate) fn parse_link_header(
    header: &str,
    origin: &str,
) -> Option<DiscoveryResult> {
    for link_str in split_links(header) {
        // Extract href from angle brackets
        let href_start = link_str.find('<')?;
        let href_end = link_str[href_start..].find('>')? + href_start;
        let href = &link_str[href_start + 1..href_end];

        let params_str = &link_str[href_end + 1..];
        let params = parse_params(params_str);

        let rel = params.get("rel");
        let link_type = params.get("type");

        if rel.map(String::as_str) == Some("alternate")
            && link_type.map(String::as_str) == Some(ANML_CONTENT_TYPE)
        {
            // Resolve relative URLs against the origin
            let endpoint = if href.starts_with("http://") || href.starts_with("https://") {
                href.to_string()
            } else {
                format!("{}{}", origin.trim_end_matches('/'), href)
            };

            return Some(DiscoveryResult {
                endpoint,
                version: None,
                metadata: HashMap::new(),
                mechanism: DiscoveryMechanism::LinkHeader,
            });
        }
    }

    None
}

/// Split a Link header value into individual link entries.
///
/// Handles commas that separate multiple links while respecting
/// angle-bracket-enclosed URIs.
fn split_links(header: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut start = 0;
    let mut in_angle = false;

    for (i, ch) in header.char_indices() {
        if ch == '<' {
            in_angle = true;
        }
        if ch == '>' {
            in_angle = false;
        }
        if ch == ',' && !in_angle {
            let segment = header[start..i].trim();
            if !segment.is_empty() {
                results.push(segment);
            }
            start = i + 1;
        }
    }

    let last = header[start..].trim();
    if !last.is_empty() {
        results.push(last);
    }

    results
}

/// Parse semicolon-separated parameters from a Link header entry.
///
/// Strips surrounding quotes from values.
fn parse_params(param_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    for part in param_str.split(';') {
        let part = part.trim();
        if let Some(eq_idx) = part.find('=') {
            let key = part[..eq_idx].trim().to_lowercase();
            let mut value = part[eq_idx + 1..].trim().to_string();
            // Strip surrounding quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            params.insert(key, value);
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_anml_alternate_link() {
        let header =
            r#"<https://example.com/api>; rel="alternate"; type="application/anml+xml""#;
        let result = parse_link_header(header, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api");
        assert_eq!(result.mechanism, DiscoveryMechanism::LinkHeader);
    }

    #[test]
    fn parse_relative_href() {
        let header = r#"</api/anml>; rel="alternate"; type="application/anml+xml""#;
        let result = parse_link_header(header, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api/anml");
    }

    #[test]
    fn parse_no_anml_link() {
        let header =
            r#"<https://example.com/feed>; rel="alternate"; type="application/rss+xml""#;
        assert!(parse_link_header(header, "https://example.com").is_none());
    }

    #[test]
    fn parse_wrong_rel() {
        let header =
            r#"<https://example.com/api>; rel="self"; type="application/anml+xml""#;
        assert!(parse_link_header(header, "https://example.com").is_none());
    }

    #[test]
    fn parse_multiple_links_finds_anml() {
        let header = r#"<https://example.com/feed>; rel="alternate"; type="application/rss+xml", <https://example.com/anml>; rel="alternate"; type="application/anml+xml""#;
        let result = parse_link_header(header, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/anml");
    }

    #[test]
    fn parse_empty_header() {
        assert!(parse_link_header("", "https://example.com").is_none());
    }

    #[test]
    fn parse_unquoted_params() {
        let header = "<https://example.com/api>; rel=alternate; type=application/anml+xml";
        let result = parse_link_header(header, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api");
    }

    #[test]
    fn split_links_single() {
        let links = split_links(r#"<https://example.com>; rel="self""#);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn split_links_multiple() {
        let links = split_links(
            r#"<https://a.com>; rel="self", <https://b.com>; rel="alternate""#,
        );
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn split_links_comma_in_url() {
        // Commas inside angle brackets should not split
        let links = split_links(r#"<https://example.com/a,b>; rel="self""#);
        assert_eq!(links.len(), 1);
        assert!(links[0].contains("a,b"));
    }

    #[test]
    fn parse_params_basic() {
        let params = parse_params(r#"; rel="alternate"; type="application/anml+xml""#);
        assert_eq!(params.get("rel").map(String::as_str), Some("alternate"));
        assert_eq!(
            params.get("type").map(String::as_str),
            Some("application/anml+xml")
        );
    }

    #[test]
    fn parse_params_strips_quotes() {
        let params = parse_params(r#"; key="value""#);
        assert_eq!(params.get("key").map(String::as_str), Some("value"));
    }

    #[test]
    fn parse_params_case_insensitive_keys() {
        let params = parse_params(r#"; REL="alternate""#);
        assert_eq!(params.get("rel").map(String::as_str), Some("alternate"));
    }
}
