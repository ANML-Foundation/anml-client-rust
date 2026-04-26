//! Subresource Integrity (SRI) verification for external media.
//!
//! Implements RFC Section 10.5: verifies that fetched media bytes match
//! the `integrity` attribute on `<img>`, `<audio>`, `<video>`, and `<link>`
//! elements. Supports `sha256`, `sha384`, and `sha512` algorithms.
//!
//! When `inference="required"`, integrity MUST be present and verified
//! before the resource bytes are used. Missing integrity on such elements
//! is treated as a malformed document.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use base64::Engine;
use sha2::{Digest, Sha256, Sha384, Sha512};
use tracing::{error, warn};

use crate::config::ActionBudget;
use crate::error::AnmlClientError;

// ---------------------------------------------------------------------------
// SRI algorithm types
// ---------------------------------------------------------------------------

/// Supported SRI hash algorithms, ordered by strength.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SriAlgorithm {
    /// SHA-256 (floor algorithm — MUST be supported).
    Sha256,
    /// SHA-384 (MUST be supported).
    Sha384,
    /// SHA-512 (RECOMMENDED).
    Sha512,
}

impl SriAlgorithm {
    /// Parse an algorithm prefix from an integrity token (e.g. `"sha256"`).
    fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "sha256" => Some(Self::Sha256),
            "sha384" => Some(Self::Sha384),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    /// Compute the digest of `data` using this algorithm, returning base64.
    fn digest(&self, data: &[u8]) -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        match self {
            Self::Sha256 => engine.encode(Sha256::digest(data)),
            Self::Sha384 => engine.encode(Sha384::digest(data)),
            Self::Sha512 => engine.encode(Sha512::digest(data)),
        }
    }
}

/// A parsed integrity token: algorithm + base64-encoded digest.
#[derive(Clone, Debug)]
pub struct IntegrityToken {
    /// The hash algorithm.
    pub algorithm: SriAlgorithm,
    /// The expected base64-encoded digest.
    pub expected_digest: String,
}

// ---------------------------------------------------------------------------
// Parse integrity attribute
// ---------------------------------------------------------------------------

/// Parse a space-separated `integrity` attribute value into tokens.
///
/// Each token has the form `algorithm-base64value`. Unknown algorithms
/// are silently skipped (per W3C SRI spec).
pub fn parse_integrity_attr(attr: &str) -> Vec<IntegrityToken> {
    attr.split_whitespace()
        .filter_map(|token| {
            let (prefix, digest) = token.split_once('-')?;
            let algo = SriAlgorithm::from_prefix(prefix)?;
            Some(IntegrityToken {
                algorithm: algo,
                expected_digest: digest.to_string(),
            })
        })
        .collect()
}

/// Select the strongest algorithm from a set of integrity tokens.
///
/// When multiple tokens are present, the agent MUST select the token
/// whose algorithm it considers strongest among those it supports.
fn select_strongest(tokens: &[IntegrityToken]) -> Option<&IntegrityToken> {
    tokens.iter().max_by_key(|t| t.algorithm)
}

// ---------------------------------------------------------------------------
// Verify integrity
// ---------------------------------------------------------------------------

/// Verify that `bytes` match the integrity attribute value.
///
/// Parses the integrity attribute, selects the strongest algorithm,
/// computes the digest, and compares. Returns `Ok(())` on match or
/// an `IntegrityMismatch` error on failure.
pub fn verify_integrity(
    bytes: &[u8],
    integrity_attr: &str,
    element_name: &str,
) -> crate::Result<()> {
    let tokens = parse_integrity_attr(integrity_attr);
    if tokens.is_empty() {
        return Err(AnmlClientError::MalformedDocument {
            detail: format!(
                "<{}> has integrity attribute but no recognized algorithm tokens",
                element_name
            ),
        });
    }

    let strongest = select_strongest(&tokens).unwrap(); // safe: tokens is non-empty
    let observed = strongest.algorithm.digest(bytes);

    if observed == strongest.expected_digest {
        Ok(())
    } else {
        let expected_str = format!("{:?}-{}", strongest.algorithm, strongest.expected_digest)
            .to_lowercase();
        let observed_str = format!("{:?}-{}", strongest.algorithm, observed).to_lowercase();

        // Log security event
        error!(
            element = element_name,
            expected = %expected_str,
            observed = %observed_str,
            "SRI integrity mismatch — resource bytes do not match expected digest"
        );

        Err(AnmlClientError::IntegrityMismatch {
            element: element_name.to_string(),
            expected: expected_str,
            observed: observed_str,
        })
    }
}


// ---------------------------------------------------------------------------
// Media budget tracker
// ---------------------------------------------------------------------------

/// Tracks media fetch count and bandwidth against an `ActionBudget`.
#[derive(Debug)]
pub struct MediaBudgetTracker {
    fetch_count: AtomicU32,
    bandwidth_bytes: AtomicU64,
    max_fetches: u32,
    max_bandwidth: u64,
}

