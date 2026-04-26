//! Confidentiality and usage rights accessors.
//!
//! Provides convenience accessors for `<inform>` confidentiality levels,
//! `<rights>` declarations, `<attribution>` requirements, and a
//! `usage_permitted` helper that checks against the usage hierarchy:
//! `none < display < cache < store < train`.

use anml::types::document::AnmlDocument;
use anml::types::elements::{AnmlAttribution, AnmlFooter, AnmlInform, AnmlRights};
use anml::types::enums::{ConfidentialityLevel, UsageType};

// ---------------------------------------------------------------------------
// Confidentiality accessors on AnmlInform
// ---------------------------------------------------------------------------

/// Extension trait providing confidentiality accessors on `AnmlInform`.
pub trait InformConfidentiality {
    /// Returns the confidentiality level, defaulting to `Public` if not set.
    fn confidentiality_level(&self) -> ConfidentialityLevel;

    /// Returns `true` if the inform is marked `private` (MUST NOT be forwarded).
    fn is_private(&self) -> bool;

    /// Returns `true` if the inform is marked `restricted` (SHOULD NOT be
    /// forwarded to third parties without principal approval).
    fn is_restricted(&self) -> bool;

    /// Returns `true` if the inform is `public` (no forwarding restrictions).
    fn is_public(&self) -> bool;
}

impl InformConfidentiality for AnmlInform {
    fn confidentiality_level(&self) -> ConfidentialityLevel {
        self.confidentiality.unwrap_or(ConfidentialityLevel::Public)
    }

    fn is_private(&self) -> bool {
        self.confidentiality_level() == ConfidentialityLevel::Private
    }

    fn is_restricted(&self) -> bool {
        self.confidentiality_level() == ConfidentialityLevel::Restricted
    }

    fn is_public(&self) -> bool {
        self.confidentiality_level() == ConfidentialityLevel::Public
    }
}

// ---------------------------------------------------------------------------
// Rights accessors
// ---------------------------------------------------------------------------

/// Extension trait providing rights accessors on `AnmlRights`.
pub trait RightsAccessors {
    /// Returns the rights holder name.
    fn holder_name(&self) -> &str;

    /// Returns the copyright year, if declared.
    fn copyright_year(&self) -> Option<&str>;

    /// Returns the license identifier, if declared.
    fn license_id(&self) -> Option<&str>;

    /// Returns the usage type granted by these rights.
    fn usage_level(&self) -> UsageType;

    /// Returns the scope (element ID or `"*"`), if declared.
    fn rights_scope(&self) -> Option<&str>;
}

impl RightsAccessors for AnmlRights {
    fn holder_name(&self) -> &str {
        &self.holder
    }

    fn copyright_year(&self) -> Option<&str> {
        self.year.as_deref()
    }

    fn license_id(&self) -> Option<&str> {
        self.license.as_deref()
    }

    fn usage_level(&self) -> UsageType {
        self.usage
    }

    fn rights_scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Attribution accessors
// ---------------------------------------------------------------------------

/// Extension trait providing attribution accessors on `AnmlAttribution`.
pub trait AttributionAccessors {
    /// Returns `true` if attribution is required.
    fn is_required(&self) -> bool;

    /// Returns the scope of the attribution requirement, if declared.
    fn attribution_scope(&self) -> Option<&str>;

    /// Returns the attribution text, if present.
    fn attribution_text(&self) -> Option<&str>;
}

impl AttributionAccessors for AnmlAttribution {
    fn is_required(&self) -> bool {
        self.required.unwrap_or(false)
    }

    fn attribution_scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    fn attribution_text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Document-level accessors
// ---------------------------------------------------------------------------

/// Extension trait providing rights and attribution accessors on `AnmlDocument`.
pub trait DocumentRights {
    /// Returns all `<rights>` declarations from the footer.
    fn rights(&self) -> Vec<&AnmlRights>;

    /// Returns all `<attribution>` declarations from the footer.
    fn attributions(&self) -> Vec<&AnmlAttribution>;

    /// Returns the footer, if present.
    fn footer_ref(&self) -> Option<&AnmlFooter>;

    /// Returns the most permissive usage level declared in the document's
    /// `<rights>` elements. Returns `UsageType::None` if no rights are declared.
    fn max_usage_level(&self) -> UsageType;
}

impl DocumentRights for AnmlDocument {
    fn rights(&self) -> Vec<&AnmlRights> {
        self.footer
            .as_ref()
            .and_then(|f| f.rights.as_ref())
            .map(|r| r.iter().collect())
            .unwrap_or_default()
    }

    fn attributions(&self) -> Vec<&AnmlAttribution> {
        self.footer
            .as_ref()
            .and_then(|f| f.attributions.as_ref())
            .map(|a| a.iter().collect())
            .unwrap_or_default()
    }

    fn footer_ref(&self) -> Option<&AnmlFooter> {
        self.footer.as_ref()
    }

