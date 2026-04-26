//! Parameter validation against `<param>` constraints.
//!
//! Validates param values against type, required, pattern, min, max, and enum
//! options. Errors reference param name + constraint + expected + actual.

use anml::types::elements::{AnmlParam, AnmlOption};
use anml::types::enums::ParamType;

use crate::error::AnmlClientError;

/// Validate a single parameter value against its `<param>` definition.
///
/// Checks: required, type, pattern, min, max, enum options.
pub fn validate_param(param: &AnmlParam, value: Option<&str>) -> crate::Result<()> {
    let name = &param.name;

    // Check required
    if param.required == Some(true) && value.is_none() {
        return Err(AnmlClientError::ParamValidation {
            param: name.clone(),
            constraint: "required".into(),
            expected: "a value".into(),
            actual: "(missing)".into(),
        });
    }

    let value = match value {
        Some(v) => v,
        None => return Ok(()), // optional and not provided
    };

    // Check type
    if let Some(ref pt) = param.param_type {
        validate_type(name, value, pt)?;
    }

    // Check pattern
    if let Some(ref pattern) = param.pattern {
        validate_pattern(name, value, pattern)?;
    }

    // Check min
    if let Some(ref min) = param.min {
        validate_min(name, value, min, param.param_type.as_ref())?;
    }

    // Check max
    if let Some(ref max) = param.max {
        validate_max(name, value, max, param.param_type.as_ref())?;
    }

    // Check enum options
    if let Some(ref options) = param.options {
        validate_enum(name, value, options)?;
    }

    Ok(())
}

fn validate_type(name: &str, value: &str, param_type: &ParamType) -> crate::Result<()> {
    let valid = match param_type {
        ParamType::String => true,
        ParamType::Number => value.parse::<f64>().is_ok(),
        ParamType::Boolean => matches!(value, "true" | "false"),
        ParamType::Date => is_valid_date(value),
        ParamType::Datetime => is_valid_datetime(value),
        ParamType::Uri => url::Url::parse(value).is_ok(),
        ParamType::Enum => true, // validated separately via options
        _ => true,
    };

    if !valid {
        return Err(AnmlClientError::ParamValidation {
            param: name.to_string(),
            constraint: "type".into(),
            expected: param_type.to_string(),
            actual: value.to_string(),
        });
    }
    Ok(())
}

fn validate_pattern(name: &str, value: &str, pattern: &str) -> crate::Result<()> {
    // Anchor the pattern to match the full value
    let anchored = if pattern.starts_with('^') && pattern.ends_with('$') {
        pattern.to_string()
    } else if pattern.starts_with('^') {
        format!("{}$", pattern)
    } else if pattern.ends_with('$') {
        format!("^{}", pattern)
    } else {
        format!("^{}$", pattern)
    };

    // Use a simple regex-like check. For production, we'd use the `regex` crate,
    // but to avoid adding a dependency, we do a basic check.
    // The pattern attribute uses XSD regex syntax per the RFC.
    // For now, we attempt a basic match.
    match regex_lite_match(&anchored, value) {
        Some(true) => Ok(()),
        _ => Err(AnmlClientError::ParamValidation {
            param: name.to_string(),
            constraint: "pattern".into(),
            expected: pattern.to_string(),
            actual: value.to_string(),
        }),
    }
}

/// Minimal regex matching using basic patterns.
/// Returns None if the pattern is too complex to evaluate without a regex engine.
fn regex_lite_match(pattern: &str, value: &str) -> Option<bool> {
    // Strip anchors for comparison
    let inner = pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .strip_suffix('$')
        .unwrap_or(pattern);

    // If the pattern is a simple literal, do exact match
    if inner.chars().all(|c| !is_regex_meta(c)) {
        return Some(value == inner);
    }

    // For patterns with metacharacters, we can't reliably match without
    // a regex engine. Return None to indicate we can't validate.
    None
}

fn is_regex_meta(c: char) -> bool {
    matches!(c, '.' | '*' | '+' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '\\')
}

fn validate_min(
    name: &str,
    value: &str,
    min: &str,
    param_type: Option<&ParamType>,
) -> crate::Result<()> {
    // For numeric types, compare as numbers
    if matches!(param_type, Some(ParamType::Number)) {
        if let (Ok(v), Ok(m)) = (value.parse::<f64>(), min.parse::<f64>()) {
            if v < m {
                return Err(AnmlClientError::ParamValidation {
                    param: name.to_string(),
                    constraint: "min".into(),
                    expected: format!(">= {}", min),
                    actual: value.to_string(),
                });
            }
        }
    } else {
        // For strings, compare length
        if let Ok(m) = min.parse::<usize>() {
            if value.len() < m {
                return Err(AnmlClientError::ParamValidation {
                    param: name.to_string(),
                    constraint: "min".into(),
                    expected: format!(">= {} chars", min),
                    actual: format!("{} chars", value.len()),
                });
            }
        }
    }
    Ok(())
}

fn validate_max(
    name: &str,
    value: &str,
    max: &str,
    param_type: Option<&ParamType>,
) -> crate::Result<()> {
    if matches!(param_type, Some(ParamType::Number)) {
        if let (Ok(v), Ok(m)) = (value.parse::<f64>(), max.parse::<f64>()) {
            if v > m {
                return Err(AnmlClientError::ParamValidation {
                    param: name.to_string(),
                    constraint: "max".into(),
                    expected: format!("<= {}", max),
                    actual: value.to_string(),
                });
            }
        }
    } else {
        if let Ok(m) = max.parse::<usize>() {
            if value.len() > m {
                return Err(AnmlClientError::ParamValidation {
                    param: name.to_string(),
                    constraint: "max".into(),
                    expected: format!("<= {} chars", max),
                    actual: format!("{} chars", value.len()),
                });
            }
        }
    }
    Ok(())
}

