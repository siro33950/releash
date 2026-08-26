use std::process::Command;

const RELEASH_CLI_PATH: &str = env!("CARGO_BIN_EXE_releash");

const VALID_WORKFLOW: &str = r#"name: valid-fixture
description: valid diagnostics fixture
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: fixture-instruction
"#;

const INVALID_WORKFLOW: &str = r#"name: invalid-fixture
description: invalid permission fixture
nodes:
  main:
    session:
      provider: claude
      permission: unknown
"#;

const BOOLEAN_SCALAR_SCHEMA_WORKFLOW: &str = r#"name: boolean-scalar-schema
description: boolean scalar schema fixture
schemas:
  payload:
    type: object
    properties:
      foo: boolean
nodes:
  main:
    command: printf ok
"#;

const NESTED_SCHEMA_REFERENCE_WORKFLOW: &str = r#"name: nested-schema-reference
description: nested schema reference fixture
schemas:
  payload:
    type: object
    properties:
      nested:
        type: object
        properties:
          child: result
  result:
    type: object
    properties:
      value: string
nodes:
  main:
    command: printf ok
"#;

const INVALID_NESTED_ARRAY_SCHEMA_WORKFLOW: &str = r#"name: invalid-nested-array-schema
description: invalid nested array schema fixture
schemas:
  payload:
    type: object
    properties:
      values:
        type: array
        items: bad.reference
nodes:
  main:
    command: printf ok
"#;

fn write_valid_fixture(directory: &std::path::Path) {
    let facet_directory = directory.join("facets").join("instructions");
    std::fs::create_dir_all(&facet_directory).unwrap();
    std::fs::write(directory.join("valid-fixture.yml"), VALID_WORKFLOW).unwrap();
    std::fs::write(
        facet_directory.join("fixture-instruction.md"),
        "# Fixture instruction\n",
    )
    .unwrap();
}

fn write_invalid_fixture(directory: &std::path::Path) {
    std::fs::write(directory.join("invalid-fixture.yml"), INVALID_WORKFLOW).unwrap();
}

fn write_scalar_schema_fixture(directory: &std::path::Path) {
    std::fs::write(
        directory.join("boolean-scalar-schema.yml"),
        BOOLEAN_SCALAR_SCHEMA_WORKFLOW,
    )
    .unwrap();
    std::fs::write(
        directory.join("nested-schema-reference.yml"),
        NESTED_SCHEMA_REFERENCE_WORKFLOW,
    )
    .unwrap();
}

fn write_invalid_nested_array_schema_fixture(directory: &std::path::Path) {
    std::fs::write(
        directory.join("invalid-nested-array-schema.yml"),
        INVALID_NESTED_ARRAY_SCHEMA_WORKFLOW,
    )
    .unwrap();
}

fn write_applied_fixture(directory: &std::path::Path) {
    std::fs::write(directory.join("applied-only.yml"), "name: [").unwrap();
}

fn run_diagnostics_cli(
    data_directory: &std::path::Path,
    target_directory: Option<&std::path::Path>,
    json: bool,
) -> std::process::Output {
    let mut command = Command::new(RELEASH_CLI_PATH);
    command.args(["workflow", "diagnostics"]);
    if let Some(target_directory) = target_directory {
        command.arg("--dir").arg(target_directory);
    }
    if json {
        command.arg("--json");
    }
    command
        .env("RELEASH_DATA_DIR", data_directory)
        .output()
        .unwrap()
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "diagnostics stdout must be JSON: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn test_診断cli_アプリ未起動時は既存commandと同じ失敗になる() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();

    // When
    let diagnostics = Command::new(RELEASH_CLI_PATH)
        .args(["workflow", "diagnostics", "--dir"])
        .arg(fixture_directory.path())
        .env("RELEASH_DATA_DIR", data_directory.path())
        .output()
        .unwrap();
    let existing_command = Command::new(RELEASH_CLI_PATH)
        .args([
            "workflow",
            "output",
            "submit",
            "--node-execution",
            "00000000-0000-4000-8000-000000000456",
        ])
        .env("RELEASH_DATA_DIR", data_directory.path())
        .output()
        .unwrap();

    // Then
    assert_eq!(diagnostics.status.code(), Some(1));
    assert!(diagnostics.stdout.is_empty());
    assert_eq!(diagnostics.status.code(), existing_command.status.code());
    assert_eq!(diagnostics.stderr, existing_command.stderr);
    assert_eq!(
        String::from_utf8(diagnostics.stderr).unwrap().trim(),
        "error: この操作には Releash アプリの起動が必要です"
    );
}

