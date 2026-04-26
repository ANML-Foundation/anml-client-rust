//! Parameter binding algorithm for all 3 enctypes with canonical forms.
//!
//! Implements the RFC parameter binding algorithm (Section 10.9) for:
//! - `application/x-www-form-urlencoded`
//! - `multipart/form-data`
//! - `application/json`
//!
//! Canonical forms are critical for idempotency key stability — same input
//! MUST produce identical bytes.

use std::collections::BTreeMap;

use anml::types::elements::AnmlParam;
use anml::types::enums::ParamType;

// ---------------------------------------------------------------------------
// ParamValue — a typed parameter value
// ---------------------------------------------------------------------------

/// A parameter name-value pair with optional type information.
#[derive(Clone, Debug)]
pub struct ParamValue {
    /// The parameter name.
    pub name: String,
    /// The string value.
    pub value: String,
    /// The declared type (from `<param>`), if any.
    pub param_type: Option<ParamType>,
}

// ---------------------------------------------------------------------------
// Canonical form helpers
// ---------------------------------------------------------------------------

/// Convert a value to its XSD canonical lexical form based on type.
pub fn canonical_value(value: &str, param_type: Option<&ParamType>) -> String {
    match param_type {
        Some(ParamType::Number) => canonical_number(value),
        Some(ParamType::Boolean) => canonical_boolean(value),
        Some(ParamType::Datetime) => canonical_datetime(value),
        _ => value.to_string(),
    }
}

/// XSD decimal canonical form for numbers.
fn canonical_number(value: &str) -> String {
    if let Ok(n) = value.parse::<f64>() {
        // If it's an integer value, emit without decimal point
        if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
            format!("{}", n as i64)
        } else {
            // Use Rust's default float formatting (no trailing zeros beyond needed)
            let s = format!("{}", n);
            s
        }
    } else {
        value.to_string()
    }
}

/// XSD boolean canonical form: `true` or `false`.
fn canonical_boolean(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "true" | "1" => "true".to_string(),
        "false" | "0" => "false".to_string(),
        _ => value.to_string(),
    }
}

/// RFC 3339 canonical form for dateTime values.
fn canonical_datetime(value: &str) -> String {
    // Already in RFC 3339 format — pass through
    value.to_string()
}

// ---------------------------------------------------------------------------
// URL-encoded encoding
// ---------------------------------------------------------------------------

/// Encode parameters as `application/x-www-form-urlencoded`.
///
/// - Space → `+`
/// - Unreserved set: `ALPHA / DIGIT / "-" / "." / "_" / "~"`
/// - Pairs joined by `&` in document order
/// - Typed values use XSD canonical forms
pub fn encode_urlencoded(params: &[ParamValue]) -> Vec<u8> {
    let pairs: Vec<String> = params
        .iter()
        .map(|p| {
            let canon = canonical_value(&p.value, p.param_type.as_ref());
            format!(
                "{}={}",
                percent_encode(&p.name),
                percent_encode(&canon)
            )
        })
        .collect();
    pairs.join("&").into_bytes()
}

/// Percent-encode a string per RFC 3986 with space→`+`.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b' ' => out.push('+'),
            b if is_unreserved(b) => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX_UPPER[(byte >> 4) as usize] as char);
                out.push(HEX_UPPER[(byte & 0x0F) as usize] as char);
            }
        }
    }
    out
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

// ---------------------------------------------------------------------------
// Multipart encoding
// ---------------------------------------------------------------------------

/// Encode parameters as `multipart/form-data`.
///
/// - Boundary = `anml-` + doc_id + 96-bit crypto random (base32-no-pad)
/// - Parts in document order
/// - Returns (content_type, body_bytes)
pub fn encode_multipart(params: &[ParamValue], doc_id: &str) -> (String, Vec<u8>) {
    let boundary = generate_boundary(doc_id);
    let content_type = format!("multipart/form-data; boundary={}", boundary);

    let mut body = Vec::new();
    for p in params {
        let canon = canonical_value(&p.value, p.param_type.as_ref());
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"\r\n", p.name).as_bytes(),
        );
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(canon.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    // Final boundary
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    (content_type, body)
}

/// Generate a multipart boundary: `anml-` + doc_id + 96-bit crypto random (base32-no-pad).
fn generate_boundary(doc_id: &str) -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut random_bytes = [0u8; 12]; // 96 bits
    rng.fill_bytes(&mut random_bytes);
    let encoded = base32_encode_no_pad(&random_bytes);
    format!("anml-{}{}", doc_id, encoded)
}

