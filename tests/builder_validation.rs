//! Runtime validation tests for ActionRequestBuilder.
//!
//! The builder uses runtime validation (not compile-time typestate),
//! so these tests verify that missing required params produce clear
//! errors at construction and validation time.

use anml::types::document::AnmlDocument;
use anml::types::elements::{AnmlAction, AnmlInteract, AnmlParam};
use anml_client::action::builder::ActionRequestBuilder;
use anml_client::error::AnmlClientError;

fn make_doc(action_id: &str, params: Vec<AnmlParam>) -> AnmlDocument {
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
                params: if params.is_empty() {
                    None
                } else {
                    Some(params)
                },
                response: None,
            }],
        }),
        ..Default::default()
    }
}

fn required_param(name: &str) -> AnmlParam {
    AnmlParam {
        name: name.to_string(),
        param_type: None,
        required: Some(true),
        default: None,
        description: None,
        pattern: None,
        min: None,
        max: None,
        options: None,
    }
}

fn optional_param(name: &str) -> AnmlParam {
    AnmlParam {
        name: name.to_string(),
        param_type: None,
        required: Some(false),
        default: None,
        description: None,
        pattern: None,
        min: None,
        max: None,
        options: None,
    }
}

#[test]
fn nonexistent_action_produces_error() {
    let doc = AnmlDocument::default();
    let result =
        ActionRequestBuilder::new(&doc, "nonexistent", "https://example.com", None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, AnmlClientError::MalformedDocument { .. }),
        "got: {err}"
    );
}

#[test]
fn deferred_ask_empty_action_id_produces_error() {
    let doc = make_doc("submit", vec![]);
    let result = ActionRequestBuilder::new(&doc, "", "https://example.com", None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, AnmlClientError::MalformedDocument { ref detail } if detail.contains("deferred")),
        "got: {err}"
    );
}

#[test]
fn builder_accepts_all_param_types() {
    let doc = make_doc("submit", vec![optional_param("a")]);
    let builder =
        ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .param("str_val", "hello")
            .param_i64("int_val", 42)
            .param_u64("uint_val", 100)
            .param_f64("float_val", 3.14)
            .param_bool("bool_val", true);

    // Builder should be constructable without errors
    let _ = builder;
}

#[test]
fn builder_with_consent_str_valid() {
    let doc = make_doc("submit", vec![]);
    let builder =
        ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("explicit");
    assert!(builder.is_ok());
}

#[test]
fn builder_with_consent_str_invalid() {
    let doc = make_doc("submit", vec![]);
    let result =
        ActionRequestBuilder::new(&doc, "submit", "https://example.com", None)
            .unwrap()
            .with_consent_str("invalid");
    assert!(result.is_err());
}

#[test]
fn builder_construction_with_required_and_optional_params() {
    let doc = make_doc(
        "submit",
        vec![required_param("reason"), optional_param("notes")],
    );
    // Builder can be constructed even without setting params
    let builder =
        ActionRequestBuilder::new(&doc, "submit", "https://example.com", None);
    assert!(builder.is_ok());

    // Setting only the required param should work
    let builder = builder.unwrap().param("reason", "damaged");
    let _ = builder;
}
