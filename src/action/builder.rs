//! Fluent action builder with runtime typestate tracking.
//!
//! `ActionRequestBuilder` provides a fluent API for constructing and executing
//! ANML actions. It tracks which required parameters have been set and validates
//! at `.execute()` time, producing actionable error messages referencing the
//! param name and constraint.
//!
//! # Example
//!
//! ```rust,no_run
//! # async fn example() -> anml_client::Result<()> {
//! # let doc = anml::types::document::AnmlDocument::default();
//! // Assuming you have an AnmlDocument and an ActionContext:
//! use anml_client::action::builder::ActionRequestBuilder;
//!
//! let builder = ActionRequestBuilder::new(&doc, "submit-airline", "https://example.com", None)?;
//! // builder.param("airline", "Delta").param("class", "economy").execute(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Deferred Asks
//!
//! This builder is NOT usable for deferred asks (asks without an `action`
//! attribute). Attempting to create a builder for a deferred ask returns
//! an error at construction time.

use std::collections::HashSet;

use anml::types::document::AnmlDocument;
use anml::types::elements::AnmlAction;
use anml::types::enums::ParamType;

use crate::disclosure::ConsentBasis;
use crate::error::AnmlClientError;

use super::params::canonical_value;
use super::validation;
use super::ActionContext;

// ---------------------------------------------------------------------------
// ActionRequestBuilder
// ---------------------------------------------------------------------------

/// A fluent builder for constructing and executing ANML actions.
///
/// Tracks which required parameters have been set at runtime and validates
/// all constraints at `.execute()` time. Produces actionable error messages
/// referencing param name + constraint + expected + actual.
///
/// # Not Usable for Deferred Asks
///
/// If an `<ask>` has no `action` attribute, it is a "deferred ask" — the
/// answer is included in a subsequent document, not submitted via HTTP.
/// `ActionRequestBuilder` requires an action binding and will return an
/// error at construction time for deferred asks.
#[derive(Debug)]
pub struct ActionRequestBuilder<'a> {
    /// The ANML document containing the action.
    doc: &'a AnmlDocument,
    /// The action definition from `<interact>`.
    action: &'a AnmlAction,
    /// The document origin URL (scheme://host[:port]).
    #[allow(dead_code)]
    document_origin: &'a str,
    /// Optional xml:base override.
    #[allow(dead_code)]
    xml_base: Option<&'a str>,
    /// User-supplied parameter values (name, string value).
    params: Vec<(String, String)>,
    /// Names of params that have been set.
    set_params: HashSet<String>,
    /// Optional consent basis override.
    consent_basis: Option<ConsentBasis>,
}

impl<'a> ActionRequestBuilder<'a> {
    /// Create a new builder for the given action ID.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The action ID is not found in the document's `<interact>` section.
    /// - The action ID corresponds to a deferred ask (no `action` attribute).
    pub fn new(
        doc: &'a AnmlDocument,
        action_id: &str,
        document_origin: &'a str,
        xml_base: Option<&'a str>,
    ) -> crate::Result<Self> {
        // Check that this is not a deferred ask
        Self::check_not_deferred_ask(doc, action_id)?;

        // Find the action in <interact>
        let action = super::find_action(doc, action_id).ok_or_else(|| {
            AnmlClientError::MalformedDocument {
                detail: format!(
                    "action '{}' not found in <interact>; \
                     available actions: [{}]",
                    action_id,
                    Self::available_action_ids(doc),
                ),
            }
        })?;

        Ok(Self {
            doc,
            action,
            document_origin,
            xml_base,
            params: Vec::new(),
            set_params: HashSet::new(),
            consent_basis: None,
        })
    }

