//! Consent store for tracking disclosure consent grants.
//!
//! The `ConsentStore` tracks grants by (field, origin, scope) where scope is
//! session, origin, or global. It is `Send + Sync` (uses `RwLock` internally).

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use crate::config::Origin;
use super::matching::ConsentScope;

// ---------------------------------------------------------------------------
// ConsentBasis
// ---------------------------------------------------------------------------

/// The consent basis under which a disclosure was authorized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConsentBasis {
    /// The principal explicitly granted consent for this specific field.
    Explicit,
    /// Consent was inferred from a prior standing grant.
    Implicit,
    /// Consent was delegated (e.g., from a parent agent).
    Delegated,
}

impl std::fmt::Display for ConsentBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Explicit => write!(f, "explicit"),
            Self::Implicit => write!(f, "implicit"),
            Self::Delegated => write!(f, "delegated"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConsentKey + ConsentGrant
// ---------------------------------------------------------------------------

/// Key for looking up a consent grant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConsentKey {
    /// The field name.
    pub field: String,
    /// The origin (serialized as string for HashMap key).
    pub origin: String,
    /// The consent scope.
    pub scope: ConsentScope,
}

/// A recorded consent grant.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConsentGrant {
    /// The consent basis (explicit, implicit, delegated).
    pub basis: ConsentBasis,
    /// When the grant was recorded.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub granted_at: Option<Instant>,
    /// The field this grant covers.
    pub field: String,
    /// The origin this grant covers.
    pub origin: String,
    /// The scope of this grant.
    pub scope: ConsentScope,
}

// ---------------------------------------------------------------------------
// ConsentStore
// ---------------------------------------------------------------------------

/// Thread-safe consent store tracking disclosure consent grants.
///
/// Grants are keyed by (field, origin, scope). The store supports:
/// - `grant()` — record a new consent grant
/// - `check()` — check if a grant exists for (field, origin, scope)
/// - `revoke()` — remove a specific grant
/// - `list()` — list all grants, optionally filtered
///
/// Session scope grants are ephemeral (per client instance).
/// Origin scope grants are visible to the same origin.
/// Global scope grants are cross-origin.
#[derive(Debug, Default)]
pub struct ConsentStore {
    grants: RwLock<HashMap<ConsentKey, ConsentGrant>>,
}

impl ConsentStore {
    /// Create a new empty consent store.
    pub fn new() -> Self {
        Self {
            grants: RwLock::new(HashMap::new()),
        }
    }

    /// Record a consent grant.
    pub fn grant(
        &self,
        field: &str,
        origin: &Origin,
        scope: ConsentScope,
        basis: ConsentBasis,
    ) {
        let key = ConsentKey {
            field: field.to_string(),
            origin: origin.to_string(),
            scope,
        };
        let grant = ConsentGrant {
            basis,
            granted_at: Some(Instant::now()),
            field: field.to_string(),
            origin: origin.to_string(),
            scope,
        };
        let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
        grants.insert(key, grant);
    }

    /// Check if a consent grant exists for the given (field, origin, scope).
    ///
    /// For `Global` scope, the origin is ignored (matches any origin).
    /// For `Origin` scope, matches the specific origin.
    /// For `Session` scope, matches the specific origin (ephemeral).
    pub fn check(
        &self,
        field: &str,
        origin: &Origin,
        scope: ConsentScope,
    ) -> Option<ConsentBasis> {
        let grants = self.grants.read().unwrap_or_else(|e| e.into_inner());

        // Check exact match first
        let key = ConsentKey {
            field: field.to_string(),
            origin: origin.to_string(),
            scope,
        };
        if let Some(grant) = grants.get(&key) {
            return Some(grant.basis);
        }

        // For global scope, check if there's a global grant for this field
        // from any origin
        if scope == ConsentScope::Global || scope == ConsentScope::Origin {
            for (k, v) in grants.iter() {
                if k.field == field && k.scope == ConsentScope::Global {
                    return Some(v.basis);
                }
            }
        }

        None
    }

    /// Revoke a specific consent grant.
    ///
    /// Returns `true` if a grant was removed, `false` if none existed.
    pub fn revoke(
        &self,
        field: &str,
        origin: &Origin,
        scope: ConsentScope,
    ) -> bool {
        let key = ConsentKey {
            field: field.to_string(),
            origin: origin.to_string(),
            scope,
        };
        let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
        grants.remove(&key).is_some()
    }

