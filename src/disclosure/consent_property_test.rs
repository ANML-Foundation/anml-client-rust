//! Property-based tests for consent store and rate limits.
//!
//! **Validates: Requirements 7.4, 7.6** (consent store scoping, rate limits)

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::time::{Duration, Instant};

    use crate::config::Origin;
    use crate::disclosure::consent::{ConsentBasis, ConsentStore};
    use crate::disclosure::matching::ConsentScope;
    use crate::disclosure::rate_limit::RateLimitTracker;

    // -----------------------------------------------------------------------
    // Arbitrary generators
    // -----------------------------------------------------------------------

    fn arb_origin() -> impl Strategy<Value = Origin> {
        "[a-z]{3,8}".prop_map(|host| Origin {
            scheme: "https".to_string(),
            host: format!("{}.example.com", host),
            port: None,
        })
    }

    fn arb_field() -> impl Strategy<Value = String> {
        "[a-z]{2,8}"
    }

    fn arb_consent_basis() -> impl Strategy<Value = ConsentBasis> {
        prop_oneof![
            Just(ConsentBasis::Explicit),
            Just(ConsentBasis::Implicit),
            Just(ConsentBasis::Delegated),
        ]
    }

    // -----------------------------------------------------------------------
    // Consent store properties
    // -----------------------------------------------------------------------

    proptest! {
        /// Session scope grants are isolated per origin.
        #[test]
        fn session_scope_isolated(
            field in arb_field(),
            origin1 in arb_origin(),
            origin2 in arb_origin(),
            basis in arb_consent_basis(),
        ) {
            prop_assume!(origin1.host != origin2.host);
            let store = ConsentStore::new();
            store.grant(&field, &origin1, ConsentScope::Session, basis);

            // Same origin can see it
            prop_assert!(store.check(&field, &origin1, ConsentScope::Session).is_some());
            // Different origin cannot
            prop_assert!(store.check(&field, &origin2, ConsentScope::Session).is_none());
        }

        /// Global scope grants are visible cross-origin.
        #[test]
        fn global_scope_cross_origin(
            field in arb_field(),
            origin1 in arb_origin(),
            origin2 in arb_origin(),
            basis in arb_consent_basis(),
        ) {
            let store = ConsentStore::new();
            store.grant(&field, &origin1, ConsentScope::Global, basis);

            // Visible from any origin at global scope
            prop_assert!(store.check(&field, &origin2, ConsentScope::Global).is_some());
        }

        /// Revoke removes exactly the targeted grant.
        #[test]
        fn revoke_removes_targeted_grant(
            field1 in arb_field(),
            field2 in arb_field(),
            origin in arb_origin(),
        ) {
            prop_assume!(field1 != field2);
            let store = ConsentStore::new();
            store.grant(&field1, &origin, ConsentScope::Session, ConsentBasis::Explicit);
            store.grant(&field2, &origin, ConsentScope::Session, ConsentBasis::Explicit);

            // Revoke field1
            let removed = store.revoke(&field1, &origin, ConsentScope::Session);
            prop_assert!(removed);

            // field1 is gone, field2 remains
            prop_assert!(store.check(&field1, &origin, ConsentScope::Session).is_none());
            prop_assert!(store.check(&field2, &origin, ConsentScope::Session).is_some());
        }
    }

    // -----------------------------------------------------------------------
    // Rate limit properties
    // -----------------------------------------------------------------------

    proptest! {
        /// Rate limit 24h sliding window: allows up to max, rejects after.
        #[test]
        fn rate_limit_sliding_window(
            field in arb_field(),
            origin in arb_origin(),
            max in 1u32..10,
        ) {
            let tracker = RateLimitTracker::new();
            let now = Instant::now();

            // Fill up to the limit
            for i in 0..max {
                let t = now + Duration::from_secs(i as u64);
                let result = tracker.check_and_record_at(&field, &origin, max, t);
                prop_assert!(result.is_ok(), "should allow disclosure {} of {}", i + 1, max);
            }

            // One more should fail
            let t = now + Duration::from_secs(max as u64);
            let result = tracker.check_and_record_at(&field, &origin, max, t);
            prop_assert!(result.is_err(), "should reject disclosure {} (limit {})", max + 1, max);
        }

        /// Rate limit resets after 24h window passes.
        #[test]
        fn rate_limit_resets_after_24h(
            field in arb_field(),
            origin in arb_origin(),
        ) {
            let tracker = RateLimitTracker::new();
            let now = Instant::now();

            // Use up the limit (max=1)
            prop_assert!(tracker.check_and_record_at(&field, &origin, 1, now).is_ok());
            prop_assert!(tracker.check_and_record_at(&field, &origin, 1, now + Duration::from_secs(1)).is_err());

            // After 25 hours, should be allowed again
            let later = now + Duration::from_secs(25 * 3600);
            prop_assert!(tracker.check_and_record_at(&field, &origin, 1, later).is_ok());
        }
    }
}
