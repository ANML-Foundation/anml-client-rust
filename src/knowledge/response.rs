//! Answer/Refuse/Ask/Inform builder wrappers with disclosure integration.
//!
//! These functions wrap the `anml` crate's [`ResponseBuilder`] and integrate
//! with the client's disclosure evaluation engine so that every `<answer>`
//! is gated by the full 7-step algorithm.

use anml::builder::ResponseBuilder as AnmlResponseBuilder;
use anml::types::document::AnmlDocument;
use anml::types::elements::{AnmlAnswer, AnmlAsk, AnmlInform, AnmlRefuse};
use anml::types::enums::ConsentType;

use crate::disclosure::{
    evaluate, extract_rules, ConsentBasis, DisclosureContext, DisclosureDecision, DisclosureRule,
    RefuseReason,
};

// ---------------------------------------------------------------------------
// ConsentBasis → ConsentType mapping
// ---------------------------------------------------------------------------

/// Convert our client-side `ConsentBasis` to the `anml` crate's `ConsentType`.
fn consent_basis_to_type(basis: ConsentBasis) -> ConsentType {
    match basis {
        ConsentBasis::Explicit => ConsentType::Explicit,
        ConsentBasis::Implicit => ConsentType::Implicit,
        ConsentBasis::Delegated => ConsentType::Delegated,
    }
}

/// Convert our client-side `RefuseReason` to the `anml` crate's `RefuseReason`.
///
/// The `anml` crate's `RefuseReason` has fewer variants than ours, so we
/// map extended reasons to the closest match.
fn refuse_reason_to_anml(reason: &RefuseReason) -> anml::types::enums::RefuseReason {
    match reason {
        RefuseReason::ConstraintViolation => {
            anml::types::enums::RefuseReason::ConstraintViolation
        }
        RefuseReason::UserDenied => anml::types::enums::RefuseReason::UserDenied,
        RefuseReason::PolicyViolation => anml::types::enums::RefuseReason::PolicyViolation,
        RefuseReason::UnsupportedField => anml::types::enums::RefuseReason::UnsupportedField,
        RefuseReason::TrustInsufficient => anml::types::enums::RefuseReason::TrustInsufficient,
        // RateLimited and NotAvailable don't exist in the anml crate's enum;
        // map to PolicyViolation as the closest semantic match.
        RefuseReason::RateLimited { .. } => anml::types::enums::RefuseReason::PolicyViolation,
        RefuseReason::NotAvailable => anml::types::enums::RefuseReason::PolicyViolation,
    }
}

// ---------------------------------------------------------------------------
// AnswerOutcome
// ---------------------------------------------------------------------------

/// The outcome of a `build_answer` call.
///
/// Either the disclosure was allowed and an `<answer>` was produced, or it
/// was denied and a `<refuse>` was produced instead.
#[derive(Clone, Debug)]
pub enum AnswerOutcome {
    /// Disclosure was allowed; contains the constructed `<answer>`.
    Answer(AnmlAnswer),
    /// Disclosure was denied; contains the constructed `<refuse>`.
    Refuse(AnmlRefuse),
}

// ---------------------------------------------------------------------------
// build_answer
// ---------------------------------------------------------------------------

/// Build an `<answer>` element with automatic disclosure evaluation.
///
/// Runs the full 7-step RFC disclosure algorithm. If disclosure is allowed,
/// returns `AnswerOutcome::Answer` with the consent basis set. If denied,
/// returns `AnswerOutcome::Refuse` with the appropriate reason.
///
/// This function works for both action-bound asks and deferred asks (asks
/// without an `action` attribute). For deferred asks, no HTTP submission
/// occurs, but disclosure is still fully evaluated.
///
/// # Arguments
///
/// * `doc` — The ANML document containing the `<constraints>` and `<knowledge>`.
/// * `field` — The field name being answered (must match an `<ask>` field).
/// * `value` — The value to disclose.
/// * `ctx` — The disclosure context with all required dependencies.
///
/// # Returns
///
/// An [`AnswerOutcome`] — either an `<answer>` or a `<refuse>`.
pub fn build_answer(
    doc: &AnmlDocument,
    field: &str,
    value: &str,
    ctx: &DisclosureContext<'_>,
) -> AnswerOutcome {
    build_answer_with_rules(doc, &extract_rules(doc), field, value, ctx)
}

