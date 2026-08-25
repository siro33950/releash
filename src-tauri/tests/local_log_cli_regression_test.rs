use std::process::Command;

const RELEASH_CLI_PATH: &str = env!("CARGO_BIN_EXE_releash");

#[test]
fn test_review_cli_存在しないdata_dirをlogger初期化で作らずnot_foundを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let missing_data_dir = directory.path().join("missing");

    // When
    let output = Command::new(RELEASH_CLI_PATH)
        .args(["review", "list", "--session-id", "abc"])
        .env("RELEASH_DATA_DIR", &missing_data_dir)
        .output()
        .unwrap();

    // Then
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        format!(
            "data directory does not exist: {}",
            missing_data_dir.display()
        )
    );
    assert!(!missing_data_dir.exists());
}