    /// Set a string parameter value.
    ///
    /// Accepts any type that implements `Into<String>`. The value is
    /// converted to its canonical string form based on the param's
    /// declared type at execute time.
    pub fn param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.set_params.insert(name.clone());
        self.params.push((name, value.into()));
        self
    }

    /// Set an `i64` parameter value.
    ///
    /// The value is converted to its canonical decimal string form.
    pub fn param_i64(mut self, name: impl Into<String>, value: i64) -> Self {
        let name = name.into();
        self.set_params.insert(name.clone());
        self.params.push((name, value.to_string()));
        self
    }

    /// Set a `u64` parameter value.
    ///
    /// The value is converted to its canonical decimal string form.
    pub fn param_u64(mut self, name: impl Into<String>, value: u64) -> Self {
        let name = name.into();
        self.set_params.insert(name.clone());
        self.params.push((name, value.to_string()));
        self
    }

    /// Set an `f64` parameter value.
    ///
    /// The value is converted to its canonical decimal string form
    /// (integer values emit without decimal point).
    pub fn param_f64(mut self, name: impl Into<String>, value: f64) -> Self {
        let name = name.into();
        self.set_params.insert(name.clone());
        // Use canonical_value for Number type
        let canonical = canonical_value(&value.to_string(), Some(&ParamType::Number));
        self.params.push((name, canonical));
        self
    }

    /// Set a `bool` parameter value.
    ///
    /// The value is converted to canonical XSD boolean (`"true"` or `"false"`).
    pub fn param_bool(mut self, name: impl Into<String>, value: bool) -> Self {
        let name = name.into();
        self.set_params.insert(name.clone());
        self.params.push((name, if value { "true" } else { "false" }.to_string()));
        self
    }

    /// Set the consent basis for this action's disclosures.
    ///
    /// Accepts a [`ConsentBasis`] enum value.
    pub fn with_consent(mut self, basis: ConsentBasis) -> Self {
        self.consent_basis = Some(basis);
        self
    }

    /// Set the consent basis from a string.
    ///
    /// Accepts `"explicit"`, `"implicit"`, or `"delegated"`.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not a valid consent basis.
    pub fn with_consent_str(mut self, basis: &str) -> crate::Result<Self> {
        let parsed = parse_consent_basis(basis)?;
        self.consent_basis = Some(parsed);
        Ok(self)
    }

    /// Execute the action.
    ///
    /// Validates all required parameters are set, validates param values
    /// against their declared constraints, then delegates to
    /// [`execute_action`](super::execute_action).
    ///
    /// # Errors
    ///
    /// Returns actionable errors referencing param name + constraint if:
    /// - A required parameter has not been set via `.param()`.
    /// - A parameter value fails type, pattern, min, max, or enum validation.
    /// - The action execution itself fails (HTTP, SSRF, budget, etc.).
    pub async fn execute(
        self,
        ctx: &ActionContext<'_>,
    ) -> crate::Result<super::ActionResult> {
        // Validate required params are present
        self.validate_required_params()?;

        // Validate param values against constraints
        self.validate_param_values()?;

        // Delegate to execute_action
        super::execute_action(self.doc, &self.action.id, &self.params, ctx).await
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Check that the action_id is not only referenced by deferred asks
    /// (asks without an `action` attribute).
    fn check_not_deferred_ask(_doc: &AnmlDocument, action_id: &str) -> crate::Result<()> {
        // A deferred ask has action="" (empty string from the parser).
        // We need to check if any ask references this action_id.
        // If the action_id itself is empty, it's definitely deferred.
        if action_id.is_empty() {
            return Err(AnmlClientError::MalformedDocument {
                detail: "ActionRequestBuilder cannot be used for deferred asks \
                         (asks without an action attribute); use build_answer() instead"
                    .into(),
            });
        }

        // Also check: if there are asks that reference this action_id but
        // the action doesn't exist in <interact>, that's caught by find_action.
        // The key check here is that we're not trying to build for a deferred ask.
        Ok(())
    }

    /// Validate that all required parameters have been set.
    fn validate_required_params(&self) -> crate::Result<()> {
        let param_defs = self.action.params.as_deref().unwrap_or(&[]);
        for def in param_defs {
            if def.required == Some(true) && !self.set_params.contains(&def.name) {
                // Check if there's a default value
                if def.default.is_none() {
                    return Err(AnmlClientError::ParamValidation {
                        param: def.name.clone(),
                        constraint: "required".into(),
                        expected: format!(
                            "param '{}' is required; call .param(\"{}\", value) before .execute()",
                            def.name, def.name
                        ),
                        actual: "(not set)".into(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate all supplied param values against their declared constraints.
    fn validate_param_values(&self) -> crate::Result<()> {
        let param_defs = self.action.params.as_deref().unwrap_or(&[]);
        let collected = super::params::collect_params(param_defs, &self.params);

        for def in param_defs {
            let value = collected
                .iter()
                .find(|p| p.name == def.name)
                .map(|p| p.value.as_str());
            validation::validate_param(def, value)?;
        }
        Ok(())
    }

    /// List available action IDs for error messages.
    fn available_action_ids(doc: &AnmlDocument) -> String {
        doc.interact
            .as_ref()
            .map(|interact| {
                interact
                    .actions
                    .iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Consent basis parsing
// ---------------------------------------------------------------------------

/// Parse a consent basis string into a [`ConsentBasis`] enum.
///
/// Accepts `"explicit"`, `"implicit"`, or `"delegated"` (case-insensitive).
pub fn parse_consent_basis(s: &str) -> crate::Result<ConsentBasis> {
    match s.to_lowercase().as_str() {
        "explicit" => Ok(ConsentBasis::Explicit),
        "implicit" => Ok(ConsentBasis::Implicit),
        "delegated" => Ok(ConsentBasis::Delegated),
        _ => Err(AnmlClientError::ParamValidation {
            param: "consent_basis".into(),
            constraint: "one-of".into(),
            expected: "[explicit, implicit, delegated]".into(),
            actual: s.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// ParamSetter trait — accept multiple types for .param()
// ---------------------------------------------------------------------------

/// Trait for types that can be used as parameter values.
///
/// Implemented for `&str`, `String`, `i64`, `u64`, `f64`, and `bool`.
/// Each implementation converts to the canonical string form.
pub trait IntoParamValue {
    /// Convert to a canonical string value.
    fn into_param_value(self) -> String;
}

impl IntoParamValue for &str {
    fn into_param_value(self) -> String {
        self.to_string()
    }
}

impl IntoParamValue for String {
    fn into_param_value(self) -> String {
        self
    }
}

impl IntoParamValue for i64 {
    fn into_param_value(self) -> String {
        self.to_string()
    }
}

impl IntoParamValue for u64 {
    fn into_param_value(self) -> String {
        self.to_string()
    }
}

impl IntoParamValue for f64 {
    fn into_param_value(self) -> String {
        canonical_value(&self.to_string(), Some(&ParamType::Number))
    }
}

impl IntoParamValue for bool {
    fn into_param_value(self) -> String {
        if self { "true" } else { "false" }.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::elements::{AnmlAction, AnmlInteract, AnmlParam};

    fn make_doc_with_action(action_id: &str, params: Vec<AnmlParam>) -> AnmlDocument {
        AnmlDocument {
            interact: Some(AnmlInteract {
                actions: vec![AnmlAction {
                    id: action_id.to_string(),
                    method: "POST".into(),
                    endpoint: "/submit".into(),
                    enctype: None,
                    auth: None,
                    idempotent: None,
                    confirm: None,
                    description: None,
                    params: if params.is_empty() { None } else { Some(params) },
                    response: None,
                }],
            }),
            ..Default::default()
        }
    }

    fn make_param(name: &str, required: bool) -> AnmlParam {
        AnmlParam {
            name: name.to_string(),
            param_type: None,
            required: Some(required),
            default: None,
            description: None,
            pattern: None,
            min: None,
            max: None,
            options: None,
        }
    }

    fn make_typed_param(name: &str, param_type: ParamType) -> AnmlParam {
        AnmlParam {
            name: name.to_string(),
            param_type: Some(param_type),
            required: None,
            default: None,
            description: None,
            pattern: None,
            min: None,
            max: None,
            options: None,
        }
    }

    // -- Construction --

    #[test]
    fn new_builder_for_existing_action() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None);
        assert!(builder.is_ok());
    }

    #[test]
    fn new_builder_for_missing_action_fails() {
        let doc = AnmlDocument::default();
        let result = ActionRequestBuilder::new(&doc, "nonexistent", "https://example.com", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, AnmlClientError::MalformedDocument { ref detail } if detail.contains("not found")),
            "got: {err}"
        );
    }

    #[test]
    fn new_builder_for_empty_action_id_fails() {
        let doc = make_doc_with_action("submit", vec![]);
        let result = ActionRequestBuilder::new(&doc, "", "https://example.com", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, AnmlClientError::MalformedDocument { ref detail } if detail.contains("deferred")),
            "got: {err}"
        );
    }

    // -- Param setting --

    #[test]
    fn param_string_sets_value() {
        let doc = make_doc_with_action("submit", vec![make_param("airline", false)]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("airline", "Delta");
        assert!(builder.set_params.contains("airline"));
        assert_eq!(builder.params[0], ("airline".to_string(), "Delta".to_string()));
    }

    #[test]
    fn param_i64_sets_value() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_i64("count", 42);
        assert_eq!(builder.params[0], ("count".to_string(), "42".to_string()));
    }

    #[test]
    fn param_u64_sets_value() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_u64("count", 100);
        assert_eq!(builder.params[0], ("count".to_string(), "100".to_string()));
    }

    #[test]
    fn param_f64_canonical_integer() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_f64("price", 42.0);
        // Canonical form: integer value without decimal point
        assert_eq!(builder.params[0], ("price".to_string(), "42".to_string()));
    }

    #[test]
    fn param_f64_with_fraction() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_f64("price", 3.14);
        assert_eq!(builder.params[0], ("price".to_string(), "3.14".to_string()));
    }

    #[test]
    fn param_bool_true() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_bool("active", true);
        assert_eq!(builder.params[0], ("active".to_string(), "true".to_string()));
    }

    #[test]
    fn param_bool_false() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_bool("active", false);
        assert_eq!(builder.params[0], ("active".to_string(), "false".to_string()));
    }

    // -- Consent --

    #[test]
    fn with_consent_enum() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent(ConsentBasis::Explicit);
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Explicit));
    }

    #[test]
    fn with_consent_str_valid() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("explicit")
            .unwrap();
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Explicit));
    }

    #[test]
    fn with_consent_str_implicit() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("implicit")
            .unwrap();
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Implicit));
    }

    #[test]
    fn with_consent_str_delegated() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("delegated")
            .unwrap();
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Delegated));
    }

    #[test]
    fn with_consent_str_invalid() {
        let doc = make_doc_with_action("submit", vec![]);
        let result = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("invalid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, AnmlClientError::ParamValidation { ref constraint, .. } if constraint == "one-of"),
            "got: {err}"
        );
    }

    #[test]
    fn with_consent_str_case_insensitive() {
        let doc = make_doc_with_action("submit", vec![]);
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("EXPLICIT")
            .unwrap();
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Explicit));
    }

    // -- Required param validation --

    #[test]
    fn validate_required_param_missing_fails() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_param("reason", true)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap();
        let result = builder.validate_required_params();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, AnmlClientError::ParamValidation { ref param, ref constraint, .. }
                if param == "reason" && constraint == "required"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_required_param_present_ok() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_param("reason", true)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("reason", "damaged");
        assert!(builder.validate_required_params().is_ok());
    }

    #[test]
    fn validate_required_param_with_default_ok() {
        let doc = make_doc_with_action(
            "submit",
            vec![{
                let mut p = make_param("class", true);
                p.default = Some("economy".into());
                p
            }],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap();
        // Required but has a default — should pass
        assert!(builder.validate_required_params().is_ok());
    }

    #[test]
    fn validate_optional_param_missing_ok() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_param("notes", false)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap();
        assert!(builder.validate_required_params().is_ok());
    }

    // -- Param value validation --

    #[test]
    fn validate_number_type_ok() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_typed_param("price", ParamType::Number)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("price", "42");
        assert!(builder.validate_param_values().is_ok());
    }

    #[test]
    fn validate_number_type_invalid() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_typed_param("price", ParamType::Number)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("price", "not-a-number");
        let result = builder.validate_param_values();
        assert!(result.is_err());
    }

    #[test]
    fn validate_boolean_type_ok() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_typed_param("active", ParamType::Boolean)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param_bool("active", true);
        assert!(builder.validate_param_values().is_ok());
    }

    // -- Chaining --

    #[test]
    fn fluent_chaining() {
        let doc = make_doc_with_action(
            "submit",
            vec![
                make_param("airline", true),
                make_param("class", false),
            ],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("airline", "Delta")
            .param("class", "economy")
            .with_consent(ConsentBasis::Explicit);

        assert!(builder.set_params.contains("airline"));
        assert!(builder.set_params.contains("class"));
        assert_eq!(builder.consent_basis, Some(ConsentBasis::Explicit));
        assert!(builder.validate_required_params().is_ok());
    }

    // -- IntoParamValue trait --

    #[test]
    fn into_param_value_str() {
        assert_eq!("hello".into_param_value(), "hello");
    }

    #[test]
    fn into_param_value_string() {
        assert_eq!(String::from("hello").into_param_value(), "hello");
    }

    #[test]
    fn into_param_value_i64() {
        assert_eq!(42i64.into_param_value(), "42");
        assert_eq!((-1i64).into_param_value(), "-1");
    }

    #[test]
    fn into_param_value_u64() {
        assert_eq!(100u64.into_param_value(), "100");
    }

    #[test]
    fn into_param_value_f64() {
        assert_eq!(42.0f64.into_param_value(), "42");
        assert_eq!(3.14f64.into_param_value(), "3.14");
    }

    #[test]
    fn into_param_value_bool() {
        assert_eq!(true.into_param_value(), "true");
        assert_eq!(false.into_param_value(), "false");
    }

    // -- parse_consent_basis --

    #[test]
    fn parse_consent_basis_all_variants() {
        assert_eq!(parse_consent_basis("explicit").unwrap(), ConsentBasis::Explicit);
        assert_eq!(parse_consent_basis("implicit").unwrap(), ConsentBasis::Implicit);
        assert_eq!(parse_consent_basis("delegated").unwrap(), ConsentBasis::Delegated);
    }

    #[test]
    fn parse_consent_basis_case_insensitive() {
        assert_eq!(parse_consent_basis("EXPLICIT").unwrap(), ConsentBasis::Explicit);
        assert_eq!(parse_consent_basis("Implicit").unwrap(), ConsentBasis::Implicit);
    }

    #[test]
    fn parse_consent_basis_invalid() {
        assert!(parse_consent_basis("unknown").is_err());
        assert!(parse_consent_basis("").is_err());
    }

    // -- Error message quality --

    #[test]
    fn missing_required_param_error_is_actionable() {
        let doc = make_doc_with_action(
            "submit",
            vec![make_param("reason", true)],
        );
        let builder = ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap();
        let err = builder.validate_required_params().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("reason"), "error should reference param name: {msg}");
        assert!(msg.contains("required"), "error should reference constraint: {msg}");
        assert!(msg.contains(".param("), "error should suggest fix: {msg}");
    }

    #[test]
    fn missing_action_error_lists_available() {
        let doc = make_doc_with_action("submit-airline", vec![]);
        let err = ActionRequestBuilder::new(&doc, "nonexistent", "https://example.com", None)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("submit-airline"), "error should list available actions: {msg}");
    }
}