/// Build an `<answer>` with pre-extracted disclosure rules.
///
/// This is useful when building multiple answers against the same document
/// to avoid re-extracting rules each time.
pub fn build_answer_with_rules(
    doc: &AnmlDocument,
    rules: &[DisclosureRule],
    field: &str,
    value: &str,
    ctx: &DisclosureContext<'_>,
) -> AnswerOutcome {
    let decision = evaluate(doc, rules, field, value, ctx);

    match decision {
        DisclosureDecision::Allow {
            value: disclosed_value,
            consent_basis,
            tokenized: _,
        } => AnswerOutcome::Answer(AnmlAnswer {
            field: field.to_string(),
            value: disclosed_value,
            consent: Some(consent_basis_to_type(consent_basis)),
        }),
        DisclosureDecision::Deny {
            field: denied_field,
            reason,
            refuse_reason,
        } => AnswerOutcome::Refuse(AnmlRefuse {
            field: denied_field,
            reason: refuse_reason_to_anml(&refuse_reason),
            constraint: None,
            message: Some(reason),
        }),
    }
}

// ---------------------------------------------------------------------------
// build_refuse
// ---------------------------------------------------------------------------

/// Build a `<refuse>` element.
///
/// # Arguments
///
/// * `field` — The field name being refused.
/// * `reason` — The `anml` crate's `RefuseReason` enum value.
/// * `constraint` — Optional reference to the `<disclosure>` field that
///   caused the refusal.
/// * `message` — Optional human-readable explanation.
pub fn build_refuse(
    field: impl Into<String>,
    reason: anml::types::enums::RefuseReason,
    constraint: Option<String>,
    message: Option<String>,
) -> AnmlRefuse {
    AnmlRefuse {
        field: field.into(),
        reason,
        constraint,
        message,
    }
}

// ---------------------------------------------------------------------------
// build_ask
// ---------------------------------------------------------------------------

/// Build an `<ask>` element for symmetric knowledge exchange.
///
/// Agents can include `<ask>` elements in responses to request information
/// from the service.
///
/// # Arguments
///
/// * `field` — The field name being requested.
/// * `action_id` — The action ID this ask is bound to.
/// * `purpose` — Optional human-readable purpose for the request.
pub fn build_ask(
    field: impl Into<String>,
    action_id: impl Into<String>,
    purpose: Option<String>,
) -> AnmlAsk {
    AnmlAsk {
        field: field.into(),
        action: action_id.into(),
        required: None,
        purpose,
        ask_type: None,
    }
}

// ---------------------------------------------------------------------------
// build_inform
// ---------------------------------------------------------------------------

/// Build an `<inform>` element for symmetric knowledge exchange.
///
/// Agents can include `<inform>` elements in responses to communicate
/// information to the service.
///
/// # Arguments
///
/// * `text` — The information text content.
pub fn build_inform(text: impl Into<String>) -> AnmlInform {
    AnmlInform {
        text: Some(text.into()),
        ..AnmlInform::default()
    }
}

// ---------------------------------------------------------------------------
// ResponseBuilder (composite)
// ---------------------------------------------------------------------------

/// Composite builder for constructing agent response documents.
///
/// Wraps the `anml` crate's `ResponseBuilder` and provides convenience
/// methods that integrate with the disclosure engine. Answers added via
/// [`answer_checked`](ResponseBuilder::answer_checked) automatically run
/// the 7-step disclosure algorithm.
///
/// # Example
///
/// ```rust,no_run
/// use anml_client::knowledge::response::ResponseBuilder;
/// # use anml::types::document::AnmlDocument;
/// # use anml_client::disclosure::DisclosureContext;
///
/// # fn example(doc: &AnmlDocument, ctx: &DisclosureContext<'_>) {
/// let response_doc = ResponseBuilder::new()
///     .answer_checked(doc, "email", "user@example.com", ctx)
///     .inform("User prefers morning departures.")
///     .build();
/// # }
/// ```
pub struct ResponseBuilder {
    inner: AnmlResponseBuilder,
    /// Tracks refused fields for inspection.
    refused: Vec<AnmlRefuse>,
}