#[test]
fn test_診断cli_helpに対象directoryと終了コードと出力形式が載る() {
    // Given
    let arguments = ["workflow", "diagnostics", "--help"];

    // When
    let output = Command::new(RELEASH_CLI_PATH)
        .args(arguments)
        .output()
        .unwrap();

    // Then
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "--dir",
        "--json",
        "適用済み",
        "Facet base",
        "出力形式",
        "終了コード",
        "0  severity error",
        "3  severity error",
        "1  command 自体の失敗",
        "2  引数が不正",
        "4  対象 directory が存在しない",
    ] {
        assert!(stdout.contains(expected), "missing help text: {expected}");
    }
}

#[test]
fn test_診断cli_存在しないdirはnot_foundで終わる() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let fixture_parent = tempfile::tempdir().unwrap();
    let missing_directory = fixture_parent.path().join("missing");

    // When
    let output = Command::new(RELEASH_CLI_PATH)
        .args(["workflow", "diagnostics", "--dir"])
        .arg(&missing_directory)
        .env("RELEASH_DATA_DIR", data_directory.path())
        .output()
        .unwrap();

    // Then
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        format!("directory does not exist: {}", missing_directory.display())
    );
}

#[tokio::test]
async fn test_診断local_api_実http経由で指定directoryのreportを返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    std::fs::write(fixture_directory.path().join("custom.yml"), "name: [").unwrap();
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();
    // When
    let report = host.diagnose(Some(fixture_directory.path())).await.unwrap();

    // Then
    assert!(report["workflow_summaries"]["custom"].is_object());
    assert!(report["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["workflow_name"] == "custom"));
    assert!(report["facet_summaries"].is_object());
    assert!(report["facet_usage"].is_object());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_ui経路とcli経路が同じvalid_fixtureで同一reportを返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_valid_fixture(fixture_directory.path());
    write_applied_fixture(applied_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();
    // When
    let ui_report = host
        .diagnose_via_ui_entry(fixture_directory.path())
        .await
        .unwrap();
    let cli_output =
        run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let cli_report = json_stdout(&cli_output);

    // Then
    assert_eq!(cli_output.status.code(), Some(0));
    assert_eq!(cli_report, ui_report);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_ui経路とcli経路が同じinvalid_fixtureで同一reportを返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_invalid_fixture(fixture_directory.path());
    write_applied_fixture(applied_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let ui_report = host
        .diagnose_via_ui_entry(fixture_directory.path())
        .await
        .unwrap();
    let cli_output =
        run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let cli_report = json_stdout(&cli_output);

    // Then
    assert_eq!(cli_output.status.code(), Some(3));
    assert_eq!(cli_report, ui_report);
    let item = cli_report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "WFS002")
        .expect("WFS002 diagnostic");
    assert_eq!(item["severity"], "error");
    assert_eq!(item["stage"], "parse_shape");
    assert_eq!(item["field"], "permission");
    assert_eq!(item["span"]["start_line"], 7);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_ui経路とcli経路がscalar_schema宣言のwfs002で同一reportを返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_scalar_schema_fixture(fixture_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let ui_report = host
        .diagnose_via_ui_entry(fixture_directory.path())
        .await
        .unwrap();
    let cli_output =
        run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let cli_report = json_stdout(&cli_output);

    // Then
    assert_eq!(cli_output.status.code(), Some(3));
    assert_eq!(cli_report, ui_report);
    let items = cli_report["items"].as_array().unwrap();
    for (workflow_name, message_fragment) in [
        (
            "boolean-scalar-schema",
            "properties.foo: scalar schema supports only 'string', got 'boolean'",
        ),
        (
            "nested-schema-reference",
            "properties.nested: properties.child: scalar schema supports only 'string', got 'result'",
        ),
    ] {
        let item = items
            .iter()
            .find(|item| item["code"] == "WFS002" && item["workflow_name"] == workflow_name)
            .unwrap_or_else(|| panic!("{workflow_name} must produce WFS002"));
        assert_eq!(item["severity"], "error");
        assert_eq!(item["stage"], "parse_shape");
        let message = item["message"].as_str().unwrap();
        assert!(message.starts_with("workflow shape error: error:"));
        assert!(message.contains(message_fragment), "unexpected WFS002 item: {item:?}");
        assert!(item["span"]["start_line"].as_u64().is_some());
    }
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_ui経路とcli経路がnested_schema_validationのwfs002で同一reportを返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_invalid_nested_array_schema_fixture(fixture_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let ui_report = host
        .diagnose_via_ui_entry(fixture_directory.path())
        .await
        .unwrap();
    let cli_output =
        run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let cli_report = json_stdout(&cli_output);

    // Then
    assert_eq!(cli_output.status.code(), Some(3));
    assert_eq!(cli_report, ui_report);
    let item = cli_report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "WFS002")
        .expect("nested schema declaration must produce WFS002");
    assert_eq!(item["severity"], "error");
    assert_eq!(item["stage"], "parse_shape");
    assert!(item["message"]
        .as_str()
        .unwrap()
        .contains("must start with an ASCII alphanumeric"));
    assert_eq!(item["span"]["start_line"], 7);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_cli経路は指定directory配下のfacetを含めて適用前に診断する() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_valid_fixture(fixture_directory.path());
    write_applied_fixture(applied_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let output = run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let report = json_stdout(&output);

    // Then
    assert_eq!(output.status.code(), Some(0));
    assert!(!report["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["code"] == "FAC002" && item["facet_key"] == "fixture-instruction" }));
    assert!(!report["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["code"] == "FAC000" && item["facet_key"] == "fixture-instruction" }));
    assert!(report["facet_usage"]["instruction/fixture-instruction"].is_array());
    assert_eq!(
        report["facet_summaries"]["instruction/fixture-instruction"],
        serde_json::json!({ "error_count": 0, "info_count": 0 })
    );
    assert!(report["workflow_summaries"].get("applied-only").is_none());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_error検出時に終了コード3を返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_invalid_fixture(fixture_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let output = run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);

    // Then
    assert_eq!(output.status.code(), Some(3));
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_error無しなら終了コード0を返す() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_valid_fixture(fixture_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let output = run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), true);
    let report = json_stdout(&output);

    // Then
    assert_eq!(output.status.code(), Some(0));
    assert!(report["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["severity"] != "error"));
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_dir省略時は適用済みconfig_directoryを対象にする() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_valid_fixture(fixture_directory.path());
    write_applied_fixture(applied_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let output = run_diagnostics_cli(data_directory.path(), None, true);
    let report = json_stdout(&output);

    // Then
    assert_eq!(output.status.code(), Some(3));
    assert!(report["workflow_summaries"]["applied-only"].is_object());
    assert!(report["workflow_summaries"].get("valid-fixture").is_none());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_診断_human_readable出力に検出したdiagnosticが含まれる() {
    // Given
    let data_directory = tempfile::tempdir().unwrap();
    let applied_directory = tempfile::tempdir().unwrap();
    let fixture_directory = tempfile::tempdir().unwrap();
    write_invalid_fixture(fixture_directory.path());
    let host =
        releash_lib::workflow_diagnostics_acceptance::WorkflowDiagnosticsAcceptanceHost::start(
            data_directory.path().to_path_buf(),
            applied_directory.path().to_path_buf(),
        )
        .unwrap();

    // When
    let output = run_diagnostics_cli(data_directory.path(), Some(fixture_directory.path()), false);

    // Then
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).is_err());
    assert!(stdout.lines().any(|line| line.contains("WFS002")));
    assert!(stdout.lines().any(|line| line.starts_with("1 error, ")));
    host.shutdown().await.unwrap();
}
