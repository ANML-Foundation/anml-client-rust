//! Per-field 24-hour sliding window rate limit tracker.
//!
//! Tracks (field, origin) → Vec<Instant> of disclosure timestamps and
//! enforces a configurable maximum within a 24-hour sliding window.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::config::Origin;

/// The 24-hour window duration.
const WINDOW_DURATION: Duration = Duration::from_secs(24 * 60 * 60);

/// Key for rate limit tracking.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RateLimitKey {
    field: String,
    origin: String,
}

/// Per-field rate limit tracker using a 24-hour sliding window.
///
/// Tracks disclosure timestamps for (field, origin) pairs and enforces
/// a maximum number of disclosures within the trailing 24 hours.
///
/// Expired entries are cleaned up on each `check_and_record` call.
#[derive(Debug, Default)]
pub struct RateLimitTracker {
    entries: RwLock<HashMap<RateLimitKey, Vec<Instant>>>,
}

impl RateLimitTracker {
    /// Create a new empty rate limit tracker.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a disclosure would exceed the rate limit, and if not, record it.
    ///
    /// Returns `Ok(())` if the disclosure is allowed (and records the timestamp),
    /// or `Err(retry_after_secs)` if the rate limit would be exceeded.
    pub fn check_and_record(
        &self,
        field: &str,
        origin: &Origin,
        max_per_window: u32,
    ) -> Result<(), u64> {
        self.check_and_record_at(field, origin, max_per_window, Instant::now())
    }

    /// Check and record with a specific timestamp (for testing).
    pub fn check_and_record_at(
        &self,
        field: &str,
        origin: &Origin,
        max_per_window: u32,
        now: Instant,
    ) -> Result<(), u64> {
        let key = RateLimitKey {
            field: field.to_string(),
            origin: origin.to_string(),
        };

        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        let timestamps = entries.entry(key).or_default();

        // Clean up expired entries
        let cutoff = now.checked_sub(WINDOW_DURATION).unwrap_or(now);
        timestamps.retain(|t| *t > cutoff);

        // Check if adding one more would exceed the limit
        if timestamps.len() >= max_per_window as usize {
            // Calculate retry-after: time until the oldest entry expires
            let retry_after = if let Some(oldest) = timestamps.first() {
                let expires_at = *oldest + WINDOW_DURATION;
                if expires_at > now {
                    (expires_at - now).as_secs()
                } else {
                    0
                }
            } else {
                0
            };
            return Err(retry_after);
        }

        // Record the disclosure
        timestamps.push(now);
        Ok(())
    }

    /// Check the current count for a (field, origin) pair without recording.
    pub fn current_count(&self, field: &str, origin: &Origin) -> usize {
        let key = RateLimitKey {
            field: field.to_string(),
            origin: origin.to_string(),
        };
        let now = Instant::now();
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        entries
            .get(&key)
            .map(|timestamps| {
                let cutoff = now.checked_sub(WINDOW_DURATION).unwrap_or(now);
                timestamps.iter().filter(|t| **t > cutoff).count()
            })
            .unwrap_or(0)
    }

    /// Clear all tracked entries.
    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        entries.clear();
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
    fn allows_within_limit() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        assert!(tracker.check_and_record("email", &origin, 3).is_ok());
        assert!(tracker.check_and_record("email", &origin, 3).is_ok());
        assert!(tracker.check_and_record("email", &origin, 3).is_ok());
    }

    #[test]
    fn rejects_over_limit() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        assert!(tracker.check_and_record("email", &origin, 2).is_ok());
        assert!(tracker.check_and_record("email", &origin, 2).is_ok());
        assert!(tracker.check_and_record("email", &origin, 2).is_err());
    }

    #[test]
    fn different_fields_independent() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        assert!(tracker.check_and_record("email", &origin, 1).is_ok());
        assert!(tracker.check_and_record("phone", &origin, 1).is_ok());
    }

    #[test]
    fn different_origins_independent() {
        let tracker = RateLimitTracker::new();
        let origin1 = test_origin();
        let origin2 = Origin {
            scheme: "https".into(),
            host: "other.com".into(),
            port: None,
        };
        assert!(tracker.check_and_record("email", &origin1, 1).is_ok());
        assert!(tracker.check_and_record("email", &origin2, 1).is_ok());
    }

    #[test]
    fn expired_entries_cleaned_up() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        let now = Instant::now();

        // Record at a time 25 hours ago (expired)
        let old_time = now - Duration::from_secs(25 * 60 * 60);
        assert!(tracker
            .check_and_record_at("email", &origin, 1, old_time)
            .is_ok());

        // Should be allowed now because the old entry expired
        assert!(tracker
            .check_and_record_at("email", &origin, 1, now)
            .is_ok());
    }

    #[test]
    fn retry_after_is_reasonable() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        let now = Instant::now();

        assert!(tracker
            .check_and_record_at("email", &origin, 1, now)
            .is_ok());
        let err = tracker
            .check_and_record_at("email", &origin, 1, now + Duration::from_secs(1))
            .unwrap_err();
        // retry_after should be close to 24h minus 1s
        assert!(err > 0);
        assert!(err <= 24 * 60 * 60);
    }

    #[test]
    fn current_count_reflects_state() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        assert_eq!(tracker.current_count("email", &origin), 0);
        tracker.check_and_record("email", &origin, 10).unwrap();
        assert_eq!(tracker.current_count("email", &origin), 1);
        tracker.check_and_record("email", &origin, 10).unwrap();
        assert_eq!(tracker.current_count("email", &origin), 2);
    }

    #[test]
    fn clear_resets_all() {
        let tracker = RateLimitTracker::new();
        let origin = test_origin();
        tracker.check_and_record("email", &origin, 10).unwrap();
        tracker.clear();
        assert_eq!(tracker.current_count("email", &origin), 0);
    }

    #[test]
    fn tracker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RateLimitTracker>();
    }
}
