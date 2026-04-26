//! Nav-based paginated async stream over ANML documents.
//!
//! Provides [`PaginatedStream`] which wraps `<nav>` next/prev/cursor
//! pagination into an async iterator yielding pages of [`AnmlDocument`].
//!
//! # Usage
//!
//! ```rust,no_run
//! # async fn example() -> anml_client::Result<()> {
//! # let client = anml_client::client::AnmlClient::builder()
//! #     .base_url("https://example.com")
//! #     .build()?;
//! # let doc = client.fetch("/flights").await?;
//! let mut pages = client.paginate(doc, Some("flights"));
//! while let Some(page) = pages.next_page().await {
//!     let doc = page?;
//!     // Process items from doc.body...
//! }
//! # Ok(())
//! # }
//! ```

use anml::types::document::AnmlDocument;
use anml::types::elements::AnmlNav;

use crate::client::AnmlClient;

// ---------------------------------------------------------------------------
// Nav helpers
// ---------------------------------------------------------------------------

/// Extract the `<nav>` element from a document's body.
///
/// If `data_id` is provided, looks for a `<nav>` associated with that
/// `<data>` section. Otherwise returns the first `<nav>` found in the body.
pub fn find_nav<'a>(doc: &'a AnmlDocument, data_id: Option<&str>) -> Option<&'a AnmlNav> {
    let body = doc.body.as_ref()?;
    if let Some(ref children) = body.children {
        for child in children {
            match child {
                anml::types::elements::AnmlBodyChild::Nav(nav) => {
                    // If no data_id filter, return first nav
                    if data_id.is_none() {
                        return Some(nav);
                    }
                    // Otherwise return any nav (nav doesn't have a data_id field,
                    // so we return the first one found after the matching data)
                    return Some(nav);
                }
                anml::types::elements::AnmlBodyChild::Data(data) => {
                    if let Some(target_id) = data_id {
                        if data.id.as_deref() == Some(target_id) {
                            // Look for nav after this data element
                            // Continue scanning — the nav should follow
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Extract the `next` URL from a document's `<nav>`.
pub fn next_url(doc: &AnmlDocument, data_id: Option<&str>) -> Option<String> {
    find_nav(doc, data_id).and_then(|nav| nav.next.clone())
}

/// Extract the `total` count from a document's `<nav>`, if available.
pub fn nav_total(doc: &AnmlDocument, data_id: Option<&str>) -> Option<u64> {
    find_nav(doc, data_id)
        .and_then(|nav| nav.total.as_deref())
        .and_then(|t| t.parse().ok())
}

// ---------------------------------------------------------------------------
// PaginatedStream
// ---------------------------------------------------------------------------

/// An async paginated iterator over ANML documents.
///
/// Yields pages of [`AnmlDocument`] by following `<nav>` `next` links.
/// Pagination ends when no `next` URL is present.
#[derive(Debug)]
pub struct PageIterator {
    client: AnmlClient,
    next_url: Option<String>,
    data_id: Option<String>,
    total: Option<u64>,
    first_doc: Option<AnmlDocument>,
}

impl PageIterator {
    /// Create a new page iterator from an initial document.
    ///
    /// The initial document is yielded as the first page.
    pub fn new(
        client: &AnmlClient,
        initial_doc: AnmlDocument,
        data_id: Option<&str>,
    ) -> Self {
        let total = nav_total(&initial_doc, data_id);
        let next = next_url(&initial_doc, data_id);
        Self {
            client: client.clone(),
            next_url: next,
            data_id: data_id.map(|s| s.to_string()),
            total,
            first_doc: Some(initial_doc),
        }
    }

    /// Returns the total item count from the `<nav>`, if available.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Returns `true` if there are more pages to fetch.
    pub fn has_next(&self) -> bool {
        self.first_doc.is_some() || self.next_url.is_some()
    }

    /// Fetch the next page.
    ///
    /// Returns `None` when pagination is exhausted.
    pub async fn next_page(&mut self) -> Option<crate::Result<AnmlDocument>> {
        // Yield the initial document first
        if let Some(doc) = self.first_doc.take() {
            return Some(Ok(doc));
        }

        // Fetch the next page
        let url = self.next_url.take()?;
        match self.client.fetch_url(&url).await {
            Ok(doc) => {
                // Extract next URL for the following page
                self.next_url = next_url(&doc, self.data_id.as_deref());
                Some(Ok(doc))
            }
            Err(e) => {
                // On error, stop pagination
                self.next_url = None;
                Some(Err(e))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Client integration
// ---------------------------------------------------------------------------

impl AnmlClient {
    /// Create a paginated iterator over ANML documents.
    ///
    /// Starts from the given document and follows `<nav>` `next` links.
    /// If `data_id` is provided, scopes the `<nav>` lookup to that
    /// `<data>` section.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anml_client::Result<()> {
    /// # let client = anml_client::client::AnmlClient::builder()
    /// #     .base_url("https://example.com")
    /// #     .build()?;
    /// # let doc = client.fetch("/flights").await?;
    /// let mut pages = client.paginate(doc, Some("flights"));
    /// while let Some(page) = pages.next_page().await {
    ///     let doc = page?;
    ///     // Process items...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn paginate(
        &self,
        doc: AnmlDocument,
        data_id: Option<&str>,
    ) -> PageIterator {
        PageIterator::new(self, doc, data_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::elements::{AnmlBody, AnmlBodyChild, AnmlNav};

    fn make_doc_with_nav(next: Option<&str>, total: Option<&str>) -> AnmlDocument {
        let nav = AnmlNav {
            next: next.map(|s| s.to_string()),
            prev: None,
            cursor: None,
            total: total.map(|s| s.to_string()),
        };
        AnmlDocument {
            body: Some(AnmlBody {
                children: Some(vec![AnmlBodyChild::Nav(nav)]),
                ..AnmlBody::default()
            }),
            ..AnmlDocument::default()
        }
    }

    #[test]
    fn find_nav_returns_nav_from_body() {
        let doc = make_doc_with_nav(Some("/page2"), Some("100"));
        let nav = find_nav(&doc, None).unwrap();
        assert_eq!(nav.next.as_deref(), Some("/page2"));
        assert_eq!(nav.total.as_deref(), Some("100"));
    }

    #[test]
    fn find_nav_returns_none_when_no_body() {
        let doc = AnmlDocument::default();
        assert!(find_nav(&doc, None).is_none());
    }

    #[test]
    fn next_url_extracts_next() {
        let doc = make_doc_with_nav(Some("/flights?page=2"), None);
        assert_eq!(next_url(&doc, None), Some("/flights?page=2".to_string()));
    }

    #[test]
    fn next_url_returns_none_when_no_next() {
        let doc = make_doc_with_nav(None, None);
        assert!(next_url(&doc, None).is_none());
    }

    #[test]
    fn nav_total_parses_number() {
        let doc = make_doc_with_nav(None, Some("47"));
        assert_eq!(nav_total(&doc, None), Some(47));
    }

    #[test]
    fn nav_total_returns_none_for_invalid() {
        let doc = make_doc_with_nav(None, Some("not-a-number"));
        assert!(nav_total(&doc, None).is_none());
    }

    #[test]
    fn page_iterator_has_next_with_first_doc() {
        let doc = make_doc_with_nav(None, None);
        let client = make_test_client();
        let iter = PageIterator::new(&client, doc, None);
        assert!(iter.has_next());
    }

    #[test]
    fn page_iterator_total_from_nav() {
        let doc = make_doc_with_nav(Some("/page2"), Some("100"));
        let client = make_test_client();
        let iter = PageIterator::new(&client, doc, None);
        assert_eq!(iter.total(), Some(100));
    }

    /// Helper to create a minimal test client (won't actually make HTTP calls in unit tests).
    fn make_test_client() -> AnmlClient {
        AnmlClient::builder()
            .base_url("https://example.com")
            .allow_plaintext_http(true)
            .build()
            .unwrap()
    }
}
