//! ANML service discovery (well-known URI, Link header, HTML link, DNS-SD).
//!
//! This module implements all four RFC-defined discovery mechanisms:
//!
//! 1. **Well-known URI** — fetch `/.well-known/anml` and parse as ANML.
//! 2. **HTTP Link header** — parse `Link` response headers for
//!    `rel="alternate" type="application/anml+xml"`.
//! 3. **HTML `<link>` element** (feature-gated `html-discovery`) — parse
//!    HTML for `<link rel="alternate" type="application/anml+xml">`.
//! 4. **DNS-SD** (feature-gated `dns-sd`) — resolve `_anml._tcp` SRV + TXT
//!    records via `hickory-resolver`.
//!
//! The [`discover`] orchestrator tries each mechanism in order and returns
//! the first successful [`DiscoveryResult`].

pub mod well_known;
pub mod link_header;

#[cfg(feature = "html-discovery")]
pub mod html_link;

#[cfg(feature = "dns-sd")]
pub mod dns_sd;

use std::collections::HashMap;

use crate::error::AnmlClientError;

// ---------------------------------------------------------------------------
// DiscoveryResult
// ---------------------------------------------------------------------------

/// The result of an ANML service discovery attempt.
///
/// Contains the endpoint URL where the ANML service can be reached,
/// the ANML version it advertises, and optional metadata extracted
/// from the discovery mechanism.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiscoveryResult {
    /// The ANML endpoint URL (e.g. `"https://api.example.com/.well-known/anml"`).
    pub endpoint: String,
    /// The ANML version advertised by the service, if known (e.g. `"1.0"`).
    pub version: Option<String>,
    /// Optional metadata extracted from the discovery mechanism.
    ///
    /// For well-known: may include document title, meta entries.
    /// For DNS-SD: may include TXT record key-value pairs.
    /// For Link/HTML: typically empty.
    pub metadata: HashMap<String, String>,
    /// Which discovery mechanism produced this result.
    pub mechanism: DiscoveryMechanism,
}

/// Which discovery mechanism produced a [`DiscoveryResult`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DiscoveryMechanism {
    /// `/.well-known/anml` endpoint.
    WellKnown,
    /// HTTP `Link` response header.
    LinkHeader,
    /// HTML `<link rel="alternate">` element.
    HtmlLink,
    /// DNS-SD `_anml._tcp` SRV + TXT records.
    DnsSd,
}

// ---------------------------------------------------------------------------
// discover() orchestrator
// ---------------------------------------------------------------------------

/// Try all discovery mechanisms in RFC-defined order and return the first
/// successful result.
///
/// Order: well-known → Link header → HTML link → DNS-SD.
///
/// The `http_client` is used for HTTP-based mechanisms (well-known, Link
/// header, HTML link). The `origin` should be a scheme+host (e.g.
/// `"https://example.com"`).
///
/// Feature-gated mechanisms (HTML link, DNS-SD) are only attempted when
/// the corresponding feature flag is enabled.
pub async fn discover(
    http_client: &reqwest::Client,
    origin: &str,
) -> crate::Result<DiscoveryResult> {
    // 1. Well-known URI
    match well_known::discover_well_known(http_client, origin).await {
        Ok(result) => return Ok(result),
        Err(e) => {
            tracing::debug!(mechanism = "well-known", error = %e, "well-known discovery failed");
        }
    }

    // 2. Link header — HEAD request to the origin root
    match link_header::discover_link_header(http_client, origin).await {
        Ok(result) => return Ok(result),
        Err(e) => {
            tracing::debug!(mechanism = "link-header", error = %e, "link header discovery failed");
        }
    }

    // 3. HTML link (feature-gated)
    #[cfg(feature = "html-discovery")]
    {
        match html_link::discover_html_link(http_client, origin).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                tracing::debug!(mechanism = "html-link", error = %e, "HTML link discovery failed");
            }
        }
    }

    // 4. DNS-SD (feature-gated)
    #[cfg(feature = "dns-sd")]
    {
        match dns_sd::discover_dns_sd(origin).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                tracing::debug!(mechanism = "dns-sd", error = %e, "DNS-SD discovery failed");
            }
        }
    }

    Err(AnmlClientError::MalformedDocument {
        detail: format!(
            "no ANML service discovered at origin '{}': all discovery mechanisms failed",
            origin
        ),
    })
}
