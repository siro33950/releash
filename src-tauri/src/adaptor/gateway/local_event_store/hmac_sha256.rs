//! HMAC-SHA256 built on the existing `sha2` dependency (RFC 2104).
//!
//! Used for opaque cursor MACs. Verification is constant-time via `subtle`.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const BLOCK_SIZE: usize = 64;

pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        key_block[..32].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Sha256::new();
    let mut inner_pad = [0u8; BLOCK_SIZE];
    for (pad, key_byte) in inner_pad.iter_mut().zip(key_block.iter()) {
        *pad = key_byte ^ 0x36;
    }
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    let mut outer_pad = [0u8; BLOCK_SIZE];
    for (pad, key_byte) in outer_pad.iter_mut().zip(key_block.iter()) {
        *pad = key_byte ^ 0x5c;
    }
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();

    let mut output = [0u8; 32];
    output.copy_from_slice(&digest);
    output
}

pub fn verify_hmac_sha256(key: &[u8], message: &[u8], expected: &[u8; 32]) -> bool {
    let actual = hmac_sha256(key, message);
    bool::from(actual.as_slice().ct_eq(expected.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4231_test_case_2() {
        // Key = "Jefe", Data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn rfc4231_test_case_1() {
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex::encode(mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn long_key_is_hashed_first() {
        let key = [0xaau8; 131];
        let mac = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex::encode(mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn verify_rejects_modified_mac() {
        let mut mac = hmac_sha256(b"key", b"message");
        assert!(verify_hmac_sha256(b"key", b"message", &mac));
        mac[0] ^= 0x01;
        assert!(!verify_hmac_sha256(b"key", b"message", &mac));
    }
}
