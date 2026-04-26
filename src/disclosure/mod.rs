//! Disclosure evaluation engine implementing the RFC 7-step algorithm.
//!
//! This module contains:
//! - [`matching`] — field matching with RFC precedence
//! - [`consent`] — consent store with session/origin/global scoping
//! - [`rate_limit`] — per-field 24-hour sliding window rate limiter
//! - [`evaluate`] — the full 7-step disclosure evaluation algorithm

pub mod matching;
pub mod consent;
pub mod rate_limit;

#[cfg(test)]
mod matching_property_test;
#[cfg(test)]
mod consent_property_test;

use anml::types::document::AnmlDocument;
use anml::types::enums::DisclosureRequires;
use tracing::{debug, warn};

use crate::config::{
    AuthProvider, ConsentDecision, ConsentHandler, Origin, TrustDecision, TrustPolicy,
};

pub use consent::{ConsentBasis, ConsentStore};
pub use matching::{ConsentScope, DisclosureRule, FieldSelector};
pub use rate_limit::RateLimitTracker;

// ---------------------------------------------------------------------------
// DisclosureDecision
// ---------------------------------------------------------------------------

/// The outcome of the 7-step disclosure evaluation algorithm.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DisclosureDecision {
    /// Disclosure is allowed.
    Allow {
        /// The (possibly tokenized) value to disclose.
        value: String,
        /// The consent basis under which disclosure was authorized.
        consent_basis: ConsentBasis,
        /// Whether the value was tokenized.
        tokenized: bool,
    },
    /// Disclosure is denied.
    Deny {
        /// The field that was denied.
        field: String,
        /// The reason for denial.
        reason: String,
        /// The `<refuse>` reason attribute value.
        refuse_reason: RefuseReason,
    },
}

/// RFC refuse reason codes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefuseReason {
    /// Constraint violation.
    ConstraintViolation,
    /// User denied consent.
    UserDenied,
    /// Policy violation.
    PolicyViolation,
    /// Unsupported field.
    UnsupportedField,
    /// Trust insufficient.
    TrustInsufficient,
    /// Rate limited.
    RateLimited {
        /// Seconds until retry is allowed.
        retry_after: Option<u64>,
    },
    /// Not available.
    NotAvailable,
}

impl std::fmt::Display for RefuseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConstraintViolation => write!(f, "constraint-violation"),
            Self::UserDenied => write!(f, "user-denied"),
            Self::PolicyViolation => write!(f, "policy-violation"),
            Self::UnsupportedField => write!(f, "unsupported-field"),
            Self::TrustInsufficient => write!(f, "trust-insufficient"),
            Self::RateLimited { .. } => write!(f, "rate-limited"),
            Self::NotAvailable => write!(f, "not-available"),
        }
    }
}

// ---------------------------------------------------------------------------
// DisclosureContext — dependencies for the 7-step algorithm
// ---------------------------------------------------------------------------

/// Context required to run the disclosure evaluation algorithm.
///
/// Bundles references to the consent store, rate limiter, trust policy,
/// auth provider, consent handler, and tokenizer so that `evaluate` is
/// a pure function of its inputs.
pub struct DisclosureContext<'a> {
    /// The origin of the document.
    pub origin: &'a Origin,
    /// The consent store.
    pub consent_store: &'a ConsentStore,
    /// The rate limit tracker.
    pub rate_limiter: &'a RateLimitTracker,
    /// The trust policy.
    pub trust_policy: &'a dyn TrustPolicy,
    /// Optional auth provider (for `authentication` consent requirement).
    pub auth_provider: Option<&'a dyn AuthProvider>,
    /// Optional consent handler (for `explicit` consent requirement).
    pub consent_handler: Option<&'a dyn ConsentHandler>,
    /// Optional tokenizer for HMAC-SHA256 tokenization.
    pub tokenizer: Option<&'a crate::security::Tokenizer>,
    /// Optional principal ID for tokenization binding.
    pub principal_id: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// evaluate — the 7-step RFC disclosure algorithm
// ---------------------------------------------------------------------------

