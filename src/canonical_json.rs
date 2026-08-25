use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub fn to_canonical_string(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&normalize(value))
}

/// Returns `true` if `s` is a canonical decimal string.
///
/// Canonical form rules:
/// - Must be a valid number (no empty string)
/// - No exponent notation (`1e5`, `1.5E3`)
/// - No negative zero (`-0`, `-0.0`)
/// - No leading zeros in the integer part (`01`, `007`)
/// - No empty fractional part (`1.`)
/// - No trailing zeros in the fractional part (`1.50`, `0.10`)
pub fn is_canonical_decimal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Reject exponent notation.
    if s.contains('e') || s.contains('E') {
        return false;
    }

    let (negative, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };

    if digits.is_empty() {
        return false;
    }

    match digits.split_once('.') {
        Some((int_part, frac_part)) => {
            // Empty fractional part (trailing dot).
            if frac_part.is_empty() {
                return false;
            }
            // Trailing zeros in fractional part.
            if frac_part.ends_with('0') {
                return false;
            }
            // All characters must be ASCII digits.
            if !int_part.chars().all(|c| c.is_ascii_digit())
                || !frac_part.chars().all(|c| c.is_ascii_digit())
            {
                return false;
            }
            // Leading zeros in integer part (e.g. "01.5").
            if int_part.len() > 1 && int_part.starts_with('0') {
                return false;
            }
            true
        }
        None => {
            // Integer form.
            if !digits.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            // Leading zeros (e.g. "007").
            if digits.len() > 1 && digits.starts_with('0') {
                return false;
            }
            // Negative zero.
            if negative && digits == "0" {
                return false;
            }
            true
        }
    }
}

/// Converts `s` to canonical decimal form.
///
/// - Strips trailing zeros from the fractional part.
/// - Removes an empty fractional part (e.g. `"1."` → `"1"`).
/// - Normalises negative zero to `"0"`.
/// - Returns `Err` if `s` is not a valid decimal number or uses exponent
///   notation, which cannot be represented as a finite canonical decimal.
pub fn canonicalize_number(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("empty string is not a valid number".to_string());
    }

    if s.contains('e') || s.contains('E') {
        return Err(format!(
            "exponent notation is not supported in canonical form: {s}"
        ));
    }

    let (negative, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };

    if digits.is_empty() {
        return Err("bare '-' is not a valid number".to_string());
    }

    let (int_part, frac_part) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };

    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid characters in integer part: {int_part}"));
    }
    if let Some(f) = frac_part {
        if !f.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("invalid characters in fractional part: {f}"));
        }
    }

    // Strip leading zeros from integer part (preserve at least one digit).
    let int_canonical = int_part.trim_start_matches('0');
    let int_canonical = if int_canonical.is_empty() {
        "0"
    } else {
        int_canonical
    };

    // Strip trailing zeros from fractional part.
    let frac_canonical = frac_part.map(|f| f.trim_end_matches('0'));

    let result = match frac_canonical {
        Some(f) if !f.is_empty() => format!("{int_canonical}.{f}"),
        _ => int_canonical.to_string(),
    };

    // Normalise negative zero.
    if negative && result == "0" {
        return Ok("0".to_string());
    }

    if negative {
        Ok(format!("-{result}"))
    } else {
        Ok(result)
    }
}

fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in map {
                sorted.insert(key.clone(), normalize(value));
            }

            let mut normalized = Map::new();
            for (key, value) in sorted {
                normalized.insert(key, value);
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{canonicalize_number, is_canonical_decimal, to_canonical_string};

    #[test]
    fn encodes_objects_in_sorted_key_order() {
        let value = json!({
            "z": {"b": 2, "a": 1},
            "a": [ {"k": 2, "j": 1} ]
        });

        let canonical = to_canonical_string(&value).expect("canonical encoding should succeed");
        assert_eq!(canonical, r#"{"a":[{"j":1,"k":2}],"z":{"a":1,"b":2}}"#);
    }

    // --- is_canonical_decimal ---

    #[test]
    fn accepts_valid_canonical_decimals() {
        assert!(is_canonical_decimal("0"));
        assert!(is_canonical_decimal("1"));
        assert!(is_canonical_decimal("-1"));
        assert!(is_canonical_decimal("123"));
        assert!(is_canonical_decimal("0.5"));
        assert!(is_canonical_decimal("-3.14"));
        assert!(is_canonical_decimal("1.1"));
        assert!(is_canonical_decimal("10.01"));
    }

    #[test]
    fn rejects_negative_zero() {
        assert!(!is_canonical_decimal("-0"));
        // "-0.5" is a valid non-zero negative decimal, not negative zero.
        assert!(is_canonical_decimal("-0.5"));
    }

    #[test]
    fn rejects_exponent_notation() {
        assert!(!is_canonical_decimal("1e5"));
        assert!(!is_canonical_decimal("1.5E3"));
        assert!(!is_canonical_decimal("-2e10"));
    }

    #[test]
    fn rejects_leading_zeros() {
        assert!(!is_canonical_decimal("01"));
        assert!(!is_canonical_decimal("007"));
        assert!(!is_canonical_decimal("01.5"));
    }

    #[test]
    fn rejects_trailing_zeros_in_fractional_part() {
        assert!(!is_canonical_decimal("1.50"));
        assert!(!is_canonical_decimal("0.10"));
        assert!(!is_canonical_decimal("3.1400"));
    }

    #[test]
    fn rejects_empty_fractional_part() {
        assert!(!is_canonical_decimal("1."));
        assert!(!is_canonical_decimal("-5."));
    }

    #[test]
    fn rejects_empty_string() {
        assert!(!is_canonical_decimal(""));
    }

    // --- canonicalize_number ---

    #[test]
    fn canonicalizes_trailing_dot() {
        assert_eq!(canonicalize_number("1.").unwrap(), "1");
        assert_eq!(canonicalize_number("-5.").unwrap(), "-5");
    }

    #[test]
    fn canonicalizes_trailing_zeros() {
        assert_eq!(canonicalize_number("1.50").unwrap(), "1.5");
        assert_eq!(canonicalize_number("0.100").unwrap(), "0.1");
        assert_eq!(canonicalize_number("3.1400").unwrap(), "3.14");
    }

    #[test]
    fn canonicalizes_leading_zeros() {
        assert_eq!(canonicalize_number("007").unwrap(), "7");
        assert_eq!(canonicalize_number("01.5").unwrap(), "1.5");
    }

    #[test]
    fn canonicalizes_negative_zero_to_zero() {
        assert_eq!(canonicalize_number("-0").unwrap(), "0");
        assert_eq!(canonicalize_number("-0.0").unwrap(), "0");
    }

    #[test]
    fn canonicalize_number_rejects_exponent_notation() {
        assert!(canonicalize_number("1e5").is_err());
        assert!(canonicalize_number("1.5E3").is_err());
    }

    #[test]
    fn canonicalize_number_rejects_empty_string() {
        assert!(canonicalize_number("").is_err());
    }

    #[test]
    fn already_canonical_numbers_are_unchanged() {
        for s in &["0", "1", "-1", "0.5", "-3.14", "123", "10.01"] {
            assert_eq!(canonicalize_number(s).unwrap(), *s);
        }
    }
}
