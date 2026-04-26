//! HMAC-SHA256 tokenization for disclosure field values.
//!
//! The [`Tokenizer`] replaces plaintext answer values with non-reversible,
//! hex-encoded tokens bound to the (field, principal, origin) tuple. This
//! prevents cross-origin correlation while still allowing a single service
//! to recognize repeated disclosures from the same principal.
//!
//! # Security Properties
//!
//! - Per-client secret generated via [`OsRng`](rand::rngs::OsRng) (256-bit).
//! - HMAC-SHA256 ensures non-reversibility and collision resistance.
//! - Tokens are hex-encoded (not URL-safe, not base64).
//! - Binding to (field, principal, origin) prevents cross-field and
//!   cross-origin linkability.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::Origin;

type HmacSha256 = Hmac<Sha256>;

/// Per-client HMAC-SHA256 tokenizer for disclosure step 6.
///
/// Each `Tokenizer` holds a 256-bit secret generated at construction
/// time via [`OsRng`](rand::rngs::OsRng). The secret never leaves
/// the process and is not serializable.
///
/// `Tokenizer` is `Send + Sync` and can be shared across tasks.
#[derive(Debug)]
pub struct Tokenizer {
    secret: [u8; 32],
}

impl Tokenizer {
    /// Create a new `Tokenizer` with a cryptographically random secret.
    pub fn new() -> Self {
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        Self { secret }
    }

    /// Create a `Tokenizer` from an existing secret.
    ///
    /// This is useful for testing or when restoring state. In production,
    /// prefer [`Tokenizer::new()`] which generates a fresh secret.
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    /// Returns a reference to the underlying secret bytes.
    ///
    /// Useful for passing to the disclosure engine which needs the raw
    /// secret for its `DisclosureContext`.
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    /// Produce a hex-encoded, non-reversible token bound to the
    /// (field, principal_id, origin) tuple.
    ///
    /// The token is computed as:
    /// ```text
    /// HMAC-SHA256(secret, field || "|" || principal_id || "|" || origin)
    /// ```
    ///
    /// The output is a 64-character lowercase hex string (256 bits).
    /// It is intentionally not URL-safe and not reversible.
    pub fn tokenize(&self, field: &str, principal_id: &str, origin: &Origin) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC can take key of any size");
        mac.update(field.as_bytes());
        mac.update(b"|");
        mac.update(principal_id.as_bytes());
        mac.update(b"|");
        mac.update(origin.to_string().as_bytes());

        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
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
            host: "example.com".into(),
            port: None,
        }
    }

    #[test]
    fn tokenizer_produces_hex_encoded_output() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let token = t.tokenize("email", "user-123", &test_origin());
        // 64 hex chars = 32 bytes = SHA-256 output
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokenizer_is_deterministic() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let origin = test_origin();
        let a = t.tokenize("email", "user-123", &origin);
        let b = t.tokenize("email", "user-123", &origin);
        assert_eq!(a, b);
    }

    #[test]
    fn tokenizer_different_fields_produce_different_tokens() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let origin = test_origin();
        let a = t.tokenize("email", "user-123", &origin);
        let b = t.tokenize("phone", "user-123", &origin);
        assert_ne!(a, b);
    }

    #[test]
    fn tokenizer_different_principals_produce_different_tokens() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let origin = test_origin();
        let a = t.tokenize("email", "user-123", &origin);
        let b = t.tokenize("email", "user-456", &origin);
        assert_ne!(a, b);
    }

    #[test]
    fn tokenizer_different_origins_produce_different_tokens() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let origin_a = test_origin();
        let origin_b = Origin {
            scheme: "https".into(),
            host: "other.com".into(),
            port: None,
        };
        let a = t.tokenize("email", "user-123", &origin_a);
        let b = t.tokenize("email", "user-123", &origin_b);
        assert_ne!(a, b);
    }

    #[test]
    fn tokenizer_different_secrets_produce_different_tokens() {
        let t1 = Tokenizer::from_secret([1u8; 32]);
        let t2 = Tokenizer::from_secret([2u8; 32]);
        let origin = test_origin();
        let a = t1.tokenize("email", "user-123", &origin);
        let b = t2.tokenize("email", "user-123", &origin);
        assert_ne!(a, b);
    }

    #[test]
    fn tokenizer_is_not_reversible() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let token = t.tokenize("email", "user-123", &test_origin());
        // Token should not contain the original value
        assert!(!token.contains("email"));
        assert!(!token.contains("user-123"));
        assert!(!token.contains("example.com"));
    }

    #[test]
    fn tokenizer_default_generates_random_secret() {
        let t1 = Tokenizer::default();
        let t2 = Tokenizer::default();
        // Two independently created tokenizers should have different secrets
        assert_ne!(t1.secret, t2.secret);
    }

    #[test]
    fn tokenizer_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Tokenizer>();
    }

    #[test]
    fn tokenizer_with_port_in_origin() {
        let t = Tokenizer::from_secret([42u8; 32]);
        let origin_no_port = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        let origin_with_port = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: Some(8443),
        };
        let a = t.tokenize("email", "user-123", &origin_no_port);
        let b = t.tokenize("email", "user-123", &origin_with_port);
        // Different origins (port matters) should produce different tokens
        assert_ne!(a, b);
    }
}
