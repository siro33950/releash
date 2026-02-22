use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub(super) const CHALLENGE_LENGTH: usize = 32;

pub(super) fn generate_challenge() -> String {
    let bytes: Vec<u8> = (0..CHALLENGE_LENGTH)
        .map(|_| rand::random::<u8>())
        .collect();
    hex::encode(bytes)
}

pub(super) fn verify_hmac(challenge: &str, token: &str, client_hmac: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(token.as_bytes()) else {
        return false;
    };
    mac.update(challenge.as_bytes());
    let Ok(client_bytes) = hex::decode(client_hmac) else {
        return false;
    };
    mac.verify_slice(&client_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_challenge_length() {
        let c = generate_challenge();
        assert_eq!(c.len(), CHALLENGE_LENGTH * 2); // hex encoding doubles length
        assert!(c.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_challenge_uniqueness() {
        let c1 = generate_challenge();
        let c2 = generate_challenge();
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_verify_hmac_valid() {
        let challenge = "test_challenge";
        let token = "secret_token";

        let mut mac = HmacSha256::new_from_slice(token.as_bytes()).unwrap();
        mac.update(challenge.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        assert!(verify_hmac(challenge, token, &expected));
    }

    #[test]
    fn test_verify_hmac_invalid() {
        assert!(!verify_hmac("challenge", "token", "wrong_hmac"));
    }

    #[test]
    fn test_verify_hmac_empty_token() {
        assert!(!verify_hmac("challenge", "", "abcdef"));
    }

    #[test]
    fn test_verify_hmac_invalid_hex() {
        assert!(!verify_hmac("challenge", "token", "not_valid_hex_zzz"));
    }

    #[test]
    fn test_verify_hmac_wrong_challenge() {
        let token = "secret_token";
        let mut mac = HmacSha256::new_from_slice(token.as_bytes()).unwrap();
        mac.update(b"correct_challenge");
        let hmac_hex = hex::encode(mac.finalize().into_bytes());
        assert!(!verify_hmac("wrong_challenge", token, &hmac_hex));
    }
}