    /// List all consent grants, optionally filtered by origin.
    pub fn list(&self, origin_filter: Option<&Origin>) -> Vec<ConsentGrant> {
        let grants = self.grants.read().unwrap_or_else(|e| e.into_inner());
        grants
            .values()
            .filter(|g| {
                origin_filter
                    .map_or(true, |o| g.origin == o.to_string())
            })
            .cloned()
            .collect()
    }

    /// Clear all grants (useful for testing or session reset).
    pub fn clear(&self) {
        let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
        grants.clear();
    }

    /// Export all grants as a serializable snapshot.
    #[cfg(feature = "serde")]
    pub fn export_grants(&self) -> Vec<ConsentGrant> {
        let grants = self.grants.read().unwrap_or_else(|e| e.into_inner());
        grants.values().cloned().collect()
    }

    /// Restore grants from a serialized snapshot.
    #[cfg(feature = "serde")]
    pub fn restore_grants(&self, grants: Vec<ConsentGrant>) {
        let mut store = self.grants.write().unwrap_or_else(|e| e.into_inner());
        store.clear();
        for grant in grants {
            let key = ConsentKey {
                field: grant.field.clone(),
                origin: grant.origin.clone(),
                scope: grant.scope,
            };
            store.insert(key, grant);
        }
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

    fn other_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "other.com".into(),
            port: None,
        }
    }

    #[test]
    fn grant_and_check_session() {
        let store = ConsentStore::new();
        let origin = test_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        assert_eq!(
            store.check("email", &origin, ConsentScope::Session),
            Some(ConsentBasis::Explicit)
        );
    }

    #[test]
    fn check_missing_returns_none() {
        let store = ConsentStore::new();
        let origin = test_origin();
        assert_eq!(store.check("email", &origin, ConsentScope::Session), None);
    }

    #[test]
    fn session_scope_isolated_by_origin() {
        let store = ConsentStore::new();
        let origin = test_origin();
        let other = other_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        assert!(store.check("email", &origin, ConsentScope::Session).is_some());
        assert!(store.check("email", &other, ConsentScope::Session).is_none());
    }

    #[test]
    fn global_scope_visible_cross_origin() {
        let store = ConsentStore::new();
        let origin = test_origin();
        let other = other_origin();
        store.grant("email", &origin, ConsentScope::Global, ConsentBasis::Implicit);
        // Global grants are visible from any origin
        assert!(store.check("email", &other, ConsentScope::Global).is_some());
    }

    #[test]
    fn revoke_removes_grant() {
        let store = ConsentStore::new();
        let origin = test_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        assert!(store.check("email", &origin, ConsentScope::Session).is_some());
        assert!(store.revoke("email", &origin, ConsentScope::Session));
        assert!(store.check("email", &origin, ConsentScope::Session).is_none());
    }

    #[test]
    fn revoke_nonexistent_returns_false() {
        let store = ConsentStore::new();
        let origin = test_origin();
        assert!(!store.revoke("email", &origin, ConsentScope::Session));
    }

    #[test]
    fn list_all_grants() {
        let store = ConsentStore::new();
        let origin = test_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        store.grant("phone", &origin, ConsentScope::Origin, ConsentBasis::Implicit);
        let all = store.list(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_filtered_by_origin() {
        let store = ConsentStore::new();
        let origin = test_origin();
        let other = other_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        store.grant("phone", &other, ConsentScope::Session, ConsentBasis::Implicit);
        let filtered = store.list(Some(&origin));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].field, "email");
    }

    #[test]
    fn clear_removes_all() {
        let store = ConsentStore::new();
        let origin = test_origin();
        store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Explicit);
        store.grant("phone", &origin, ConsentScope::Origin, ConsentBasis::Implicit);
        store.clear();
        assert!(store.list(None).is_empty());
    }

    #[test]
    fn consent_basis_display() {
        assert_eq!(ConsentBasis::Explicit.to_string(), "explicit");
        assert_eq!(ConsentBasis::Implicit.to_string(), "implicit");
        assert_eq!(ConsentBasis::Delegated.to_string(), "delegated");
    }

    #[test]
    fn store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ConsentStore>();
    }
}
