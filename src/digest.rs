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

    #[test]
    fn hashes_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