/// Base32 encode without padding (RFC 4648).
fn base32_encode_no_pad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::new();
    let mut buffer: u64 = 0;
    let mut bits_left = 0;

    for &byte in data {
        buffer = (buffer << 8) | byte as u64;
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let index = ((buffer >> bits_left) & 0x1F) as usize;
            result.push(ALPHABET[index] as char);
        }
    }
    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1F) as usize;
        result.push(ALPHABET[index] as char);
    }
    result
}

// ---------------------------------------------------------------------------
// JSON encoding
// ---------------------------------------------------------------------------

/// Encode parameters as `application/json`.
///
/// - Keys in document order (MUST NOT sort)
/// - Numbers as JSON numbers, booleans as JSON booleans
/// - dateTime as RFC 3339 strings
/// - Minified UTF-8, no BOM
pub fn encode_json(params: &[ParamValue]) -> Vec<u8> {
    // We build JSON manually to preserve document order (serde_json's Map
    // sorts keys by default with BTreeMap, and IndexMap would add a dep).
    let mut out = String::from("{");
    let mut first = true;

    // Track duplicate names → collect into arrays
    let mut seen: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, p) in params.iter().enumerate() {
        seen.entry(p.name.clone()).or_default().push(i);
    }

    // Emit in document order, but group duplicates into arrays
    let mut emitted = std::collections::HashSet::new();
    for p in params {
        if emitted.contains(&p.name) {
            continue;
        }
        emitted.insert(p.name.clone());

        if !first {
            out.push(',');
        }
        first = false;

        out.push('"');
        json_escape_string(&p.name, &mut out);
        out.push('"');
        out.push(':');

        let indices = &seen[&p.name];
        if indices.len() > 1 {
            // Duplicate names → array
            out.push('[');
            for (j, &idx) in indices.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                json_encode_value(&params[idx], &mut out);
            }
            out.push(']');
        } else {
            json_encode_value(p, &mut out);
        }
    }

    out.push('}');
    out.into_bytes()
}

fn json_encode_value(p: &ParamValue, out: &mut String) {
    match p.param_type {
        Some(ParamType::Number) => {
            if let Ok(n) = p.value.parse::<f64>() {
                if n.is_finite() {
                    // Check if it's an integer
                    if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
                        out.push_str(&format!("{}", n as i64));
                    } else {
                        out.push_str(&format!("{}", n));
                    }
                } else {
                    // Out-of-range: emit as string
                    out.push('"');
                    json_escape_string(&p.value, out);
                    out.push('"');
                }
            } else {
                out.push('"');
                json_escape_string(&p.value, out);
                out.push('"');
            }
        }
        Some(ParamType::Boolean) => {
            let canon = canonical_boolean(&p.value);
            out.push_str(&canon);
        }
        _ => {
            // String, date, datetime, uri, enum → JSON string
            let canon = canonical_value(&p.value, p.param_type.as_ref());
            out.push('"');
            json_escape_string(&canon, out);
            out.push('"');
        }
    }
}

