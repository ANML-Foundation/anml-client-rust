//! TTL-based in-memory document cache (feature-gated: `cache`).
//!
//! Provides a simple URL-keyed cache that respects `ttl` from the `<anml>`
//! root element and `<inform>` elements. Entries are automatically
//! invalidated when their TTL expires.
//!
//! # Usage
//!
//! ```rust,no_run
//! use anml_client::cache::DocumentCache;
//! use std::time::Duration;
//!
//! let cache = DocumentCache::new();
//! // Cache is checked automatically by AnmlClient when the `cache` feature is enabled.
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anml::types::document::AnmlDocument;

// ---------------------------------------------------------------------------
// Cache entry
// ---------------------------------------------------------------------------

/// A cached document with its expiration time.
#[derive(Clone, Debug)]
struct CacheEntry {
    /// The cached document.
    document: AnmlDocument,
    /// When this entry was inserted.
    inserted_at: Instant,
    /// Time-to-live for this entry.
    ttl: Duration,
}

impl CacheEntry {
    /// Returns `true` if this entry has expired.
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

// ---------------------------------------------------------------------------
// DocumentCache
// ---------------------------------------------------------------------------

/// An in-memory TTL-based document cache keyed by URL.
///
/// Thread-safe (`Clone + Send + Sync`) via `Arc<RwLock<...>>`.
///
/// The cache respects the `ttl` attribute from the `<anml>` root element.
/// If no TTL is specified on the document, a configurable default TTL is used.
/// Entries are lazily evicted on access (no background thread).
#[derive(Clone, Debug)]
pub struct DocumentCache {
    inner: Arc<RwLock<CacheInner>>,
}

#[derive(Debug)]
struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    default_ttl: Duration,
    max_entries: usize,
}

impl DocumentCache {
    /// Create a new cache with default settings (5-minute default TTL, 1000 max entries).
    pub fn new() -> Self {
        Self::with_config(Duration::from_secs(300), 1000)
    }

    /// Create a new cache with custom default TTL and max entry count.
    pub fn with_config(default_ttl: Duration, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheInner {
                entries: HashMap::new(),
                default_ttl,
                max_entries,
            })),
        }
    }

    /// Look up a cached document by URL.
    ///
    /// Returns `None` if the URL is not cached or the entry has expired.
    /// Expired entries are removed on access.
    pub fn get(&self, url: &str) -> Option<AnmlDocument> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());

        if let Some(entry) = inner.entries.get(url) {
            if entry.is_expired() {
                inner.entries.remove(url);
                return None;
            }
            return Some(entry.document.clone());
        }
        None
    }

    /// Insert a document into the cache.
    ///
    /// The TTL is extracted from the document's `ttl` attribute. If not
    /// present, the cache's default TTL is used. If the document's TTL
    /// is `"0"`, the document is not cached.
    pub fn insert(&self, url: &str, document: AnmlDocument) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());

        // Extract TTL from document
        let ttl = extract_ttl(&document).unwrap_or(inner.default_ttl);

        // Don't cache if TTL is zero
        if ttl.is_zero() {
            return;
        }

        // Evict expired entries if we're at capacity
        if inner.entries.len() >= inner.max_entries {
            inner.entries.retain(|_, entry| !entry.is_expired());
        }

        // If still at capacity after eviction, skip insertion
        if inner.entries.len() >= inner.max_entries {
            return;
        }

        inner.entries.insert(
            url.to_string(),
            CacheEntry {
                document,
                inserted_at: Instant::now(),
                ttl,
            },
        );
    }

    /// Insert a document with an explicit TTL override.
    pub fn insert_with_ttl(&self, url: &str, document: AnmlDocument, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }

        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());

        if inner.entries.len() >= inner.max_entries {
            inner.entries.retain(|_, entry| !entry.is_expired());
        }

        if inner.entries.len() >= inner.max_entries {
            return;
        }

        inner.entries.insert(
            url.to_string(),
            CacheEntry {
                document,
                inserted_at: Instant::now(),
                ttl,
            },
        );
    }

    /// Remove a specific URL from the cache.
    pub fn invalidate(&self, url: &str) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.remove(url);
    }

    /// Remove all entries from the cache.
    pub fn clear(&self) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.clear();
    }

    /// Returns the number of entries currently in the cache (including expired).
    pub fn len(&self) -> usize {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.entries.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove all expired entries.
    pub fn evict_expired(&self) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.retain(|_, entry| !entry.is_expired());
    }
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TTL extraction
// ---------------------------------------------------------------------------