    fn max_usage_level(&self) -> UsageType {
        self.rights()
            .iter()
            .map(|r| r.usage)
            .max()
            .unwrap_or(UsageType::None)
    }
}

// ---------------------------------------------------------------------------
// Usage permission check
// ---------------------------------------------------------------------------

/// Check whether a requested usage level is permitted by the declared level.
///
/// The usage hierarchy is: `none < display < cache < store < train`.
/// A higher declared level subsumes every lower permission. This function
/// returns `true` if `requested <= declared`.
///
/// # Examples
///
/// ```
/// use anml::types::enums::UsageType;
/// use anml_client::rights::usage_permitted;
///
/// assert!(usage_permitted(UsageType::Display, UsageType::Cache));
/// assert!(usage_permitted(UsageType::Cache, UsageType::Cache));
/// assert!(!usage_permitted(UsageType::Train, UsageType::Cache));
/// ```
pub fn usage_permitted(requested: UsageType, declared: UsageType) -> bool {
    requested <= declared
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::elements::AnmlFooter;

    // -- InformConfidentiality --

    #[test]
    fn inform_default_confidentiality_is_public() {
        let inform = AnmlInform::default();
        assert_eq!(inform.confidentiality_level(), ConfidentialityLevel::Public);
        assert!(inform.is_public());
        assert!(!inform.is_restricted());
        assert!(!inform.is_private());
    }

    #[test]
    fn inform_private_confidentiality() {
        let inform = AnmlInform {
            confidentiality: Some(ConfidentialityLevel::Private),
            ..AnmlInform::default()
        };
        assert!(inform.is_private());
        assert!(!inform.is_public());
        assert!(!inform.is_restricted());
    }

    #[test]
    fn inform_restricted_confidentiality() {
        let inform = AnmlInform {
            confidentiality: Some(ConfidentialityLevel::Restricted),
            ..AnmlInform::default()
        };
        assert!(inform.is_restricted());
        assert!(!inform.is_public());
        assert!(!inform.is_private());
    }

    // -- RightsAccessors --

    #[test]
    fn rights_accessors() {
        let rights = AnmlRights {
            holder: "Acme Corp".to_string(),
            year: Some("2026".to_string()),
            license: Some("MIT".to_string()),
            usage: UsageType::Cache,
            scope: Some("flights".to_string()),
            text: None,
        };
        assert_eq!(rights.holder_name(), "Acme Corp");
        assert_eq!(rights.copyright_year(), Some("2026"));
        assert_eq!(rights.license_id(), Some("MIT"));
        assert_eq!(rights.usage_level(), UsageType::Cache);
        assert_eq!(rights.rights_scope(), Some("flights"));
    }

    // -- AttributionAccessors --

    #[test]
    fn attribution_default_not_required() {
        let attr = AnmlAttribution::default();
        assert!(!attr.is_required());
        assert!(attr.attribution_scope().is_none());
        assert!(attr.attribution_text().is_none());
    }

    #[test]
    fn attribution_required_with_scope() {
        let attr = AnmlAttribution {
            required: Some(true),
            scope: Some("flights".to_string()),
            text: Some("Data provided by Acme".to_string()),
        };
        assert!(attr.is_required());
        assert_eq!(attr.attribution_scope(), Some("flights"));
        assert_eq!(attr.attribution_text(), Some("Data provided by Acme"));
    }

    // -- DocumentRights --

    #[test]
    fn document_rights_empty_when_no_footer() {
        let doc = AnmlDocument::default();
        assert!(doc.rights().is_empty());
        assert!(doc.attributions().is_empty());
        assert_eq!(doc.max_usage_level(), UsageType::None);
    }

    #[test]
    fn document_rights_from_footer() {
        let doc = AnmlDocument {
            footer: Some(AnmlFooter {
                rights: Some(vec![
                    AnmlRights {
                        holder: "A".to_string(),
                        year: None,
                        license: None,
                        usage: UsageType::Display,
                        scope: None,
                        text: None,
                    },
                    AnmlRights {
                        holder: "B".to_string(),
                        year: None,
                        license: None,
                        usage: UsageType::Store,
                        scope: None,
                        text: None,
                    },
                ]),
                attributions: Some(vec![AnmlAttribution {
                    required: Some(true),
                    scope: None,
                    text: Some("Credit us".to_string()),
                }]),
                text: None,
            }),
            ..AnmlDocument::default()
        };
        assert_eq!(doc.rights().len(), 2);
        assert_eq!(doc.attributions().len(), 1);
        assert_eq!(doc.max_usage_level(), UsageType::Store);
    }

    // -- usage_permitted --

    #[test]
    fn usage_permitted_same_level() {
        assert!(usage_permitted(UsageType::Display, UsageType::Display));
        assert!(usage_permitted(UsageType::Cache, UsageType::Cache));
        assert!(usage_permitted(UsageType::None, UsageType::None));
    }

    #[test]
    fn usage_permitted_lower_than_declared() {
        assert!(usage_permitted(UsageType::None, UsageType::Display));
        assert!(usage_permitted(UsageType::Display, UsageType::Cache));
        assert!(usage_permitted(UsageType::Cache, UsageType::Store));
        assert!(usage_permitted(UsageType::Store, UsageType::Train));
        assert!(usage_permitted(UsageType::None, UsageType::Train));
    }

    #[test]
    fn usage_not_permitted_higher_than_declared() {
        assert!(!usage_permitted(UsageType::Display, UsageType::None));
        assert!(!usage_permitted(UsageType::Cache, UsageType::Display));
        assert!(!usage_permitted(UsageType::Store, UsageType::Cache));
        assert!(!usage_permitted(UsageType::Train, UsageType::Store));
        assert!(!usage_permitted(UsageType::Train, UsageType::None));
    }

    #[test]
    fn usage_hierarchy_complete() {
        let levels = [
            UsageType::None,
            UsageType::Display,
            UsageType::Cache,
            UsageType::Store,
            UsageType::Train,
        ];
        for (i, &lower) in levels.iter().enumerate() {
            for &higher in &levels[i..] {
                assert!(
                    usage_permitted(lower, higher),
                    "{:?} should be permitted under {:?}",
                    lower,
                    higher
                );
            }
            for &higher in &levels[..i] {
                assert!(
                    !usage_permitted(lower, higher),
                    "{:?} should NOT be permitted under {:?}",
                    lower,
                    higher
                );
            }
        }
    }
}