impl MediaBudgetTracker {
    /// Create a new tracker from an `ActionBudget`.
    pub fn new(budget: &ActionBudget) -> Self {
        Self {
            fetch_count: AtomicU32::new(0),
            bandwidth_bytes: AtomicU64::new(0),
            max_fetches: budget.max_media_fetches,
            max_bandwidth: budget.max_media_bandwidth,
        }
    }

    /// Record a media fetch of `size` bytes. Returns an error if the
    /// budget would be exceeded.
    pub fn record_fetch(&self, size: u64) -> crate::Result<()> {
        let count = self.fetch_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count > self.max_fetches {
            return Err(AnmlClientError::ResourceLimitExceeded {
                limit: "max_media_fetches".into(),
                value: count as u64,
                max: self.max_fetches as u64,
            });
        }

        let total = self.bandwidth_bytes.fetch_add(size, Ordering::Relaxed) + size;
        if total > self.max_bandwidth {
            return Err(AnmlClientError::ResourceLimitExceeded {
                limit: "max_media_bandwidth".into(),
                value: total,
                max: self.max_bandwidth,
            });
        }

        Ok(())
    }

    /// Current fetch count.
    pub fn fetch_count(&self) -> u32 {
        self.fetch_count.load(Ordering::Relaxed)
    }

