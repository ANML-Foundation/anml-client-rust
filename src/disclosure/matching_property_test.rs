//! Property-based tests for disclosure rule matching.
//!
//! **Validates: Requirements 7.1** (disclosure matching precedence)

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::disclosure::matching::*;
    use anml::types::enums::DisclosureRequires;

    // -----------------------------------------------------------------------
    // Arbitrary generators
    // -----------------------------------------------------------------------

    fn arb_field_name() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-z]{1,8}", 1..=3)
            .prop_map(|parts| parts.join("."))
    }

    // -----------------------------------------------------------------------
    // Properties
    // -----------------------------------------------------------------------

    proptest! {
        /// Exact match always wins over prefix, pattern, and default.
        #[test]
        fn exact_wins_over_all(field in arb_field_name()) {
            let exact_rule = DisclosureRule {
                selector: FieldSelector::Exact(field.clone()),
                requires: DisclosureRequires::ExplicitConsent,
                consent_scope: ConsentScope::Session,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 10, // later in doc order
            };
            // Build a prefix that could match
            let prefix = field.split('.').next().unwrap_or(&field).to_string();
            let prefix_rule = DisclosureRule {
                selector: FieldSelector::Prefix(prefix),
                requires: DisclosureRequires::None,
                consent_scope: ConsentScope::Global,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 0, // earlier in doc order
            };
            let default_rule = DisclosureRule {
                selector: FieldSelector::Default,
                requires: DisclosureRequires::None,
                consent_scope: ConsentScope::Global,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 1,
            };

            let rules = vec![prefix_rule, default_rule, exact_rule];
            let result = resolve_rule(&field, &rules);
            prop_assert!(!result.synthesized);
            prop_assert_eq!(result.rule.unwrap().selector.clone(), FieldSelector::Exact(field));
        }

        /// Longest prefix wins among prefix rules.
        #[test]
        fn longest_prefix_wins(
            base in "[a-z]{2,4}",
            mid in "[a-z]{2,4}",
            leaf in "[a-z]{2,4}",
        ) {
            let field = format!("{}.{}.{}", base, mid, leaf);
            let short_prefix = DisclosureRule {
                selector: FieldSelector::Prefix(base.clone()),
                requires: DisclosureRequires::None,
                consent_scope: ConsentScope::Session,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 0,
            };
            let long_prefix = DisclosureRule {
                selector: FieldSelector::Prefix(format!("{}.{}", base, mid)),
                requires: DisclosureRequires::ExplicitConsent,
                consent_scope: ConsentScope::Origin,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 1,
            };

            let rules = vec![short_prefix, long_prefix.clone()];
            let result = resolve_rule(&field, &rules);
            prop_assert!(!result.synthesized);
            prop_assert_eq!(result.rule.unwrap().selector.clone(), long_prefix.selector);
        }

        /// Default rule only matches when no exact, prefix, or pattern matches.
        #[test]
        fn default_only_when_no_other_match(field in "[a-z]{3,6}") {
            let default_rule = DisclosureRule {
                selector: FieldSelector::Default,
                requires: DisclosureRequires::None,
                consent_scope: ConsentScope::Session,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 0,
            };
            // An exact rule for a different field
            let other_exact = DisclosureRule {
                selector: FieldSelector::Exact(format!("other_{}", field)),
                requires: DisclosureRequires::ExplicitConsent,
                consent_scope: ConsentScope::Session,
                rate_limit: None,
                tokenize: false,
                purpose: None,
                document_order: 1,
            };

            let rules = vec![other_exact, default_rule];
            let result = resolve_rule(&field, &rules);
            prop_assert!(!result.synthesized);
            prop_assert_eq!(result.rule.unwrap().selector.clone(), FieldSelector::Default);
        }

        /// Missing rule synthesizes explicit/session.
        #[test]
        fn missing_rule_synthesizes(field in arb_field_name()) {
            let rules: Vec<DisclosureRule> = vec![];
            let result = resolve_rule(&field, &rules);
            prop_assert!(result.synthesized);
            prop_assert!(result.rule.is_none());
        }

        /// Mutual exclusion: field+prefix, field+pattern, prefix+pattern never coexist.
        #[test]
        fn mutual_exclusion_rejects_pairs(
            has_field in any::<bool>(),
            has_prefix in any::<bool>(),
            has_pattern in any::<bool>(),
        ) {
            let count = has_field as u8 + has_prefix as u8 + has_pattern as u8;
            let result = DisclosureRule::validate_mutual_exclusion(has_field, has_prefix, has_pattern);
            if count > 1 {
                prop_assert!(result.is_err());
            } else {
                prop_assert!(result.is_ok());
            }
        }
    }
}
