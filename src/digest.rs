use sha2::{Digest, Sha256};

use crate::hex::encode_hex;

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    encode_hex(digest.as_slice())
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
