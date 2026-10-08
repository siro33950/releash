use super::*;
use std::path::Path;

#[test]
fn validate_path_rejects_empty() {
    let result = validate_path("", "ファイルパス");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "ファイルパスが指定されていません");
}

#[test]
fn validate_path_accepts_non_empty() {
    assert!(validate_path(Path::new("/some/path").to_str().unwrap(), "ファイルパス").is_ok());
}
