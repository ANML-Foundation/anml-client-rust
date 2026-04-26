//! Audit logging trait and in-memory implementation.
//!
//! Every disclosure event is recorded via the [`AuditLog`] trait. The default
//! [`InMemoryAuditLog`] stores entries in an append-only `Vec` behind a
//! `Mutex`, suitable for single-process use. For production deployments,
//! implement `AuditLog` to write to a database, file, or external service.
//!
//! [`NoOpAuditLog`] silently discards entries and is used when no audit log
//! is configured.

use std::sync::Mutex;
use std::time::SystemTime;

use crate::config::Origin;
use crate::disclosure::ConsentBasis;

// ---------------------------------------------------------------------------
// AuditEntry
// ---------------------------------------------------------------------------

/// A single audit record capturing a disclosure event.
///
/// Created by the disclosure algorithm (step 7) and passed to
/// [`AuditLog::record`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AuditEntry {
    /// The field that was disclosed.
    pub field: String,
    /// The origin the field was disclosed to.
    pub origin: Origin,
    /// When the disclosure occurred.
    #[cfg_attr(
        feature = "serde",
        serde(
            serialize_with = "serialize_system_time",
            deserialize_with = "deserialize_system_time"
        )
    )]
    pub timestamp: SystemTime,
    /// The consent basis under which disclosure was authorized.
    pub consent_basis: ConsentBasis,
    /// The disclosure rule that governed this field (e.g. `"field=email"`,
    /// `"field-prefix=contact"`, `"(synthesized)"`).
    pub disclosure_rule: String,
    /// The action ID associated with this disclosure, if any.
    pub action_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Serde helpers for SystemTime
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
fn serialize_system_time<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    serializer.serialize_u64(duration.as_secs())
}

#[cfg(feature = "serde")]
fn deserialize_system_time<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let secs = u64::deserialize(deserializer)?;
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

// ---------------------------------------------------------------------------
// AuditLog trait
// ---------------------------------------------------------------------------

/// Trait for recording disclosure audit entries.
///
/// Implementations must be `Send + Sync` so the audit log can be shared
/// across async tasks via `Arc`.
pub trait AuditLog: Send + Sync + std::fmt::Debug {
    /// Record a disclosure audit entry.
    fn record(&self, entry: AuditEntry);

    /// Downcast support for state export.
    fn as_any(&self) -> &dyn std::any::Any;
}

// ---------------------------------------------------------------------------
// NoOpAuditLog
// ---------------------------------------------------------------------------

/// A no-op audit log that silently discards all entries.
///
/// Used as the default when no audit log is configured.
#[derive(Debug, Default)]
pub struct NoOpAuditLog;

impl AuditLog for NoOpAuditLog {
    fn record(&self, _entry: AuditEntry) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// InMemoryAuditLog
// ---------------------------------------------------------------------------

/// Append-only in-memory audit log backed by `Mutex<Vec<AuditEntry>>`.
///
/// Suitable for single-process use, testing, and short-lived clients.
/// For production, implement [`AuditLog`] with durable storage.
///
/// # Thread Safety
///
/// `InMemoryAuditLog` is `Send + Sync`. The internal `Mutex` serializes
/// writes; reads acquire the lock briefly to clone the entry list.
#[derive(Debug, Default)]
pub struct InMemoryAuditLog {
    entries: Mutex<Vec<AuditEntry>>,
}

impl InMemoryAuditLog {
    /// Create a new empty in-memory audit log.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all recorded entries.
    pub fn list(&self) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.clone()
    }

    /// Return entries matching the given origin.
    pub fn filter_by_origin(&self, origin: &Origin) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().filter(|e| e.origin == *origin).cloned().collect()
    }

    /// Return entries matching the given field name.
    pub fn filter_by_field(&self, field: &str) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().filter(|e| e.field == field).cloned().collect()
    }

    /// Return the number of recorded entries.
    pub fn len(&self) -> usize {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.len()
    }

    /// Returns `true` if no entries have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AuditLog for InMemoryAuditLog {
    fn record(&self, entry: AuditEntry) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Origin;
    use crate::disclosure::ConsentBasis;

    fn test_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        }
    }

    fn other_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "other.com".into(),
            port: None,
        }
    }

    fn make_entry(field: &str, origin: Origin) -> AuditEntry {
        AuditEntry {
            field: field.to_string(),
            origin,
            timestamp: SystemTime::now(),
            consent_basis: ConsentBasis::Explicit,
            disclosure_rule: format!("field={}", field),
            action_id: None,
        }
    }

    #[test]
    fn noop_audit_log_accepts_entries() {
        let log = NoOpAuditLog;
        log.record(make_entry("email", test_origin()));
        // No panic, no storage — just verifying it compiles and runs.
    }

    #[test]
    fn in_memory_starts_empty() {
        let log = InMemoryAuditLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert!(log.list().is_empty());
    }

    #[test]
    fn in_memory_records_and_lists() {
        let log = InMemoryAuditLog::new();
        log.record(make_entry("email", test_origin()));
        log.record(make_entry("phone", test_origin()));
        assert_eq!(log.len(), 2);
        let entries = log.list();
        assert_eq!(entries[0].field, "email");
        assert_eq!(entries[1].field, "phone");
    }

    #[test]
    fn in_memory_filter_by_origin() {
        let log = InMemoryAuditLog::new();
        log.record(make_entry("email", test_origin()));
        log.record(make_entry("phone", other_origin()));
        log.record(make_entry("name", test_origin()));

        let filtered = log.filter_by_origin(&test_origin());
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| e.origin == test_origin()));
    }

    #[test]
    fn in_memory_filter_by_field() {
        let log = InMemoryAuditLog::new();
        log.record(make_entry("email", test_origin()));
        log.record(make_entry("email", other_origin()));
        log.record(make_entry("phone", test_origin()));

        let filtered = log.filter_by_field("email");
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| e.field == "email"));
    }

    #[test]
    fn in_memory_append_only() {
        let log = InMemoryAuditLog::new();
        log.record(make_entry("a", test_origin()));
        log.record(make_entry("b", test_origin()));
        log.record(make_entry("c", test_origin()));
        // Entries are in insertion order
        let entries = log.list();
        let fields: Vec<&str> = entries.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields, vec!["a", "b", "c"]);
    }

    #[test]
    fn audit_log_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemoryAuditLog>();
        assert_send_sync::<NoOpAuditLog>();
    }

    #[test]
    fn audit_entry_has_all_fields() {
        let entry = AuditEntry {
            field: "airline".into(),
            origin: test_origin(),
            timestamp: SystemTime::now(),
            consent_basis: ConsentBasis::Explicit,
            disclosure_rule: "field=airline".into(),
            action_id: Some("submit-airline".into()),
        };
        assert_eq!(entry.field, "airline");
        assert_eq!(entry.action_id, Some("submit-airline".into()));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn audit_entry_serde_round_trip() {
        let entry = AuditEntry {
            field: "email".into(),
            origin: test_origin(),
            timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            consent_basis: ConsentBasis::Implicit,
            disclosure_rule: "field=email".into(),
            action_id: None,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let restored: AuditEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.field, "email");
        assert_eq!(restored.disclosure_rule, "field=email");
        assert!(restored.action_id.is_none());
    }
}
