use sha2::{Digest, Sha256};

use crate::hex::encode_hex;

pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_slice());
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    encode_hex(&sha256_bytes(data))
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    /// NIST FIPS 180-4 test vector: SHA-256("abc")
    #[test]
    fn vector_sha256_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// NIST FIPS 180-4 test vector: SHA-256("") (empty input)
    #[test]
    fn vector_sha256_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// TRI-SYNC protocol vector: SHA-256(0x00000000) — root digest of an empty BSM.
    /// This value is pinned in the TypeScript conformance suite and SPEC §3.5.
    #[test]
    fn vector_sha256_four_zero_bytes_empty_bsm() {
        assert_eq!(
            sha256_hex(&[0x00, 0x00, 0x00, 0x00]),
            "df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119"
        );
    }

    /// NIST FIPS 180-4 test vector: SHA-256("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
    #[test]
    fn vector_sha256_448bit_message() {
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