impl ResponseBuilder {
    /// Create a new `ResponseBuilder`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: AnmlResponseBuilder::new(),
            refused: Vec::new(),
        }
    }

    /// Add an `<answer>` with automatic disclosure evaluation.
    ///
    /// Runs the full 7-step algorithm. If disclosure is allowed, adds an
    /// `<answer>`. If denied, adds a `<refuse>` instead.
    #[must_use]
    pub fn answer_checked(
        mut self,
        doc: &AnmlDocument,
        field: &str,
        value: &str,
        ctx: &DisclosureContext<'_>,
    ) -> Self {
        let outcome = build_answer(doc, field, value, ctx);
        match outcome {
            AnswerOutcome::Answer(answer) => {
                self.inner = self.inner.answer(answer);
            }
            AnswerOutcome::Refuse(refuse) => {
                self.refused.push(refuse.clone());
                self.inner = self.inner.refuse(refuse);
            }
        }
        self
    }

    /// Add a pre-built `<answer>` element directly (no disclosure check).
    ///
    /// Use this only when you have already run disclosure evaluation
    /// externally. Prefer [`answer_checked`](Self::answer_checked).
    #[must_use]
    pub fn answer(mut self, answer: AnmlAnswer) -> Self {
        self.inner = self.inner.answer(answer);
        self
    }

    /// Add a `<refuse>` element.
    #[must_use]
    pub fn refuse(mut self, refuse: AnmlRefuse) -> Self {
        self.refused.push(refuse.clone());
        self.inner = self.inner.refuse(refuse);
        self
    }

    /// Add an `<ask>` element.
    #[must_use]
    pub fn ask(mut self, ask: AnmlAsk) -> Self {
        self.inner = self.inner.ask(ask);
        self
    }

    /// Add an `<inform>` element with text content.
    #[must_use]
    pub fn inform(mut self, text: impl Into<String>) -> Self {
        self.inner = self.inner.inform(build_inform(text));
        self
    }

    /// Add a pre-built `<inform>` element.
    #[must_use]
    pub fn inform_element(mut self, inform: AnmlInform) -> Self {
        self.inner = self.inner.inform(inform);
        self
    }

    /// Returns the fields that were refused during building.
    pub fn refused_fields(&self) -> &[AnmlRefuse] {
        &self.refused
    }

    /// Consume the builder and produce the final `AnmlDocument`.
    #[must_use]
    pub fn build(self) -> AnmlDocument {
        self.inner.build()
    }
}

