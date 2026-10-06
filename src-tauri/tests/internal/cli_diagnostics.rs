use releash_lib::test_support::integration::platform::cmd_diagnostics;
use releash_lib::test_support::integration::platform::ensure_existing_target_dir;
use releash_lib::test_support::integration::platform::CliError;
use releash_lib::test_support::integration::platform::CliSuccess;
use std::path::Path;

use crate::cli_test_helpers::start_local_api_test_host as start_host;

const VALID_WORKFLOW: &str = r#"name: valid-fixture
description: valid diagnostics fixture
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: fixture-instruction
"#;
fn write_valid_fixture(directory: &Path) {
    std::fs::create_dir_all(directory.join("instructions")).unwrap();
    std::fs::write(directory.join("valid-fixture.yml"), VALID_WORKFLOW).unwrap();
    std::fs::write(
        directory
            .join("instructions")
            .join("fixture-instruction.md"),
        "# Fixture instruction\n",
    )
    .unwrap();
}

fn json_stdout(success: &CliSuccess) -> serde_json::Value {
    serde_json::from_str(&success.test_stdout()).unwrap_or_else(|error| {
        panic!(
            "diagnostics stdout must be JSON: {error}; stdout={}",
            success.test_stdout()
        )
    })
}

#[test]
pub fn test_診断_存在しない対象directoryをnot_foundにする() {
    // Given
    let existing = tempfile::tempdir().unwrap();
    let missing = existing.path().join("missing");

    // When
    let existing_result = ensure_existing_target_dir(existing.path());
    let missing_error = ensure_existing_target_dir(&missing).unwrap_err();

    // Then
    assert_eq!(existing_result, Ok(()));
    assert_eq!(
        missing_error,
        CliError::NotFound(format!("directory does not exist: {}", missing.display()))
    );
}

#[test]
pub fn test_診断_usecase経路は存在しない指定directoryをerrorにする() {
    // Given
    let client_data = tempfile::tempdir().unwrap();
    let query_data = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let missing = query_data.path().join("missing");
    let host = start_host(
        client_data.path(),
        query_data.path(),
        applied_directory.path(),
    );

    // When
    let error = host
        .workflow
        .diagnose_all(
            releash_lib::test_support::integration::workflow::WorkflowDiagnosticsTarget::from_optional_directory(
                Some(missing.to_string_lossy().into_owned()),
            )
            .unwrap(),
        )
        .unwrap_err();

    // Then
    assert_eq!(
        error.to_string(),
        format!("not_found: directory does not exist: {}", missing.display())
    );
}

#[test]
pub fn test_診断_cli経路は前後空白を含む実在directoryを同じpathでgatewayへ渡す() {
    // Given
    let client_data = tempfile::tempdir().unwrap();
    let query_data = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_parent = tempfile::tempdir().unwrap();
    let fixture_directory = fixture_parent.path().join(" workflows ");
    std::fs::create_dir(&fixture_directory).unwrap();
    write_valid_fixture(&fixture_directory);
    let _host = start_host(
        client_data.path(),
        query_data.path(),
        applied_directory.path(),
    );

    // When
    let success = cmd_diagnostics(client_data.path(), Some(fixture_directory), true).unwrap();
    let report = json_stdout(&success);

    // Then
    assert_eq!(success.test_exit_code(), 0);
    assert!(report["facet_usage"]["instruction/fixture-instruction"].is_array());
}

#[test]
pub fn test_診断_cli経路は通常fileの指定をcommand失敗にする() {
    // Given
    let client_data = tempfile::tempdir().unwrap();
    let query_data = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let target_parent = tempfile::tempdir().unwrap();
    let file = target_parent.path().join("workflow.yml");
    std::fs::write(&file, "name: workflow").unwrap();
    let _host = start_host(
        client_data.path(),
        query_data.path(),
        applied_directory.path(),
    );

    // When
    let error = cmd_diagnostics(client_data.path(), Some(file), true).unwrap_err();

    // Then
    assert_eq!(
        releash_lib::test_support::integration::platform::cli_result_exit_code(Err(error)),
        1
    );
}

#[cfg(unix)]
#[test]
pub fn test_診断_cli経路は列挙不能なdirectoryの指定をcommand失敗にする() {
    use std::os::unix::fs::PermissionsExt;

    // Given
    let client_data = tempfile::tempdir().unwrap();
    let query_data = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let target_parent = tempfile::tempdir().unwrap();
    let unreadable = target_parent.path().join("unreadable");
    std::fs::create_dir(&unreadable).unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&unreadable).is_ok() {
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        eprintln!("skipping unreadable directory test because this process can read mode 0o000");
        return;
    }
    let _host = start_host(
        client_data.path(),
        query_data.path(),
        applied_directory.path(),
    );

    // When
    let result = cmd_diagnostics(client_data.path(), Some(unreadable.clone()), true);
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let error = result.unwrap_err();

    // Then
    assert_eq!(
        releash_lib::test_support::integration::platform::cli_result_exit_code(Err(error)),
        1
    );
}
