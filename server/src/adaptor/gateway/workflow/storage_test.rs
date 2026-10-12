use super::*;

#[test]
fn validate_name_rejects_leading_special_chars() {
    assert!(validation::validate_name("-starts-with-dash").is_err());
    assert!(validation::validate_name("_starts-with-underscore").is_err());
    assert!(validation::validate_name("a-valid-name").is_ok());
    assert!(validation::validate_name("1-starts-with-digit").is_ok());
}
