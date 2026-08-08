pub fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(nibble_to_hex((b >> 4) & 0x0f));
        out.push(nibble_to_hex(b & 0x0f));
    }
    out
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex value must have even length".to_string());
    }

    let mut out = Vec::with_capacity(value.len() / 2);
    let chars: Vec<char> = value.chars().collect();
    for i in (0..chars.len()).step_by(2) {
        let high = hex_to_nibble(chars[i])?;
        let low = hex_to_nibble(chars[i + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

fn hex_to_nibble(value: char) -> Result<u8, String> {
    value
        .to_digit(16)
        .map(|digit| digit as u8)
        .ok_or_else(|| format!("invalid hex character: {value}"))
}

#[cfg(test)]
mod tests {
    use super::{decode_hex, encode_hex};

    #[test]
    fn round_trips_hex_encoding() {
        let bytes = b"tri-sync";
        let encoded = encode_hex(bytes);
        let decoded = decode_hex(&encoded).expect("decode should succeed");
        assert_eq!(decoded, bytes);
    }
}