    /// Current total bandwidth in bytes.
    pub fn bandwidth_bytes(&self) -> u64 {
        self.bandwidth_bytes.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Fetch verified resource
// ---------------------------------------------------------------------------

/// Fetch a media resource and verify its integrity.
///
/// Uses the isolated media `reqwest::Client` (no credentials, no cookies).
/// Enforces the per-media-fetch timeout and tracks against the media budget.
///
/// # Arguments
///
/// * `media_client` — The isolated media HTTP client (no auth, no cookies).
/// * `url` — The resource URL.
/// * `integrity_attr` — The `integrity` attribute value (space-separated tokens).
/// * `element_name` — The element type (e.g. `"img"`, `"audio"`) for error messages.
/// * `timeout` — Per-media-fetch timeout.
/// * `budget` — Media budget tracker for count/bandwidth enforcement.
///
/// # Errors
///
/// Returns `IntegrityMismatch` if the digest doesn't match,
/// `ResourceLimitExceeded` if the budget is exceeded,
/// `Timeout` if the fetch exceeds the timeout,
/// or `Http` for network errors.
pub async fn fetch_verified_resource(
    media_client: &reqwest::Client,
    url: &str,
    integrity_attr: &str,
    element_name: &str,
    timeout: std::time::Duration,
    budget: &MediaBudgetTracker,
) -> crate::Result<Vec<u8>> {
    // Fetch with timeout
    let response = tokio::time::timeout(timeout, media_client.get(url).send())
        .await
        .map_err(|_| AnmlClientError::Timeout {
            operation: "per_media_fetch".into(),
            timeout_secs: timeout.as_secs(),
        })??;

    let bytes = tokio::time::timeout(timeout, response.bytes())
        .await
        .map_err(|_| AnmlClientError::Timeout {
            operation: "per_media_fetch_body".into(),
            timeout_secs: timeout.as_secs(),
        })??;

    let data = bytes.to_vec();

    // Track against budget
    budget.record_fetch(data.len() as u64)?;

    // Verify integrity
    verify_integrity(&data, integrity_attr, element_name)?;

    Ok(data)
}

/// Enforce that `inference="required"` elements have an integrity attribute.
///
/// Call this during document post-parse checks. If an element has
/// `inference="required"` but no `integrity` attribute, this returns
/// a `MalformedDocument` error.
pub fn enforce_inference_integrity(
    inference: Option<&anml::types::enums::InferenceType>,
    integrity: Option<&str>,
    element_name: &str,
    src: &str,
) -> crate::Result<()> {
    if matches!(inference, Some(anml::types::enums::InferenceType::Required)) {
        if integrity.is_none() {
            warn!(
                element = element_name,
                src = src,
                "inference=\"required\" but no integrity attribute — treating as malformed"
            );
            return Err(AnmlClientError::MalformedDocument {
                detail: format!(
                    "<{element_name} src=\"{src}\"> has inference=\"required\" but no integrity attribute"
                ),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_sha256_token() {
        let tokens = parse_integrity_attr(
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=",
        );
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].algorithm, SriAlgorithm::Sha256);
        assert_eq!(
            tokens[0].expected_digest,
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn parse_multiple_tokens_selects_strongest() {
        let tokens = parse_integrity_attr(
            "sha256-abc123 sha512-def456 sha384-ghi789",
        );
        assert_eq!(tokens.len(), 3);
        let strongest = select_strongest(&tokens).unwrap();
        assert_eq!(strongest.algorithm, SriAlgorithm::Sha512);
        assert_eq!(strongest.expected_digest, "def456");
    }

    #[test]
    fn parse_unknown_algorithm_skipped() {
        let tokens = parse_integrity_attr("md5-abc sha256-def");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].algorithm, SriAlgorithm::Sha256);
    }

    #[test]
    fn verify_integrity_sha256_ok() {
        let data = b"hello world";
        let digest = SriAlgorithm::Sha256.digest(data);
        let attr = format!("sha256-{}", digest);
        assert!(verify_integrity(data, &attr, "img").is_ok());
    }

    #[test]
    fn verify_integrity_sha384_ok() {
        let data = b"test data";
        let digest = SriAlgorithm::Sha384.digest(data);
        let attr = format!("sha384-{}", digest);
        assert!(verify_integrity(data, &attr, "audio").is_ok());
    }

    #[test]
    fn verify_integrity_sha512_ok() {
        let data = b"some bytes";
        let digest = SriAlgorithm::Sha512.digest(data);
        let attr = format!("sha512-{}", digest);
        assert!(verify_integrity(data, &attr, "video").is_ok());
    }

    #[test]
    fn verify_integrity_mismatch() {
        let data = b"hello world";
        let attr = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let err = verify_integrity(data, attr, "img").unwrap_err();
        assert!(matches!(err, AnmlClientError::IntegrityMismatch { .. }));
    }

    #[test]
    fn verify_integrity_no_recognized_tokens() {
        let err = verify_integrity(b"data", "md5-abc", "img").unwrap_err();
        assert!(matches!(err, AnmlClientError::MalformedDocument { .. }));
    }

    #[test]
    fn verify_selects_strongest_when_multiple() {
        let data = b"test";
        let sha512_digest = SriAlgorithm::Sha512.digest(data);
        // sha256 digest is wrong, but sha512 is correct — should pass
        // because strongest (sha512) is selected
        let attr = format!("sha256-WRONG sha512-{}", sha512_digest);
        assert!(verify_integrity(data, &attr, "img").is_ok());
    }

    #[test]
    fn media_budget_tracker_allows_within_limits() {
        let budget = ActionBudget {
            max_media_fetches: 5,
            max_media_bandwidth: 1000,
            ..ActionBudget::default()
        };
        let tracker = MediaBudgetTracker::new(&budget);
        assert!(tracker.record_fetch(100).is_ok());
        assert_eq!(tracker.fetch_count(), 1);
        assert_eq!(tracker.bandwidth_bytes(), 100);
    }

    #[test]
    fn media_budget_tracker_rejects_excess_fetches() {
        let budget = ActionBudget {
            max_media_fetches: 1,
            max_media_bandwidth: u64::MAX,
            ..ActionBudget::default()
        };
        let tracker = MediaBudgetTracker::new(&budget);
        assert!(tracker.record_fetch(10).is_ok());
        let err = tracker.record_fetch(10).unwrap_err();
        assert!(matches!(err, AnmlClientError::ResourceLimitExceeded { .. }));
    }

    #[test]
    fn media_budget_tracker_rejects_excess_bandwidth() {
        let budget = ActionBudget {
            max_media_fetches: 100,
            max_media_bandwidth: 50,
            ..ActionBudget::default()
        };
        let tracker = MediaBudgetTracker::new(&budget);
        assert!(tracker.record_fetch(30).is_ok());
        let err = tracker.record_fetch(30).unwrap_err();
        assert!(matches!(err, AnmlClientError::ResourceLimitExceeded { .. }));
    }

    #[test]
    fn enforce_inference_required_without_integrity() {
        let err = enforce_inference_integrity(
            Some(&anml::types::enums::InferenceType::Required),
            None,
            "img",
            "https://cdn.example/logo.png",
        )
        .unwrap_err();
        assert!(matches!(err, AnmlClientError::MalformedDocument { .. }));
    }

    #[test]
    fn enforce_inference_required_with_integrity_ok() {
        assert!(enforce_inference_integrity(
            Some(&anml::types::enums::InferenceType::Required),
            Some("sha256-abc"),
            "img",
            "https://cdn.example/logo.png",
        )
        .is_ok());
    }

    #[test]
    fn enforce_inference_optional_without_integrity_ok() {
        assert!(enforce_inference_integrity(
            Some(&anml::types::enums::InferenceType::Optional),
            None,
            "img",
            "https://cdn.example/logo.png",
        )
        .is_ok());
    }

    #[test]
    fn enforce_inference_none_without_integrity_ok() {
        assert!(enforce_inference_integrity(
            Some(&anml::types::enums::InferenceType::None),
            None,
            "img",
            "https://cdn.example/logo.png",
        )
        .is_ok());
    }

    #[test]
    fn enforce_no_inference_without_integrity_ok() {
        assert!(enforce_inference_integrity(
            None,
            None,
            "img",
            "https://cdn.example/logo.png",
        )
        .is_ok());
    }

    #[test]
    fn algorithm_ordering() {
        assert!(SriAlgorithm::Sha256 < SriAlgorithm::Sha384);
        assert!(SriAlgorithm::Sha384 < SriAlgorithm::Sha512);
    }
}
