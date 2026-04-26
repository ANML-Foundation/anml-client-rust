//! HTML `<link>` element discovery (feature-gated: `html-discovery`).
//!
//! Per RFC Section 6.4.3, a service MAY include a
//! `<link rel="alternate" type="application/anml+xml">` element in HTML
//! documents. This module fetches the origin's root page and parses the
//! HTML to find such a link.
//!
//! Requires the `html-discovery` feature flag, which adds `scraper` as a
//! dependency.

use std::collections::HashMap;

use scraper::{Html, Selector};

use crate::discovery::{DiscoveryMechanism, DiscoveryResult};
use crate::error::AnmlClientError;

/// Attempt discovery via HTML `<link>` elements.
///
/// Fetches `{origin}/` with an HTML Accept header, parses the response as
/// HTML, and looks for `<link rel="alternate" type="application/anml+xml" href="...">`.
pub async fn discover_html_link(
    http_client: &reqwest::Client,
    origin: &str,
) -> crate::Result<DiscoveryResult> {
    let url = format!("{}/", origin.trim_end_matches('/'));

    let response = http_client
        .get(&url)
        .header("Accept", "text/html")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(AnmlClientError::MalformedDocument {
            detail: format!(
                "HTML discovery at '{}' returned HTTP {}",
                url,
                response.status()
            ),
        });
    }

    let body = response.text().await?;
    parse_html_for_anml_link(&body, origin)
}

/// Parse an HTML document and extract the first ANML alternate link.
///
/// Looks for `<link rel="alternate" type="application/anml+xml" href="...">`.
pub(crate) fn parse_html_for_anml_link(
    html: &str,
    origin: &str,
) -> crate::Result<DiscoveryResult> {
    let document = Html::parse_document(html);

    // Select <link> elements with rel="alternate" and type="application/anml+xml"
    let selector = Selector::parse("link[rel=\"alternate\"][type=\"application/anml+xml\"]")
        .map_err(|_| AnmlClientError::MalformedDocument {
            detail: "failed to compile HTML selector".into(),
        })?;

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            // Resolve relative URLs against the origin
            let endpoint = if href.starts_with("http://") || href.starts_with("https://") {
                href.to_string()
            } else {
                format!("{}{}", origin.trim_end_matches('/'), href)
            };

            return Ok(DiscoveryResult {
                endpoint,
                version: None,
                metadata: HashMap::new(),
                mechanism: DiscoveryMechanism::HtmlLink,
            });
        }
    }

    Err(AnmlClientError::MalformedDocument {
        detail: format!(
            "no <link rel=\"alternate\" type=\"application/anml+xml\"> found in HTML from '{}'",
            origin
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_html_with_anml_link() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Example</title>
                <link rel="alternate" type="application/anml+xml" href="https://example.com/api" />
            </head>
            <body></body>
            </html>
        "#;
        let result = parse_html_for_anml_link(html, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api");
        assert_eq!(result.mechanism, DiscoveryMechanism::HtmlLink);
    }

    #[test]
    fn parse_html_with_relative_href() {
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/anml+xml" href="/api/anml" />
            </head>
            </html>
        "#;
        let result = parse_html_for_anml_link(html, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api/anml");
    }

    #[test]
    fn parse_html_no_anml_link() {
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/rss+xml" href="/feed" />
            </head>
            </html>
        "#;
        let result = parse_html_for_anml_link(html, "https://example.com");
        assert!(result.is_err());
    }

    #[test]
    fn parse_html_empty() {
        let result = parse_html_for_anml_link("", "https://example.com");
        assert!(result.is_err());
    }

    #[test]
    fn parse_html_multiple_links_picks_first() {
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/anml+xml" href="/first" />
                <link rel="alternate" type="application/anml+xml" href="/second" />
            </head>
            </html>
        "#;
        let result = parse_html_for_anml_link(html, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/first");
    }

    #[test]
    fn parse_html_link_without_href_skipped() {
        let html = r#"
            <html>
            <head>
                <link rel="alternate" type="application/anml+xml" />
                <link rel="alternate" type="application/anml+xml" href="/api" />
            </head>
            </html>
        "#;
        let result = parse_html_for_anml_link(html, "https://example.com").unwrap();
        assert_eq!(result.endpoint, "https://example.com/api");
    }
}
