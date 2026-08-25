use serde_json::{Map, Number, Value};

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
    let rendered = number.to_string();

    if rendered.starts_with('+') {
        return Err("numeric canonicalization failed: '+' sign is forbidden".to_string());
    }

    if rendered == "-0" || rendered == "-0.0" {
        return Err("numeric canonicalization failed: '-0' is forbidden".to_string());
    }

    if rendered.contains('e') || rendered.contains('E') {
        return canonicalize_exponent(&rendered);
    }

    if let Some(dot) = rendered.find('.') {
        let integer_part = &rendered[..dot];
        let mut frac = rendered[dot + 1..].to_string();

        if has_invalid_leading_zero(integer_part) {
            return Err("numeric canonicalization failed: leading zeros are forbidden".to_string());
        }

        while frac.ends_with('0') {
            frac.pop();
        }

        if frac.is_empty() {
            if integer_part == "-0" {
                return Err("numeric canonicalization failed: '-0' is forbidden".to_string());
            }
            return Ok(integer_part.to_string());
        }

        if integer_part == "-0" {
            return Ok(format!("-0.{frac}"));
        }

        return Ok(format!("{integer_part}.{frac}"));
    }

    if has_invalid_leading_zero(&rendered) {
        return Err("numeric canonicalization failed: leading zeros are forbidden".to_string());
    }

    Ok(rendered)
}

fn canonicalize_exponent(number: &str) -> Result<String, String> {
    let normalized = number.replace('E', "e");
    let (mantissa, exponent) = normalized
        .split_once('e')
        .ok_or_else(|| "numeric canonicalization failed: malformed exponent".to_string())?;

    if exponent.starts_with('+') || !exponent.starts_with('-') {
        return Err("numeric canonicalization failed: positive exponent is forbidden".to_string());
    }

    let value = normalized
        .parse::<f64>()
        .map_err(|_| "numeric canonicalization failed: malformed decimal".to_string())?;
    if !value.is_finite() {
        return Err("numeric canonicalization failed: NaN/Infinity are forbidden".to_string());
    }

    if value.abs() >= 1e-6 {
        return Err(
            "numeric canonicalization failed: exponent notation only allowed for abs(value) < 1e-6"
                .to_string(),
        );
    }

    if mantissa.starts_with('+') || mantissa == "-0" {
        return Err("numeric canonicalization failed: invalid mantissa sign".to_string());
    }

    let mut chars = mantissa.chars();
    if matches!(chars.next(), Some('-')) {
        if !matches!(chars.next(), Some('1'..='9')) {
            return Err("numeric canonicalization failed: mantissa must be normalized".to_string());
        }
    } else if !matches!(mantissa.chars().next(), Some('1'..='9')) {
        return Err("numeric canonicalization failed: mantissa must be normalized".to_string());
    }

    let mut clean_mantissa = mantissa.to_string();
    if clean_mantissa.contains('.') {
        while clean_mantissa.ends_with('0') {
            clean_mantissa.pop();
        }
        if clean_mantissa.ends_with('.') {
            clean_mantissa.pop();
        }
    }

    if clean_mantissa == "-0" {
        return Err("numeric canonicalization failed: '-0' is forbidden".to_string());
    }

    Ok(format!("{clean_mantissa}e{exponent}"))
}

fn has_invalid_leading_zero(integer_part: &str) -> bool {
    if integer_part.starts_with("-0") {
        return integer_part.len() > 2;
    }
    integer_part.starts_with('0') && integer_part.len() > 1
}

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
        10..=15 => (b'A' + nibble - 10) as char,
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
            "1e-7"
        );
    }
}
