//! Property-based tests for parameter binding.
//!
//! **Validates: Requirements 10.1** (parameter binding algorithm)

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::action::params::*;
    use anml::types::enums::ParamType;

    // -----------------------------------------------------------------------
    // Arbitrary generators
    // -----------------------------------------------------------------------

    fn arb_param_name() -> impl Strategy<Value = String> {
        "[a-z_]{1,12}"
    }

    fn arb_param_value() -> impl Strategy<Value = ParamValue> {
        (arb_param_name(), "[a-zA-Z0-9 ._-]{0,30}", arb_param_type()).prop_map(
            |(name, value, param_type)| ParamValue {
                name,
                value,
                param_type,
            },
        )
    }

    fn arb_param_type() -> impl Strategy<Value = Option<ParamType>> {
        prop_oneof![
            Just(None),
            Just(Some(ParamType::String)),
            Just(Some(ParamType::Number)),
            Just(Some(ParamType::Boolean)),
        ]
    }

    fn arb_param_set() -> impl Strategy<Value = Vec<ParamValue>> {
        prop::collection::vec(arb_param_value(), 1..=5)
    }

    // -----------------------------------------------------------------------
    // Properties
    // -----------------------------------------------------------------------

    proptest! {
        /// URL-encoded output is deterministic: same input → same bytes.
        #[test]
        fn urlencoded_deterministic(params in arb_param_set()) {
            let a = encode_urlencoded(&params);
            let b = encode_urlencoded(&params);
            prop_assert_eq!(a, b, "urlencoded must be deterministic");
        }

        /// JSON preserves document order (keys not sorted).
        #[test]
        fn json_preserves_order(params in arb_param_set()) {
            let encoded = encode_json(&params);
            let json_str = String::from_utf8(encoded).unwrap();

            // Verify that the first param's name appears before the last param's name
            if params.len() >= 2 {
                let first_name = &params[0].name;
                let last_name = &params[params.len() - 1].name;
                if first_name != last_name {
                    let first_pos = json_str.find(&format!("\"{}\"", first_name));
                    let last_pos = json_str.find(&format!("\"{}\"", last_name));
                    if let (Some(fp), Some(lp)) = (first_pos, last_pos) {
                        prop_assert!(fp < lp, "JSON must preserve document order: {} should come before {}", first_name, last_name);
                    }
                }
            }
        }

        /// Multipart boundary contains randomness (different each call).
        #[test]
        fn multipart_boundary_has_randomness(params in arb_param_set()) {
            let (ct1, _) = encode_multipart(&params, "test");
            let (ct2, _) = encode_multipart(&params, "test");
            // Boundaries should differ due to crypto random component
            prop_assert_ne!(ct1, ct2, "multipart boundaries should differ");
        }

        /// Typed values use canonical forms.
        #[test]
        fn typed_values_canonical(
            int_val in -1000i64..1000i64,
            bool_val in any::<bool>(),
        ) {
            // Number canonical: integer without decimal
            let num_canon = canonical_value(&format!("{}.0", int_val), Some(&ParamType::Number));
            prop_assert_eq!(num_canon, int_val.to_string());

            // Boolean canonical: "true" or "false"
            let bool_str = if bool_val { "1" } else { "0" };
            let bool_canon = canonical_value(bool_str, Some(&ParamType::Boolean));
            let expected = if bool_val { "true" } else { "false" };
            prop_assert_eq!(bool_canon, expected);
        }
    }
}