fn validate_enum(name: &str, value: &str, options: &[AnmlOption]) -> crate::Result<()> {
    if options.is_empty() {
        return Ok(());
    }
    let allowed: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
    if !allowed.contains(&value) {
        return Err(AnmlClientError::ParamValidation {
            param: name.to_string(),
            constraint: "enum".into(),
            expected: format!("[{}]", allowed.join(", ")),
            actual: value.to_string(),
        });
    }
    Ok(())
}

fn is_valid_date(value: &str) -> bool {
    // YYYY-MM-DD
    if value.len() != 10 {
        return false;
    }
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts[0].parse::<u16>().is_ok()
        && parts[1].parse::<u8>().map_or(false, |m| (1..=12).contains(&m))
        && parts[2].parse::<u8>().map_or(false, |d| (1..=31).contains(&d))
}

fn is_valid_datetime(value: &str) -> bool {
    // Basic ISO 8601 / RFC 3339 check
    value.contains('T') && value.len() >= 19
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_param(name: &str) -> AnmlParam {
        AnmlParam {
            name: name.to_string(),
            param_type: None,
            required: None,
            default: None,
            description: None,
            pattern: None,
            min: None,
            max: None,
            options: None,
        }
    }

    #[test]
    fn required_param_missing_fails() {
        let mut p = make_param("reason");
        p.required = Some(true);
        let result = validate_param(&p, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AnmlClientError::ParamValidation { ref constraint, .. } if constraint == "required"));
    }

    #[test]
    fn optional_param_missing_ok() {
        let p = make_param("notes");
        assert!(validate_param(&p, None).is_ok());
    }

    #[test]
    fn number_type_valid() {
        let mut p = make_param("age");
        p.param_type = Some(ParamType::Number);
        assert!(validate_param(&p, Some("42")).is_ok());
        assert!(validate_param(&p, Some("3.14")).is_ok());
        assert!(validate_param(&p, Some("-1")).is_ok());
    }

    #[test]
    fn number_type_invalid() {
        let mut p = make_param("age");
        p.param_type = Some(ParamType::Number);
        assert!(validate_param(&p, Some("abc")).is_err());
    }

    #[test]
    fn boolean_type_valid() {
        let mut p = make_param("active");
        p.param_type = Some(ParamType::Boolean);
        assert!(validate_param(&p, Some("true")).is_ok());
        assert!(validate_param(&p, Some("false")).is_ok());
    }

    #[test]
    fn boolean_type_invalid() {
        let mut p = make_param("active");
        p.param_type = Some(ParamType::Boolean);
        assert!(validate_param(&p, Some("yes")).is_err());
    }

    #[test]
    fn date_type_valid() {
        let mut p = make_param("dob");
        p.param_type = Some(ParamType::Date);
        assert!(validate_param(&p, Some("2024-01-15")).is_ok());
    }

    #[test]
    fn date_type_invalid() {
        let mut p = make_param("dob");
        p.param_type = Some(ParamType::Date);
        assert!(validate_param(&p, Some("not-a-date")).is_err());
    }

    #[test]
    fn enum_valid() {
        let mut p = make_param("reason");
        p.options = Some(vec![
            AnmlOption { value: "damaged".into(), label: None },
            AnmlOption { value: "wrong-item".into(), label: None },
        ]);
        assert!(validate_param(&p, Some("damaged")).is_ok());
    }

    #[test]
    fn enum_invalid() {
        let mut p = make_param("reason");
        p.options = Some(vec![
            AnmlOption { value: "damaged".into(), label: None },
        ]);
        let result = validate_param(&p, Some("broken"));
        assert!(result.is_err());
    }

    #[test]
    fn min_number_valid() {
        let mut p = make_param("qty");
        p.param_type = Some(ParamType::Number);
        p.min = Some("1".into());
        assert!(validate_param(&p, Some("5")).is_ok());
    }

    #[test]
    fn min_number_invalid() {
        let mut p = make_param("qty");
        p.param_type = Some(ParamType::Number);
        p.min = Some("1".into());
        assert!(validate_param(&p, Some("0")).is_err());
    }

    #[test]
    fn max_number_valid() {
        let mut p = make_param("qty");
        p.param_type = Some(ParamType::Number);
        p.max = Some("100".into());
        assert!(validate_param(&p, Some("50")).is_ok());
    }

    #[test]
    fn max_number_invalid() {
        let mut p = make_param("qty");
        p.param_type = Some(ParamType::Number);
        p.max = Some("100".into());
        assert!(validate_param(&p, Some("200")).is_err());
    }

    #[test]
    fn pattern_literal_match() {
        let mut p = make_param("code");
        p.pattern = Some("ABC".into());
        assert!(validate_param(&p, Some("ABC")).is_ok());
    }

    #[test]
    fn pattern_literal_mismatch() {
        let mut p = make_param("code");
        p.pattern = Some("ABC".into());
        assert!(validate_param(&p, Some("XYZ")).is_err());
    }
}
