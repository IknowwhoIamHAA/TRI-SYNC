use serde_json::Number;

/// Maximum number of significant digits accepted in any decimal value.
/// Values exceeding this limit are rejected to prevent DoS via large-integer arithmetic.
pub const MAX_DECIMAL_DIGITS: usize = 256;

pub fn validate_decimal(input: &str) -> Result<(), String> {
    let canonical = canonicalize_decimal(input)?;
    if canonical == input {
        Ok(())
    } else {
        Err(format!(
            "INVALID_NUMERIC: non-canonical decimal form `{input}`, canonical form is `{canonical}`"
        ))
    }
}

pub fn canonicalize_json_number(number: &Number) -> Result<String, String> {
    canonicalize_decimal(&number.to_string())
}

pub fn canonicalize_decimal(input: &str) -> Result<String, String> {
    if input.is_empty() {
        return Err("INVALID_NUMERIC: decimal payload must not be empty".to_string());
    }
    if input.trim() != input {
        return Err("INVALID_NUMERIC: whitespace is forbidden in numeric values".to_string());
    }
    if input.starts_with('+') {
        return Err("INVALID_NUMERIC: '+' sign is forbidden".to_string());
    }

    let (negative, unsigned) = if let Some(rest) = input.strip_prefix('-') {
        (true, rest)
    } else {
        (false, input)
    };

    if unsigned.is_empty() {
        return Err("INVALID_NUMERIC: missing digits".to_string());
    }

    let (mantissa, exponent_part) = if let Some((m, e)) = unsigned.split_once(['e', 'E']) {
        if m.contains(['e', 'E']) || e.contains(['e', 'E']) {
            return Err("INVALID_NUMERIC: malformed exponent".to_string());
        }
        (m, Some(e))
    } else {
        (unsigned, None)
    };

    let exponent = parse_exponent(exponent_part)?;
    let (int_part, frac_part) = parse_mantissa(mantissa)?;

    let mut digits = String::with_capacity(int_part.len() + frac_part.len());
    digits.push_str(int_part);
    digits.push_str(frac_part);

    let non_zero_pos = digits.find(|ch| ch != '0');
    if non_zero_pos.is_none() {
        if negative {
            return Err("INVALID_NUMERIC: '-0' is forbidden".to_string());
        }
        return Ok("0".to_string());
    }
    let digits = digits[non_zero_pos.expect("checked above")..].to_string();

    if digits.len() > MAX_DECIMAL_DIGITS {
        return Err(format!(
            "INVALID_NUMERIC: decimal exceeds maximum digit count of {MAX_DECIMAL_DIGITS}"
        ));
    }
    let scale = frac_part.len() as i64;
    let shifted = exponent
        .checked_sub(scale)
        .ok_or_else(|| "INVALID_NUMERIC: exponent underflow".to_string())?;
    let abs_shift = shifted.unsigned_abs();

    let mut canonical = if shifted >= 0 {
        let mut out = digits;
        if abs_shift > usize::MAX as u64 {
            return Err("INVALID_NUMERIC: exponent too large".to_string());
        }
        out.push_str(&"0".repeat(abs_shift as usize));
        out
    } else {
        if abs_shift > usize::MAX as u64 {
            return Err("INVALID_NUMERIC: exponent too large".to_string());
        }
        let decimal_places = abs_shift as usize;
        if digits.len() > decimal_places {
            let split_at = digits.len() - decimal_places;
            format!("{}.{}", &digits[..split_at], &digits[split_at..])
        } else {
            let mut out = String::from("0.");
            out.push_str(&"0".repeat(decimal_places - digits.len()));
            out.push_str(&digits);
            out
        }
    };

    if let Some((whole, frac)) = canonical.split_once('.') {
        let trimmed = frac.trim_end_matches('0');
        canonical = if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{whole}.{trimmed}")
        };
    }

    if canonical.starts_with('0') && canonical.len() > 1 && !canonical.starts_with("0.") {
        return Err("INVALID_NUMERIC: leading zeros are forbidden".to_string());
    }
    if canonical.ends_with('.') {
        return Err("INVALID_NUMERIC: empty fractional parts are forbidden".to_string());
    }

    if negative {
        if canonical == "0" {
            return Err("INVALID_NUMERIC: '-0' is forbidden".to_string());
        }
        Ok(format!("-{canonical}"))
    } else {
        Ok(canonical)
    }
}

fn parse_exponent(exponent_part: Option<&str>) -> Result<i64, String> {
    let Some(exponent) = exponent_part else {
        return Ok(0);
    };
    if exponent.is_empty() {
        return Err("INVALID_NUMERIC: malformed exponent".to_string());
    }

    let (negative, digits) = if let Some(rest) = exponent.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = exponent.strip_prefix('+') {
        (false, rest)
    } else {
        (false, exponent)
    };

    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("INVALID_NUMERIC: malformed exponent".to_string());
    }

    let parsed = digits
        .parse::<i64>()
        .map_err(|_| "INVALID_NUMERIC: exponent out of range".to_string())?;
    if negative { Ok(-parsed) } else { Ok(parsed) }
}

fn parse_mantissa(mantissa: &str) -> Result<(&str, &str), String> {
    let (int_part, frac_part) = if let Some((whole, frac)) = mantissa.split_once('.') {
        (whole, frac)
    } else {
        (mantissa, "")
    };

    if int_part.is_empty() && frac_part.is_empty() {
        return Err("INVALID_NUMERIC: missing digits".to_string());
    }
    if int_part.is_empty() {
        return Err("INVALID_NUMERIC: missing integer part".to_string());
    }
    if mantissa.ends_with('.') {
        return Err("INVALID_NUMERIC: empty fractional parts are forbidden".to_string());
    }
    if !int_part.chars().all(|ch| ch.is_ascii_digit())
        || !frac_part.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err("INVALID_NUMERIC: decimal payload must contain only digits".to_string());
    }

    Ok((int_part, frac_part))
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_decimal, validate_decimal};

    #[test]
    fn canonicalizes_exponent_to_plain_decimal() {
        assert_eq!(
            canonicalize_decimal("1e-7").expect("canonicalize"),
            "0.0000001"
        );
    }

    #[test]
    fn canonicalizes_trailing_zeros() {
        assert_eq!(
            canonicalize_decimal("001.2300").expect("canonicalize"),
            "1.23"
        );
    }

    #[test]
    fn rejects_negative_zero() {
        assert!(canonicalize_decimal("-0").is_err());
        assert!(canonicalize_decimal("-0.0").is_err());
    }

    #[test]
    fn validates_only_already_canonical_forms() {
        assert!(validate_decimal("1.23").is_ok());
        assert!(validate_decimal("01.23").is_err());
        assert!(validate_decimal("1.230").is_err());
        assert!(validate_decimal("1e-7").is_err());
    }

    #[test]
    fn rejects_decimal_exceeding_digit_limit() {
        let long_integer = "1".repeat(257);
        assert!(canonicalize_decimal(&long_integer).is_err());

        let at_limit = "1".repeat(256);
        assert!(canonicalize_decimal(&at_limit).is_ok());
    }
}