impl Default for ResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to build a complete response document.
///
/// Takes a list of (field, value) pairs, runs disclosure on each, and
/// returns the composed `AnmlDocument` with answers and/or refuses.
///
/// # Arguments
///
/// * `doc` — The source ANML document.
/// * `answers` — Slice of `(field, value)` pairs to answer.
/// * `ctx` — The disclosure context.
///
/// # Returns
///
/// The composed response `AnmlDocument`.
pub fn build_response(
    doc: &AnmlDocument,
    answers: &[(&str, &str)],
    ctx: &DisclosureContext<'_>,
) -> AnmlDocument {
    let rules = extract_rules(doc);
    let mut builder = AnmlResponseBuilder::new();

    for &(field, value) in answers {
        let outcome = build_answer_with_rules(doc, &rules, field, value, ctx);
        match outcome {
            AnswerOutcome::Answer(answer) => {
                builder = builder.answer(answer);
            }
            AnswerOutcome::Refuse(refuse) => {
                builder = builder.refuse(refuse);
            }
        }
    }

    builder.build()
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

    use crate::config::{ConsentDecision, ConsentHandler, Origin, TrustDecision, TrustPolicy};
    use crate::disclosure::{ConsentStore, RateLimitTracker};

    fn test_origin() -> Origin {
        Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        }
    }

    struct AllowAllPolicy;
    impl TrustPolicy for AllowAllPolicy {
        fn evaluate(&self, _origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
            TrustDecision::Allow
        }
    }

    struct DenyAllPolicy;
    impl TrustPolicy for DenyAllPolicy {
        fn evaluate(&self, _origin: &Origin, _doc: &AnmlDocument) -> TrustDecision {
            TrustDecision::Deny {
                reason: "denied".into(),
            }
        }
    }

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

    fn make_ctx<'a>(
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

    // -- build_answer --

    #[test]
    fn build_answer_allowed_produces_answer() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let outcome = build_answer(&doc, "email", "test@example.com", &ctx);

        match outcome {
            AnswerOutcome::Answer(answer) => {
                assert_eq!(answer.field, "email");
                assert_eq!(answer.value, "test@example.com");
                assert_eq!(answer.consent, Some(ConsentType::Explicit));
            }
            AnswerOutcome::Refuse(_) => panic!("expected Answer, got Refuse"),
        }
    }

    #[test]
    fn build_answer_denied_produces_refuse() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = DenyAllPolicy;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::None);
        let outcome = build_answer(&doc, "email", "test@example.com", &ctx);

        match outcome {
            AnswerOutcome::Answer(_) => panic!("expected Refuse, got Answer"),
            AnswerOutcome::Refuse(refuse) => {
                assert_eq!(refuse.field, "email");
                assert_eq!(
                    refuse.reason,
                    anml::types::enums::RefuseReason::TrustInsufficient
                );
                assert!(refuse.message.is_some());
            }
        }
    }

    #[test]
    fn build_answer_consent_denied_produces_refuse() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysDenyHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);
        let outcome = build_answer(&doc, "email", "test@example.com", &ctx);

        match outcome {
            AnswerOutcome::Answer(_) => panic!("expected Refuse"),
            AnswerOutcome::Refuse(refuse) => {
                assert_eq!(refuse.reason, anml::types::enums::RefuseReason::UserDenied);
            }
        }
    }

    #[test]
    fn build_answer_works_for_deferred_ask() {
        // Deferred ask: no action attribute, but disclosure still evaluated
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        // Create a doc with an ask that has no action (deferred)
        let doc = AnmlDocument {
            constraints: Some(AnmlConstraints {
                disclosures: Some(vec![AnmlDisclosure {
                    field: "preference".to_string(),
                    requires: DisclosureRequires::ExplicitConsent,
                }]),
            }),
            knowledge: Some(AnmlKnowledge {
                asks: Some(vec![AnmlAsk {
                    field: "preference".to_string(),
                    action: String::new(), // deferred — no action
                    required: None,
                    purpose: Some("personalization".to_string()),
                    ask_type: None,
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let outcome = build_answer(&doc, "preference", "morning", &ctx);
        assert!(matches!(outcome, AnswerOutcome::Answer(_)));
    }

    #[test]
    fn build_answer_implicit_consent_allowed() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::None);
        let outcome = build_answer(&doc, "email", "test@example.com", &ctx);

        match outcome {
            AnswerOutcome::Answer(answer) => {
                assert_eq!(answer.consent, Some(ConsentType::Implicit));
            }
            AnswerOutcome::Refuse(_) => panic!("expected Answer"),
        }
    }

    // -- build_refuse --

    #[test]
    fn build_refuse_basic() {
        let refuse = build_refuse(
            "ssn",
            anml::types::enums::RefuseReason::UserDenied,
            None,
            Some("I decline to share this".to_string()),
        );
        assert_eq!(refuse.field, "ssn");
        assert_eq!(refuse.reason, anml::types::enums::RefuseReason::UserDenied);
        assert!(refuse.constraint.is_none());
        assert_eq!(refuse.message.as_deref(), Some("I decline to share this"));
    }

    #[test]
    fn build_refuse_with_constraint() {
        let refuse = build_refuse(
            "phone",
            anml::types::enums::RefuseReason::ConstraintViolation,
            Some("phone".to_string()),
            None,
        );
        assert_eq!(refuse.constraint.as_deref(), Some("phone"));
    }

    // -- build_ask --

    #[test]
    fn build_ask_basic() {
        let ask = build_ask("available-dates", "submit-booking", None);
        assert_eq!(ask.field, "available-dates");
        assert_eq!(ask.action, "submit-booking");
        assert!(ask.purpose.is_none());
        assert!(ask.required.is_none());
        assert!(ask.ask_type.is_none());
    }

    #[test]
    fn build_ask_with_purpose() {
        let ask = build_ask(
            "location",
            "search",
            Some("To find nearby results".to_string()),
        );
        assert_eq!(ask.purpose.as_deref(), Some("To find nearby results"));
    }

    // -- build_inform --

    #[test]
    fn build_inform_basic() {
        let inform = build_inform("User prefers morning departures.");
        assert_eq!(
            inform.text.as_deref(),
            Some("User prefers morning departures.")
        );
        assert!(inform.ttl.is_none());
        assert!(inform.confidentiality.is_none());
    }

    // -- build_response --

    #[test]
    fn build_response_multiple_answers() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        let doc = AnmlDocument {
            constraints: Some(AnmlConstraints {
                disclosures: Some(vec![
                    AnmlDisclosure {
                        field: "email".to_string(),
                        requires: DisclosureRequires::ExplicitConsent,
                    },
                    AnmlDisclosure {
                        field: "name".to_string(),
                        requires: DisclosureRequires::None,
                    },
                ]),
            }),
            knowledge: Some(AnmlKnowledge {
                asks: Some(vec![
                    AnmlAsk {
                        field: "email".to_string(),
                        action: "submit".to_string(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                    AnmlAsk {
                        field: "name".to_string(),
                        action: "submit".to_string(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let response = build_response(
            &doc,
            &[("email", "test@example.com"), ("name", "Alice")],
            &ctx,
        );

        let answers = response.answers.expect("should have answers");
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].field, "email");
        assert_eq!(answers[1].field, "name");
    }

    #[test]
    fn build_response_mixed_allow_deny() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysDenyHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        let doc = AnmlDocument {
            constraints: Some(AnmlConstraints {
                disclosures: Some(vec![
                    AnmlDisclosure {
                        field: "email".to_string(),
                        requires: DisclosureRequires::ExplicitConsent,
                    },
                    AnmlDisclosure {
                        field: "name".to_string(),
                        requires: DisclosureRequires::None,
                    },
                ]),
            }),
            knowledge: Some(AnmlKnowledge {
                asks: Some(vec![
                    AnmlAsk {
                        field: "email".to_string(),
                        action: "submit".to_string(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                    AnmlAsk {
                        field: "name".to_string(),
                        action: "submit".to_string(),
                        required: None,
                        purpose: None,
                        ask_type: None,
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let response = build_response(
            &doc,
            &[("email", "test@example.com"), ("name", "Alice")],
            &ctx,
        );

        // email requires explicit consent, handler denies → refuse
        let refuses = response.refuses.expect("should have refuses");
        assert_eq!(refuses.len(), 1);
        assert_eq!(refuses[0].field, "email");

        // name requires none → answer
        let answers = response.answers.expect("should have answers");
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].field, "name");
    }

    // -- ResponseBuilder (composite) --

    #[test]
    fn response_builder_answer_checked() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = AllowAllPolicy;
        let handler = AlwaysGrantHandler;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, Some(&handler));

        let doc = make_doc_with_disclosure("email", DisclosureRequires::ExplicitConsent);

        let response = ResponseBuilder::new()
            .answer_checked(&doc, "email", "test@example.com", &ctx)
            .inform("Additional info")
            .build();

        assert_eq!(response.answers.as_ref().map(|a| a.len()), Some(1));
        assert_eq!(response.informs.as_ref().map(|i| i.len()), Some(1));
    }

    #[test]
    fn response_builder_tracks_refused() {
        let origin = test_origin();
        let store = ConsentStore::new();
        let limiter = RateLimitTracker::new();
        let policy = DenyAllPolicy;
        let ctx = make_ctx(&origin, &store, &limiter, &policy, None);

        let doc = make_doc_with_disclosure("email", DisclosureRequires::None);

        let builder = ResponseBuilder::new()
            .answer_checked(&doc, "email", "test@example.com", &ctx);

        assert_eq!(builder.refused_fields().len(), 1);
        assert_eq!(builder.refused_fields()[0].field, "email");

        let response = builder.build();
        assert!(response.answers.is_none());
        assert_eq!(response.refuses.as_ref().map(|r| r.len()), Some(1));
    }

    #[test]
    fn response_builder_empty() {
        let response = ResponseBuilder::new().build();
        assert!(response.answers.is_none());
        assert!(response.refuses.is_none());
        assert!(response.asks.is_none());
        assert!(response.informs.is_none());
    }

    #[test]
    fn response_builder_with_ask_and_inform() {
        let response = ResponseBuilder::new()
            .ask(build_ask("dates", "search", None))
            .inform("User prefers mornings")
            .build();

        assert_eq!(response.asks.as_ref().map(|a| a.len()), Some(1));
        assert_eq!(response.informs.as_ref().map(|i| i.len()), Some(1));
    }

    #[test]
    fn response_builder_default() {
        let builder = ResponseBuilder::default();
        let response = builder.build();
        assert!(response.answers.is_none());
    }
}
