use rand::distr::Alphanumeric;
use rand::RngExt;

pub const TOKEN_LENGTH: usize = 48;

pub fn generate_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LENGTH)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod services_tests {
    use super::*;

    #[test]
    fn test_トークン生成_48文字英数字になる() {
        // Given / When
        let token = generate_token();

        // Then
        assert_eq!(token.len(), TOKEN_LENGTH);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_トークン生成_連続生成で異なる値になる() {
        // Given / When
        let first = generate_token();
        let second = generate_token();

        // Then
        assert_ne!(first, second);
    }
}
