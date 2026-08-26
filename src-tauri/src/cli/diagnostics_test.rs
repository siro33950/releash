use super::*;

use crate::adaptor::controller::command::workflow::diagnostics::diagnose_all_impl;
use crate::cli::test_helpers::start_local_api_test_host as start_host;

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
    serde_json::from_str(&success.stdout).unwrap_or_else(|error| {
        panic!(
            "diagnostics stdout must be JSON: {error}; stdout={}",
            success.stdout
        )
    })
}

fn report(value: serde_json::Value) -> DiagnosticReport {
    serde_json::from_value(value).unwrap()
}

#[test]
fn test_診断_相対pathだけをcwd基準の絶対pathへ解決する() {
    // Given
    let cwd = Path::new("/tmp/repository");

    // When
    let relative = absolutize(PathBuf::from("workflows"), cwd);
    let absolute = absolutize(PathBuf::from("/tmp/workflows"), cwd);

    // Then
    assert_eq!(relative, cwd.join("workflows"));
    assert_eq!(absolute, PathBuf::from("/tmp/workflows"),);
}

#[test]
fn test_診断_存在しない対象directoryをnot_foundにする() {
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

#[cfg(unix)]
#[test]
fn test_診断_utf8ではない対象directoryをinvalid_inputにする() {
    use std::os::unix::ffi::OsStringExt;

    // Given
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));

    // When
    let result = target_dir_str(&path);

    // Then
    assert!(matches!(result, Err(CliError::InvalidInput(_))));
}

#[test]
fn test_診断_error_itemがある場合だけ終了コード3にする() {
    // Given
    let error = report(serde_json::json!({
        "items": [{"code":"E1","severity":"error","stage":"parse_shape","message":"m"}],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));
    let info = report(serde_json::json!({
        "items": [{"code":"I1","severity":"info","stage":"resolve","message":"m"}],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));
    let empty = report(serde_json::json!({
        "items": [],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));

    // When
    let error_exit = diagnostics_exit_code(&error);
    let info_exit = diagnostics_exit_code(&info);
    let empty_exit = diagnostics_exit_code(&empty);

    // Then
    assert_eq!(error_exit, 3);
    assert_eq!(info_exit, 0);
    assert_eq!(empty_exit, 0);
}

#[test]
fn test_診断_human_readable出力がitem順と位置と対象を保持する() {
    // Given
    let report = report(serde_json::json!({
        "items": [
            {
                "code":"E1","severity":"error","stage":"parse_shape","message":"first",
                "span":{"source":"workflow.yml","start_line":2,"start_col":3,"end_line":2,"end_col":4},
                "workflow_name":"alpha","node_name":"main","field":"permission"
            },
            {"code":"I1","severity":"info","stage":"resolve","message":"second"},
            {"code":"I2","severity":"info","stage":"resolve","message":"third","workflow_name":"beta"},
            {
                "code":"I3","severity":"info","stage":"resolve","message":"fourth",
                "span":{"start_line":4,"start_col":5,"end_line":4,"end_col":6},
                "facet_kind":"policy","facet_key":"coding"
            }
        ],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));

    // When
    let output = format_human_readable(&report);

    // Then
    assert_eq!(output.lines().count(), 6);
    assert_eq!(
        output,
        "error E1 workflow.yml:2:3 [workflow=alpha, node=main, field=permission]: first\n\
info I1: second\n\
info I2 [workflow=beta]: third\n\
info I3 4:5 [facet=policy/coding]: fourth\n\
\n\
1 error, 3 info\n"
    );
}

#[test]
fn test_診断_json出力が受信valueを再構成しない() {
    // Given
    let value = serde_json::json!({
        "items": [],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {},
        "future_field": {"kept": true}
    });

    // When
    let output = format_json(&value).unwrap();

    // Then
    assert_eq!(
        output,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
    );
}

#[test]
fn test_診断_cli_sourceにdiagnostic_code_literalを持たない() {
    // Given
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("diagnostics.rs", include_str!("diagnostics.rs")),
        ("api_client.rs", include_str!("api_client.rs")),
        ("workflow.rs", include_str!("workflow.rs")),
        ("common.rs", include_str!("common.rs")),
    ];

    for (file, source) in sources {
        // When / Then
        for prefix in ["\"WF", "\"FAC"] {
            assert!(
                !source.contains(prefix),
                "diagnostic code prefix '{prefix}' must not appear in {file}"
            );
        }
    }
}

#[test]
fn test_診断_ui経路は存在しない指定directoryをerrorにする() {
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
        .server_runtime
        .block_on(diagnose_all_impl(
            &host.workflow,
            Some(missing.to_string_lossy().into_owned()),
        ))
        .unwrap_err();

    // Then
    assert_eq!(
        error,
        format!("not_found: directory does not exist: {}", missing.display())
    );
}

#[test]
fn test_診断_cli経路は前後空白を含む実在directoryを同じpathでgatewayへ渡す() {
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
    assert_eq!(success.exit_code, 0);
    assert!(report["facet_usage"]["instruction/fixture-instruction"].is_array());
}

#[test]
fn test_診断_cli経路は通常fileの指定をcommand失敗にする() {
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
    assert_eq!(common::cli_result_exit_code(Err(error)), 1);
}

#[cfg(unix)]
#[test]
fn test_診断_cli経路は列挙不能なdirectoryの指定をcommand失敗にする() {
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
    assert_eq!(common::cli_result_exit_code(Err(error)), 1);
}