/// Extract the TTL duration from an ANML document's `ttl` attribute.
///
/// The `ttl` attribute is an integer number of seconds. Returns `None`
/// if the attribute is absent or not a valid integer.
fn extract_ttl(doc: &AnmlDocument) -> Option<Duration> {
    doc.ttl
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc(ttl: Option<&str>) -> AnmlDocument {
        AnmlDocument {
            ttl: ttl.map(|s| s.to_string()),
            ..AnmlDocument::default()
        }
    }

    #[test]
    fn extract_ttl_from_document() {
        let doc = make_doc(Some("3600"));
        assert_eq!(extract_ttl(&doc), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn extract_ttl_none_when_absent() {
        let doc = make_doc(None);
        assert_eq!(extract_ttl(&doc), None);
    }

    #[test]
    fn extract_ttl_none_for_invalid() {
        let doc = make_doc(Some("not-a-number"));
        assert_eq!(extract_ttl(&doc), None);
    }

    #[test]
    fn cache_insert_and_get() {
        let cache = DocumentCache::new();
        let doc = make_doc(Some("3600"));
        cache.insert("https://example.com/service", doc.clone());
        let cached = cache.get("https://example.com/service");
        assert!(cached.is_some());
    }

    #[test]
    fn cache_miss_for_unknown_url() {
        let cache = DocumentCache::new();
        assert!(cache.get("https://example.com/unknown").is_none());
    }

    #[test]
    fn cache_invalidate_removes_entry() {
        let cache = DocumentCache::new();
        let doc = make_doc(Some("3600"));
        cache.insert("https://example.com/service", doc);
        cache.invalidate("https://example.com/service");
        assert!(cache.get("https://example.com/service").is_none());
    }

    #[test]
    fn cache_clear_removes_all() {
        let cache = DocumentCache::new();
        cache.insert("https://a.com", make_doc(Some("3600")));
        cache.insert("https://b.com", make_doc(Some("3600")));
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_ttl_zero_not_cached() {
        let cache = DocumentCache::new();
        let doc = make_doc(Some("0"));
        cache.insert("https://example.com/service", doc);
        assert!(cache.get("https://example.com/service").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_uses_default_ttl_when_doc_has_none() {
        let cache = DocumentCache::with_config(Duration::from_secs(60), 100);
        let doc = make_doc(None);
        cache.insert("https://example.com/service", doc);
        // Should be cached with default TTL
        assert!(cache.get("https://example.com/service").is_some());
    }

    #[test]
    fn cache_insert_with_explicit_ttl() {
        let cache = DocumentCache::new();
        let doc = make_doc(None);
        cache.insert_with_ttl(
            "https://example.com/service",
            doc,
            Duration::from_secs(120),
        );
        assert!(cache.get("https://example.com/service").is_some());
    }

    #[test]
    fn cache_max_entries_enforced() {
        let cache = DocumentCache::with_config(Duration::from_secs(3600), 2);
        cache.insert("https://a.com", make_doc(Some("3600")));
        cache.insert("https://b.com", make_doc(Some("3600")));
        // Third insert should be silently dropped (no expired entries to evict)
        cache.insert("https://c.com", make_doc(Some("3600")));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_evict_expired() {
        let cache = DocumentCache::new();
        // Insert with a very short TTL via insert_with_ttl
        cache.insert_with_ttl(
            "https://example.com/expired",
            make_doc(None),
            Duration::from_millis(1),
        );
        // Wait for expiry
        std::thread::sleep(Duration::from_millis(10));
        cache.evict_expired();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_get_removes_expired_entry() {
        let cache = DocumentCache::new();
        cache.insert_with_ttl(
            "https://example.com/expired",
            make_doc(None),
            Duration::from_millis(1),
        );
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.get("https://example.com/expired").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_is_clone_send_sync() {
        fn assert_clone_send_sync<T: Clone + Send + Sync>() {}
        assert_clone_send_sync::<DocumentCache>();
    }
}
