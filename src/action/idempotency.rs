//! Idempotency-Key generation and retry semantics.
//!
//! When an agent executes a non-idempotent `<action>`, it SHOULD include an
//! `Idempotency-Key` header (UUIDv4, ≥128 bits entropy). On network failure,
//! the same key is reused. On application error, a new key is generated
//! unless the error is explicitly retriable.

use uuid::Uuid;

/// The HTTP header name for idempotency keys.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";

/// The ANML problem type URI for transient errors (retriable with same key).
pub const TRANSIENT_PROBLEM_TYPE: &str = "urn:ietf:params:xml:ns:anml:problem:transient";

/// Generate a fresh Idempotency-Key (UUIDv4, 128 bits of entropy).
pub fn generate_key() -> String {
    Uuid::new_v4().to_string()
}

/// Determines whether a failed request should be retried with the same
/// idempotency key or a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry with the same idempotency key (network-level failure).
    RetryWithSameKey,
    /// Retry with a new idempotency key (retriable application error).
    RetryWithNewKey,
    /// Do not retry.
    DoNotRetry,
}

/// Evaluate whether a failed action execution should be retried and how.
///
/// Per RFC Section 8.6 (Idempotency-Key Binding):
/// - Network-level failure → retry with same key
/// - Application error with `retry-after` or `transient` problem → retry with new key
/// - Other application errors → do not retry
pub fn evaluate_retry(
    is_network_failure: bool,
    retry_after: Option<u64>,
    problem_type: Option<&str>,
) -> RetryDecision {
    if is_network_failure {
        return RetryDecision::RetryWithSameKey;
    }

    // Application-level error: check if explicitly retriable
    if retry_after.is_some() {
        return RetryDecision::RetryWithNewKey;
    }

    if let Some(pt) = problem_type {
        if pt == TRANSIENT_PROBLEM_TYPE {
            return RetryDecision::RetryWithNewKey;
        }
    }

    RetryDecision::DoNotRetry
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_key_is_valid_uuid() {
        let key = generate_key();
        assert!(Uuid::parse_str(&key).is_ok());
    }

    #[test]
    fn generate_key_is_unique() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert_ne!(k1, k2);
    }

    #[test]
    fn network_failure_retries_with_same_key() {
        assert_eq!(
            evaluate_retry(true, None, None),
            RetryDecision::RetryWithSameKey
        );
    }

    #[test]
    fn retry_after_retries_with_new_key() {
        assert_eq!(
            evaluate_retry(false, Some(30), None),
            RetryDecision::RetryWithNewKey
        );
    }

    #[test]
    fn transient_problem_retries_with_new_key() {
        assert_eq!(
            evaluate_retry(false, None, Some(TRANSIENT_PROBLEM_TYPE)),
            RetryDecision::RetryWithNewKey
        );
    }

    #[test]
    fn other_app_error_does_not_retry() {
        assert_eq!(
            evaluate_retry(false, None, Some("urn:ietf:params:xml:ns:anml:problem:malformed-document")),
            RetryDecision::DoNotRetry
        );
    }

    #[test]
    fn no_error_info_does_not_retry() {
        assert_eq!(
            evaluate_retry(false, None, None),
            RetryDecision::DoNotRetry
        );
    }
}
