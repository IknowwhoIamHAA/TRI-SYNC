use serde_json::{Map, Number, Value};

use crate::decimal::canonicalize_json_number;

pub fn to_canonical_string(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Value::Number(v) => out.push_str(&canonicalize_number(v)?),
        Value::String(v) => write_json_string(v, out),
        Value::Array(items) => {
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => write_object(map, out)?,
    }
    Ok(())
}

fn write_object(map: &Map<String, Value>, out: &mut String) -> Result<(), String> {
    let mut entries: Vec<(&str, &Value)> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
    entries.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));

    out.push('{');
    for (idx, (key, value)) in entries.into_iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        write_json_string(key, out);
        out.push(':');
        write_value(value, out)?;
    }
    out.push('}');
    Ok(())
}

fn canonicalize_number(number: &Number) -> Result<String, String> {
    canonicalize_json_number(number)
}

/// Write a JSON string value.
///
/// String values are written as raw UTF-8; no Unicode normalization is applied.
/// Per TRI-SYNC SPEC §5.3, implementations MUST NOT apply NFC, NFD, NFKC, or
/// NFKD normalization. Two strings that differ only in normalization form are
/// considered distinct values and will produce distinct digests.
///
/// Control characters U+0000–U+001F are escaped as `\uXXXX` with lowercase hex
/// digits (RFC 8785 §3.2.2).
fn write_json_string(input: &str, out: &mut String) {
    out.push('"');
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1F}' => {
                out.push_str("\\u");
                out.push(hex_digit(((c as u32) >> 12) as u8));
                out.push(hex_digit(((c as u32 >> 8) & 0xF) as u8));
                out.push(hex_digit(((c as u32 >> 4) & 0xF) as u8));
                out.push(hex_digit((c as u32 & 0xF) as u8));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::to_canonical_string;

    #[test]
    fn encodes_objects_in_sorted_key_order() {
        let value = json!({
            "z": {"b": 2, "a": 1},
            "a": [ {"k": 2, "j": 1} ]
        });

        let canonical = to_canonical_string(&value).expect("canonical encoding should succeed");
        assert_eq!(canonical, r#"{"a":[{"j":1,"k":2}],"z":{"a":1,"b":2}}"#);
    }

    #[test]
    fn normalizes_decimal_without_trailing_zeroes() {
        let value: serde_json::Value = serde_json::from_str("1.5000").expect("json parse");
        assert_eq!(
            to_canonical_string(&value).expect("canonical encoding"),
            "1.5"
        );
    }

    #[test]
    fn allows_negative_exponent_below_threshold() {
        let value: serde_json::Value = serde_json::from_str("1e-7").expect("json parse");
        assert_eq!(
            to_canonical_string(&value).expect("canonical encoding"),
            "0.0000001"
        );
    }

    // ---------------------------------------------------------------------------
    // No Unicode normalization — raw UTF-8 bytes must be preserved
    // ---------------------------------------------------------------------------

    #[test]
    fn raw_utf8_nfd_and_nfc_produce_different_canonical_json() {
        // SPEC §5.3: normalization is MUST NOT. NFD "a\u{0301}" and NFC "\u{00E1}"
        // are distinct byte sequences and MUST produce distinct canonical JSON.
        let nfd_value = serde_json::Value::String("a\u{0301}".to_string());
        let nfc_value = serde_json::Value::String("\u{00E1}".to_string());

        let nfd_canonical = to_canonical_string(&nfd_value).expect("nfd canonical");
        let nfc_canonical = to_canonical_string(&nfc_value).expect("nfc canonical");
        assert_ne!(
            nfd_canonical, nfc_canonical,
            "NFD and NFC forms must produce distinct canonical JSON (no normalization)"
        );
    }

    #[test]
    fn raw_utf8_nfd_and_nfc_object_keys_produce_different_canonical_json() {
        // Object keys that differ only in normalization form are distinct keys.
        let mut nfd_map = serde_json::Map::new();
        nfd_map.insert("a\u{0301}".to_string(), json!(1));
        let mut nfc_map = serde_json::Map::new();
        nfc_map.insert("\u{00E1}".to_string(), json!(1));

        let nfd_canonical = to_canonical_string(&serde_json::Value::Object(nfd_map)).expect("nfd");
        let nfc_canonical = to_canonical_string(&serde_json::Value::Object(nfc_map)).expect("nfc");
        assert_ne!(
            nfd_canonical, nfc_canonical,
            "NFD and NFC object keys must produce distinct canonical JSON"
        );
    }

    // ---------------------------------------------------------------------------
    // Fix 12: RFC 8785 / JCS conformance test vectors
    // ---------------------------------------------------------------------------

    #[test]
    fn rfc8785_null_value() {
        assert_eq!(
            to_canonical_string(&serde_json::Value::Null).expect("null"),
            "null"
        );
    }

    #[test]
    fn rfc8785_boolean_values() {
        assert_eq!(to_canonical_string(&json!(true)).expect("true"), "true");
        assert_eq!(to_canonical_string(&json!(false)).expect("false"), "false");
    }

    #[test]
    fn rfc8785_empty_object() {
        assert_eq!(to_canonical_string(&json!({})).expect("empty obj"), "{}");
    }

    #[test]
    fn rfc8785_empty_array() {
        assert_eq!(to_canonical_string(&json!([])).expect("empty arr"), "[]");
    }

    #[test]
    fn rfc8785_string_escapes_control_characters() {
        // U+0000 through U+001F must be escaped as \uXXXX with lowercase hex (RFC 8785 §3.2.2)
        let value = serde_json::Value::String("\u{0000}".to_string());
        let canonical = to_canonical_string(&value).expect("control char");
        assert_eq!(canonical, r#""\u0000""#);

        let value = serde_json::Value::String("\u{001F}".to_string());
        let canonical = to_canonical_string(&value).expect("control char");
        assert_eq!(canonical, r#""\u001f""#);
    }

    #[test]
    fn rfc8785_string_escapes_backslash_and_quote() {
        let value = serde_json::Value::String("a\"b\\c".to_string());
        assert_eq!(
            to_canonical_string(&value).expect("escapes"),
            r#""a\"b\\c""#
        );
    }

    #[test]
    fn rfc8785_nested_object_keys_sorted_lexicographically() {
        // RFC 8785 §3.2.3: keys sorted by their UTF-16 code unit sequence.
        // For ASCII-only keys, byte order equals UTF-16 code unit order.
        let value = json!({"b": 2, "a": 1, "c": 3});
        assert_eq!(
            to_canonical_string(&value).expect("sorted"),
            r#"{"a":1,"b":2,"c":3}"#
        );
    }

    #[test]
    fn rfc8785_array_preserves_order() {
        let value = json!([3, 1, 2]);
        assert_eq!(to_canonical_string(&value).expect("array"), "[3,1,2]");
    }

    #[test]
    fn rfc8785_integer_numbers() {
        assert_eq!(to_canonical_string(&json!(0)).expect("zero"), "0");
        assert_eq!(to_canonical_string(&json!(-1)).expect("neg"), "-1");
        assert_eq!(
            to_canonical_string(&json!(1234567890)).expect("large"),
            "1234567890"
        );
    }
}