/// Execute the full 7-step RFC disclosure evaluation algorithm.
///
/// This function MUST be called before emitting any `<answer>`. On any
/// failure, it returns `DisclosureDecision::Deny` with the appropriate
/// refuse reason. No step may be skipped.
///
/// # Arguments
///
/// * `doc` — The ANML document containing the `<constraints>` and `<knowledge>` sections.
/// * `rules` — The parsed disclosure rules from the document's `<constraints>`.
/// * `field` — The field name being disclosed.
/// * `value` — The value to disclose.
/// * `ctx` — The disclosure context with all required dependencies.
///
/// # Steps
///
/// 1. Resolve the rule via matching precedence
/// 2. Check rate limit
/// 3. Check consent
/// 4. Check trust policy
/// 5. Validate value against `<ask>` constraints
/// 6. Tokenize if requested
/// 7. Emit (return Allow decision)
pub fn evaluate(
    doc: &AnmlDocument,
    rules: &[DisclosureRule],
    field: &str,
    value: &str,
    ctx: &DisclosureContext<'_>,
) -> DisclosureDecision {
    // Step 1: Resolve the rule
    let match_result = matching::resolve_rule(field, rules);
    let (rule_requires, consent_scope, rate_limit, tokenize, rule_ref) = if match_result.synthesized
    {
        warn!(
            field,
            "no disclosure rule found; synthesizing requires=explicit, consent-scope=session"
        );
        (
            DisclosureRequires::ExplicitConsent,
            ConsentScope::Session,
            None,
            false,
            "(synthesized)".to_string(),
        )
    } else {
        let rule = match_result.rule.unwrap();
        let rule_ref = match &rule.selector {
            FieldSelector::Exact(f) => format!("field={}", f),
            FieldSelector::Prefix(p) => format!("field-prefix={}", p),
            FieldSelector::Pattern(p) => format!("field-pattern={}", p),
            FieldSelector::Default => "default".to_string(),
        };
        (
            rule.requires,
            rule.consent_scope,
            rule.rate_limit,
            rule.tokenize,
            rule_ref,
        )
    };

    debug!(field, %rule_ref, "step 1: resolved disclosure rule");

    // Step 2: Check rate limit
    if let Some(max) = rate_limit {
        if let Err(retry_after) = ctx.rate_limiter.check_and_record(field, ctx.origin, max) {
            debug!(field, retry_after, "step 2: rate limit exceeded");
            return DisclosureDecision::Deny {
                field: field.to_string(),
                reason: format!(
                    "rate limit exceeded for field '{}': max {} per 24h",
                    field, max
                ),
                refuse_reason: RefuseReason::RateLimited {
                    retry_after: Some(retry_after),
                },
            };
        }
        debug!(field, "step 2: rate limit check passed");
    } else {
        debug!(field, "step 2: no rate limit configured");
    }

    // Step 3: Check consent
    let consent_basis = match check_consent(
        field,
        ctx.origin,
        rule_requires,
        consent_scope,
        ctx.consent_store,
        ctx.consent_handler,
        ctx.auth_provider,
    ) {
        Ok(basis) => {
            debug!(field, %basis, "step 3: consent check passed");
            basis
        }
        Err(decision) => return decision,
    };

    // Step 4: Check trust policy
    let trust_decision = ctx.trust_policy.evaluate(ctx.origin, doc);
    match trust_decision {
        TrustDecision::Allow => {
            debug!(field, "step 4: trust policy allows");
        }
        TrustDecision::Deny { reason } => {
            debug!(field, %reason, "step 4: trust policy denied");
            return DisclosureDecision::Deny {
                field: field.to_string(),
                reason: format!("trust policy denied: {}", reason),
                refuse_reason: RefuseReason::TrustInsufficient,
            };
        }
    }

    // Step 5: Validate value against <ask> constraints
    if let Err(decision) = validate_value(doc, field, value) {
        debug!(field, "step 5: value validation failed");
        return decision;
    }
    debug!(field, "step 5: value validation passed");

    // Step 6: Tokenize if requested
    let (final_value, is_tokenized) = if tokenize {
        if let (Some(tokenizer), Some(principal_id)) = (ctx.tokenizer, ctx.principal_id) {
            let token = tokenizer.tokenize(field, principal_id, ctx.origin);
            debug!(field, "step 6: value tokenized");
            (token, true)
        } else {
            warn!(
                field,
                "step 6: tokenize=true but no tokenizer or principal_id configured"
            );
            (value.to_string(), false)
        }
    } else {
        debug!(field, "step 6: no tokenization requested");
        (value.to_string(), false)
    };

    // Step 7: Emit (audit recording is done by the caller)
    debug!(field, %consent_basis, tokenized = is_tokenized, "step 7: disclosure allowed");
    DisclosureDecision::Allow {
        value: final_value,
        consent_basis,
        tokenized: is_tokenized,
    }
}