fn json_escape_string(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// Collect parameter values from `<param>` definitions and user-supplied values.
///
/// Walks params in document order, applies defaults for missing optional params,
/// strips leading/trailing ASCII whitespace from values.
pub fn collect_params(
    param_defs: &[AnmlParam],
    user_values: &[(String, String)],
) -> Vec<ParamValue> {
    let user_map: std::collections::HashMap<&str, &str> = user_values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let mut result = Vec::new();
    for def in param_defs {
        let value = user_map
            .get(def.name.as_str())
            .map(|v| v.trim().to_string())
            .or_else(|| def.default.clone());

        if let Some(val) = value {
            result.push(ParamValue {
                name: def.name.clone(),
                value: val,
                param_type: def.param_type,
            });
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_basic() {
        let params = vec![
            ParamValue { name: "airline".into(), value: "Delta Air".into(), param_type: None },
            ParamValue { name: "class".into(), value: "economy".into(), param_type: None },
        ];
        let encoded = encode_urlencoded(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, "airline=Delta+Air&class=economy");
    }

    #[test]
    fn urlencoded_special_chars() {
        let params = vec![
            ParamValue { name: "q".into(), value: "hello world!".into(), param_type: None },
        ];
        let encoded = encode_urlencoded(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, "q=hello+world%21");
    }

    #[test]
    fn urlencoded_number_canonical() {
        let params = vec![
            ParamValue { name: "price".into(), value: "42.0".into(), param_type: Some(ParamType::Number) },
        ];
        let encoded = encode_urlencoded(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, "price=42");
    }

    #[test]
    fn urlencoded_boolean_canonical() {
        let params = vec![
            ParamValue { name: "active".into(), value: "true".into(), param_type: Some(ParamType::Boolean) },
        ];
        let encoded = encode_urlencoded(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, "active=true");
    }

    #[test]
    fn json_preserves_order() {
        let params = vec![
            ParamValue { name: "z_field".into(), value: "last".into(), param_type: None },
            ParamValue { name: "a_field".into(), value: "first".into(), param_type: None },
        ];
        let encoded = encode_json(&params);
        let s = String::from_utf8(encoded).unwrap();
        // Keys must be in document order, NOT sorted
        assert!(s.starts_with("{\"z_field\":"));
        assert_eq!(s, r#"{"z_field":"last","a_field":"first"}"#);
    }

    #[test]
    fn json_number_as_json_number() {
        let params = vec![
            ParamValue { name: "price".into(), value: "349".into(), param_type: Some(ParamType::Number) },
        ];
        let encoded = encode_json(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, r#"{"price":349}"#);
    }

    #[test]
    fn json_boolean_as_json_boolean() {
        let params = vec![
            ParamValue { name: "active".into(), value: "true".into(), param_type: Some(ParamType::Boolean) },
        ];
        let encoded = encode_json(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, r#"{"active":true}"#);
    }

    #[test]
    fn json_duplicate_names_become_array() {
        let params = vec![
            ParamValue { name: "tag".into(), value: "a".into(), param_type: None },
            ParamValue { name: "tag".into(), value: "b".into(), param_type: None },
        ];
        let encoded = encode_json(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert_eq!(s, r#"{"tag":["a","b"]}"#);
    }

    #[test]
    fn json_escapes_special_chars() {
        let params = vec![
            ParamValue { name: "msg".into(), value: "hello \"world\"\nnewline".into(), param_type: None },
        ];
        let encoded = encode_json(&params);
        let s = String::from_utf8(encoded).unwrap();
        assert!(s.contains("\\\"world\\\""));
        assert!(s.contains("\\n"));
    }

    #[test]
    fn multipart_has_boundary() {
        let params = vec![
            ParamValue { name: "airline".into(), value: "Delta".into(), param_type: None },
        ];
        let (ct, body) = encode_multipart(&params, "test-doc");
        assert!(ct.starts_with("multipart/form-data; boundary=anml-test-doc"));
        let body_str = String::from_utf8(body).unwrap();
        assert!(body_str.contains("Content-Disposition: form-data; name=\"airline\""));
        assert!(body_str.contains("Delta"));
    }

    #[test]
    fn collect_params_applies_defaults() {
        let defs = vec![
            AnmlParam {
                name: "airline".into(),
                param_type: None,
                required: Some(true),
                default: None,
                description: None,
                pattern: None,
                min: None,
                max: None,
                options: None,
            },
            AnmlParam {
                name: "class".into(),
                param_type: None,
                required: None,
                default: Some("economy".into()),
                description: None,
                pattern: None,
                min: None,
                max: None,
                options: None,
            },
        ];
        let user = vec![("airline".to_string(), "Delta".to_string())];
        let collected = collect_params(&defs, &user);
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].name, "airline");
        assert_eq!(collected[0].value, "Delta");
        assert_eq!(collected[1].name, "class");
        assert_eq!(collected[1].value, "economy");
    }

    #[test]
    fn collect_params_strips_whitespace() {
        let defs = vec![AnmlParam {
            name: "name".into(),
            param_type: None,
            required: None,
            default: None,
            description: None,
            pattern: None,
            min: None,
            max: None,
            options: None,
        }];
        let user = vec![("name".to_string(), "  Alice  ".to_string())];
        let collected = collect_params(&defs, &user);
        assert_eq!(collected[0].value, "Alice");
    }

    #[test]
    fn base32_encode_produces_valid_output() {
        let data = [0xFF, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE];
        let encoded = base32_encode_no_pad(&data);
        assert!(!encoded.is_empty());
        // All chars should be in base32 alphabet
        assert!(encoded.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn percent_encode_unreserved_passthrough() {
        assert_eq!(percent_encode("hello"), "hello");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(percent_encode("ABC123"), "ABC123");
    }

    #[test]
    fn percent_encode_space_to_plus() {
        assert_eq!(percent_encode("hello world"), "hello+world");
    }

    #[test]
    fn percent_encode_special() {
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
    }
}