// ---------------------------------------------------------------------------
// Step 3 helper: consent checking
// ---------------------------------------------------------------------------

fn check_consent(
    field: &str,
    origin: &Origin,
    requires: DisclosureRequires,
    scope: ConsentScope,
    consent_store: &ConsentStore,
    consent_handler: Option<&dyn ConsentHandler>,
    auth_provider: Option<&dyn AuthProvider>,
) -> Result<ConsentBasis, DisclosureDecision> {
    match requires {
        DisclosureRequires::ExplicitConsent => {
            // Always invoke the consent handler for explicit consent
            if let Some(handler) = consent_handler {
                let decision = handler.request_consent(field, origin, None);
                match decision {
                    ConsentDecision::Grant => {
                        // Record the grant in the consent store
                        consent_store.grant(field, origin, scope, ConsentBasis::Explicit);
                        Ok(ConsentBasis::Explicit)
                    }
                    ConsentDecision::Deny => Err(DisclosureDecision::Deny {
                        field: field.to_string(),
                        reason: format!(
                            "principal denied explicit consent for field '{}'",
                            field
                        ),
                        refuse_reason: RefuseReason::UserDenied,
                    }),
                }
            } else {
                Err(DisclosureDecision::Deny {
                    field: field.to_string(),
                    reason: format!(
                        "field '{}' requires explicit consent but no consent handler configured",
                        field
                    ),
                    refuse_reason: RefuseReason::ConstraintViolation,
                })
            }
        }
        DisclosureRequires::ImplicitConsent => {
            // Check the consent store for a prior standing grant
            if let Some(basis) = consent_store.check(field, origin, scope) {
                Ok(basis)
            } else {
                // No standing grant — fall back to explicit consent
                if let Some(handler) = consent_handler {
                    let decision = handler.request_consent(field, origin, None);
                    match decision {
                        ConsentDecision::Grant => {
                            consent_store.grant(field, origin, scope, ConsentBasis::Implicit);
                            Ok(ConsentBasis::Implicit)
                        }
                        ConsentDecision::Deny => Err(DisclosureDecision::Deny {
                            field: field.to_string(),
                            reason: format!(
                                "no standing consent for field '{}' and principal denied",
                                field
                            ),
                            refuse_reason: RefuseReason::UserDenied,
                        }),
                    }
                } else {
                    Err(DisclosureDecision::Deny {
                        field: field.to_string(),
                        reason: format!(
                            "field '{}' requires implicit consent but no standing grant and no handler",
                            field
                        ),
                        refuse_reason: RefuseReason::ConstraintViolation,
                    })
                }
            }
        }
        DisclosureRequires::Authentication => {
            // Verify an authenticated session exists via AuthProvider
            if auth_provider.is_some() {
                // We can't call async from sync context, so we check if
                // auth provider exists as a proxy for "authenticated session".
                // The actual credential check happens at HTTP dispatch time.
                Ok(ConsentBasis::Delegated)
            } else {
                Err(DisclosureDecision::Deny {
                    field: field.to_string(),
                    reason: format!(
                        "field '{}' requires authentication but no auth provider configured",
                        field
                    ),
                    refuse_reason: RefuseReason::ConstraintViolation,
                })
            }
        }
        DisclosureRequires::None => {
            // No consent check required, but trust policy still applies (step 4)
            Ok(ConsentBasis::Implicit)
        }
        _ => {
            // Unknown/future disclosure requirement — treat as explicit
            if let Some(handler) = consent_handler {
                let decision = handler.request_consent(field, origin, None);
                match decision {
                    ConsentDecision::Grant => {
                        consent_store.grant(field, origin, scope, ConsentBasis::Explicit);
                        Ok(ConsentBasis::Explicit)
                    }
                    ConsentDecision::Deny => Err(DisclosureDecision::Deny {
                        field: field.to_string(),
                        reason: format!(
                            "principal denied consent for field '{}' (unknown requirement)",
                            field
                        ),
                        refuse_reason: RefuseReason::UserDenied,
                    }),
                }
            } else {
                Err(DisclosureDecision::Deny {
                    field: field.to_string(),
                    reason: format!(
                        "field '{}' has unknown disclosure requirement and no consent handler",
                        field
                    ),
                    refuse_reason: RefuseReason::ConstraintViolation,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 5 helper: value validation
// ---------------------------------------------------------------------------

fn validate_value(
    doc: &AnmlDocument,
    field: &str,
    value: &str,
) -> Result<(), DisclosureDecision> {
    // Find the <ask> for this field to check type/pattern/one-of constraints
    let ask = find_ask(doc, field);
    let ask = match ask {
        Some(a) => a,
        None => return Ok(()), // No ask found — no validation constraints
    };

    // Check type constraint
    if let Some(ref ask_type) = ask.ask_type {
        if !validate_type(value, ask_type) {
            return Err(DisclosureDecision::Deny {
                field: field.to_string(),
                reason: format!(
                    "value '{}' does not match expected type '{}'",
                    value, ask_type
                ),
                refuse_reason: RefuseReason::UnsupportedField,
            });
        }
    }

    // Note: pattern and one-of are not currently on AnmlAsk in the anml crate.
    // When they are added, validation would go here.

    Ok(())
}

fn find_ask<'a>(
    doc: &'a AnmlDocument,
    field: &str,
) -> Option<&'a anml::types::elements::AnmlAsk> {
    if let Some(ref knowledge) = doc.knowledge {
        if let Some(ref asks) = knowledge.asks {
            return asks.iter().find(|a| a.field == field);
        }
    }
    // Also check top-level asks
    if let Some(ref asks) = doc.asks {
        return asks.iter().find(|a| a.field == field);
    }
    None
}

fn validate_type(value: &str, field_type: &anml::types::enums::FieldType) -> bool {
    use anml::types::enums::FieldType;
    match field_type {
        FieldType::String => true, // Any string is valid
        FieldType::Number => value.parse::<f64>().is_ok(),
        FieldType::Boolean => matches!(value, "true" | "false"),
        FieldType::Date => {
            // Simple ISO 8601 date check (YYYY-MM-DD)
            value.len() == 10
                && value.chars().nth(4) == Some('-')
                && value.chars().nth(7) == Some('-')
                && value[..4].parse::<u16>().is_ok()
                && value[5..7].parse::<u8>().is_ok()
                && value[8..10].parse::<u8>().is_ok()
        }
        FieldType::Datetime => {
            // Simple check: contains 'T' and looks like ISO 8601
            value.contains('T') && value.len() >= 19
        }
        FieldType::Uri => {
            // Basic URI check
            url::Url::parse(value).is_ok()
        }
        _ => true, // Unknown types pass validation
    }
}

// Step 6 tokenization is now handled by `crate::security::Tokenizer`.

// ---------------------------------------------------------------------------
// Extract disclosure rules from an AnmlDocument
// ---------------------------------------------------------------------------

/// Extract disclosure rules from an `AnmlDocument`'s `<constraints>` section.
///
/// The `anml` crate's `AnmlDisclosure` only has `field` and `requires`.
/// This function maps them to our extended `DisclosureRule` with default
/// values for the additional attributes (consent-scope=session, no rate-limit,
/// no tokenize).
pub fn extract_rules(doc: &AnmlDocument) -> Vec<DisclosureRule> {
    let mut rules = Vec::new();
    if let Some(ref constraints) = doc.constraints {
        if let Some(ref disclosures) = constraints.disclosures {
            for (i, d) in disclosures.iter().enumerate() {
                rules.push(DisclosureRule {
                    selector: FieldSelector::Exact(d.field.clone()),
                    requires: d.requires,
                    consent_scope: ConsentScope::Session,
                    rate_limit: None,
                    tokenize: false,
                    purpose: None,
                    document_order: i,
                });
            }
        }
    }
    rules
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::document::AnmlDocument;
    use anml::types::elements::{AnmlAsk, AnmlConstraints, AnmlDisclosure, AnmlKnowledge};
    use anml::types::enums::DisclosureRequires;

    fn test_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        }
    }

    /// A trust policy that allows everything.
    struct AllowAllPolicy;
    impl TrustPolicy for AllowAllPolicy {
        fn evaluate(&self, _origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
            TrustDecision::Allow
        }
    }

    /// A trust policy that denies everything.
    struct DenyAllPolicy;
    impl TrustPolicy for DenyAllPolicy {
        fn evaluate(&self, _origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
            TrustDecision::Deny {
                reason: "denied by test policy".into(),
            }
        }
    }

    /// A consent handler that always grants.
    struct AlwaysGrantHandler;
    impl ConsentHandler for AlwaysGrantHandler {
        fn request_consent(
            &self,
            _field: &str,
            _origin: &Origin,
            _purpose: Option<&str>,
        ) -> ConsentDecision {
            ConsentDecision::Grant
        }
    }

    /// A consent handler that always denies.
    struct AlwaysDenyHandler;
    impl ConsentHandler for AlwaysDenyHandler {
        fn request_consent(
            &self,
            _field: &str,
            _origin: &Origin,
            _purpose: Option<&str>,
        ) -> ConsentDecision {
            ConsentDecision::Deny
        }
    }

    fn make_doc_with_ask(field: &str) -> AnmlDocument {
        AnmlDocument {
            knowledge: Some(AnmlKnowledge {
                asks: Some(vec![AnmlAsk {
                    field: field.to_string(),
                    action: "submit".to_string(),
                    required: None,
                    purpose: None,
                    ask_type: None,
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn make_doc_with_disclosure(field: &str, requires: DisclosureRequires) -> AnmlDocument {
        AnmlDocument {
            constraints: Some(AnmlConstraints {
                disclosures: Some(vec![AnmlDisclosure {
                    field: field.to_string(),
                    requires,
                }]),
            }),
            knowledge: Some(AnmlKnowledge {
                asks: Some(vec![AnmlAsk {
                    field: field.to_string(),
                    action: "submit".to_string(),
                    required: None,
                    purpose: None,
                    ask_type: None,
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn make_context<'a>(
        origin: &'a Origin,
        consent_store: &'a ConsentStore,
        rate_limiter: &'a RateLimitTracker,
        trust_policy: &'a dyn TrustPolicy,
        consent_handler: Option<&'a dyn ConsentHandler>,
    ) -> DisclosureContext<'a> {
        DisclosureContext {
            origin,
            consent_store,
            rate_limiter,
            trust_policy,
            auth_provider: None,
            consent_handler,
            tokenizer: None,
            principal_id: None,
        }
    }

    // -- Step 1: Rule resolution --

    #[test]
    fn evaluate_with_no_rules_synthesizes_explicit() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, Some(&handler));

        let doc = make_doc_with_ask("email");
        let rules: Vec<DisclosureRule> = vec![];
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(result, DisclosureDecision::Allow { .. }));
    }

    // -- Step 2: Rate limit --

    #[test]
    fn evaluate_rate_limited() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, Some(&handler));

        let doc = make_doc_with_ask("email");
        let rules = vec![DisclosureRule {
            selector: FieldSelector::Exact("email".into()),
            requires: DisclosureRequires::None,
            consent_scope: ConsentScope::Session,
            rate_limit: Some(1),
            tokenize: false,
            purpose: None,
            document_order: 0,
        }];

        // First disclosure should succeed
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(result, DisclosureDecision::Allow { .. }));

        // Second should be rate limited
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(
            result,
            DisclosureDecision::Deny {
                refuse_reason: RefuseReason::RateLimited { .. },
                ..
            }
        ));
    }

    // -- Step 3: Consent --

    #[test]
    fn evaluate_explicit_consent_granted() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, Some(&handler));

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        match result {
            DisclosureDecision::Allow { consent_basis, .. } => {
                assert_eq!(consent_basis, ConsentBasis::Explicit);
            }
            _ => panic!("expected Allow"),
        }
    }

    #[test]
    fn evaluate_explicit_consent_denied() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysDenyHandler;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, Some(&handler));

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(
            result,
            DisclosureDecision::Deny {
                refuse_reason: RefuseReason::UserDenied,
                ..
            }
        ));
    }

    #[test]
    fn evaluate_no_consent_handler_for_explicit() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(
            result,
            DisclosureDecision::Deny {
                refuse_reason: RefuseReason::ConstraintViolation,
                ..
            }
        ));
    }

    #[test]
    fn evaluate_implicit_with_standing_grant() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        consent_store.grant("email", &origin, ConsentScope::Session, ConsentBasis::Implicit);
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ImplicitConsent);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(result, DisclosureDecision::Allow { .. }));
    }

    #[test]
    fn evaluate_none_requires_no_consent() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::None);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(result, DisclosureDecision::Allow { .. }));
    }

    // -- Step 4: Trust policy --

    #[test]
    fn evaluate_trust_denied() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = DenyAllPolicy;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::None);
        let rules = extract_rules(&doc);
        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(
            result,
            DisclosureDecision::Deny {
                refuse_reason: RefuseReason::TrustInsufficient,
                ..
            }
        ));
    }

    // -- Step 5: Value validation --

    #[test]
    fn evaluate_type_validation_number() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let ctx = make_context(&origin, &consent_store, &rate_limiter, &policy, None);

        let mut doc = make_doc_with_disclosure("age", DisclosureRequires::None);
        // Set the ask type to Number
        if let Some(ref mut knowledge) = doc.knowledge {
            if let Some(ref mut asks) = knowledge.asks {
                asks[0].ask_type = Some(anml::types::enums::FieldType::Number);
            }
        }
        let rules = extract_rules(&doc);

        // Valid number
        let result = evaluate(&doc, &rules, "age", "25", &ctx);
        assert!(matches!(result, DisclosureDecision::Allow { .. }));

        // Invalid number
        let result = evaluate(&doc, &rules, "age", "not-a-number", &ctx);
        assert!(matches!(
            result,
            DisclosureDecision::Deny {
                refuse_reason: RefuseReason::UnsupportedField,
                ..
            }
        ));
    }

    // -- Step 6: Tokenization --

    #[test]
    fn evaluate_tokenization() {
        let origin = test_origin();
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let tokenizer = crate::security::Tokenizer::from_secret([42u8; 32]);
        let ctx = DisclosureContext {
            origin: &origin,
            consent_store: &consent_store,
            rate_limiter: &rate_limiter,
            trust_policy: &policy,
            auth_provider: None,
            consent_handler: None,
            tokenizer: Some(&tokenizer),
            principal_id: Some("user-123"),
        };

        let doc = make_doc_with_ask("email");
        let rules = vec![DisclosureRule {
            selector: FieldSelector::Exact("email".into()),
            requires: DisclosureRequires::None,
            consent_scope: ConsentScope::Session,
            rate_limit: None,
            tokenize: true,
            purpose: None,
            document_order: 0,
        }];

        let result = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        match result {
            DisclosureDecision::Allow {
                value,
                tokenized,
                ..
            } => {
                assert!(tokenized);
                assert_ne!(value, "test@example.com");
                // Token should be hex-encoded
                assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
            }
            _ => panic!("expected Allow"),
        }
    }

    // -- extract_rules --

    #[test]
    fn extract_rules_from_doc() {
        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let rules = extract_rules(&doc);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, FieldSelector::Exact("email".into()));
        assert_eq!(rules[0].requires, DisclosureRequires::ExplicitConsent);
    }

    #[test]
    fn extract_rules_empty_constraints() {
        let doc = AnmlDocument::default();
        let rules = extract_rules(&doc);
        assert!(rules.is_empty());
    }

    // -- validate_type --

    #[test]
    fn validate_type_string() {
        assert!(validate_type("anything", &anml::types::enums::FieldType::String));
    }

    #[test]
    fn validate_type_boolean() {
        assert!(validate_type("true", &anml::types::enums::FieldType::Boolean));
        assert!(validate_type("false", &anml::types::enums::FieldType::Boolean));
        assert!(!validate_type("yes", &anml::types::enums::FieldType::Boolean));
    }

    #[test]
    fn validate_type_date() {
        assert!(validate_type("2026-01-15", &anml::types::enums::FieldType::Date));
        assert!(!validate_type("not-a-date", &anml::types::enums::FieldType::Date));
    }

    #[test]
    fn validate_type_uri() {
        assert!(validate_type(
            "https://example.com",
            &anml::types::enums::FieldType::Uri
        ));
        assert!(!validate_type("not a uri", &anml::types::enums::FieldType::Uri));
    }
}