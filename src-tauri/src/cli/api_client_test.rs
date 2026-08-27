use std::io::{Read, Write};

use tempfile::TempDir;

use super::*;
use crate::adaptor::controller::api::test_support as api_test_support;
use crate::adaptor::gateway::workflow::schema::{
    CommandSpec, NodeDefinition, NodeKind, WorkflowDefinitionYaml,
};
use crate::adaptor::gateway::workflow::storage;
use crate::cli::common::{cli_error_exit_code, cli_error_stderr};
use crate::cli::test_helpers::try_start_local_api_test_host;
use crate::cli::{output, workflow};
use crate::infrastructure::local_api::{local_api_discovery_path, process_start_time};

fn write_live_discovery(data_dir: &Path, token: &str) -> std::thread::JoinHandle<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let pid = std::process::id();
    std::fs::write(
        local_api_discovery_path(data_dir),
        serde_json::json!({
            "port": port,
            "token": token,
            "instance_id": "test-instance",
            "pid": pid,
            "process_started_at": process_start_time(pid).unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
    })
}

fn write_diagnostics_server(data_dir: &Path, expected_target: &str) -> std::thread::JoinHandle<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let pid = std::process::id();
    std::fs::write(
        local_api_discovery_path(data_dir),
        serde_json::json!({
            "port": port,
            "token": "secret",
            "instance_id": "test-instance",
            "pid": pid,
            "process_started_at": process_start_time(pid).unwrap(),
        })
        .to_string(),
    )
    .unwrap();
    let expected_target = expected_target.to_string();
    std::thread::spawn(move || {
        let (mut identity_stream, _) = listener.accept().unwrap();
        let mut identity_request = [0_u8; 4096];
        let read = identity_stream.read(&mut identity_request).unwrap();
        let identity_request = String::from_utf8_lossy(&identity_request[..read]);
        assert!(identity_request
            .starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
        identity_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();

        let (mut diagnostics_stream, _) = listener.accept().unwrap();
        let mut diagnostics_request = [0_u8; 4096];
        let read = diagnostics_stream.read(&mut diagnostics_request).unwrap();
        let diagnostics_request = String::from_utf8_lossy(&diagnostics_request[..read]);
        assert!(
            diagnostics_request.starts_with(&format!("GET {expected_target} HTTP/1.1")),
            "unexpected diagnostics request: {diagnostics_request}"
        );
        assert!(diagnostics_request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret"));
        diagnostics_stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
    })
}

fn command_workflow(name: &str) -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: name.to_string(),
        description: "live local API boundary fixture".to_string(),
        nodes: vec![NodeDefinition {
            name: "main".to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: "true".to_string(),
                env: Default::default(),
            }),
            ..NodeDefinition::default()
        }],
        ..WorkflowDefinitionYaml::default()
    }
}

#[test]
fn test_保持対象cli_discoveryとlive_httpを通る() {
    let client_data = TempDir::new().unwrap();
    let query_data = TempDir::new().unwrap();
    let workflows = TempDir::new().unwrap();
    let execution_id = "00000000-0000-4000-8000-000000000321";

    storage::save_workflow(workflows.path(), &command_workflow("review")).unwrap();
    api_test_support::seed_query_execution(query_data.path(), execution_id);
    api_test_support::seed_submitted_output(query_data.path(), execution_id);
    let Some(host) =
        try_start_local_api_test_host(client_data.path(), query_data.path(), workflows.path())
    else {
        return;
    };
    host.gateway.resolve_workflows_from(
        workflows.path().to_path_buf(),
        workflows.path().to_path_buf(),
    );

    let status: serde_json::Value = serde_json::from_str(
        &workflow::cmd_status(client_data.path(), execution_id, true).unwrap(),
    )
    .unwrap();
    assert_eq!(status["id"], execution_id);

    host.gateway
        .bind_node_execution("00000000-0000-4000-8000-000000000456", execution_id);

    output::cmd_output_submit(
        client_data.path(),
        "00000000-0000-4000-8000-000000000456".to_string(),
        Some("review-result"),
        Some(r#"{"status":"approved"}"#.to_string()),
        None,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_str(
        &output::cmd_output_get(client_data.path(), execution_id, "review", true).unwrap(),
    )
    .unwrap();
    assert_eq!(output["status"], "submitted");
    assert_eq!(output["contract"], "review-result");

    let commands = host.gateway.commands.lock().unwrap();
    assert_eq!(commands.outputs.len(), 1);
    assert_eq!(
        commands.outputs[0].node_execution_id,
        "00000000-0000-4000-8000-000000000456"
    );
    drop(commands);

    drop(host);
    assert!(!local_api_discovery_path(client_data.path()).exists());
}

#[test]
fn test_local_api読取_discovery欠落時にfallbackする() {
    let temp = TempDir::new().unwrap();
    let value = read_with_fallback(temp.path(), |_| unreachable!(), || Ok(42)).unwrap();
    assert_eq!(value, 42);
}

#[test]
fn test_local_api更新_discovery欠落時に日本語で拒否する() {
    let temp = TempDir::new().unwrap();
    let result: Result<(), CliError> = mutation(temp.path(), |_| unreachable!());
    assert!(
        matches!(result, Err(CliError::Other(message)) if message.contains("アプリの起動が必要"))
    );
}

#[test]
fn test_local_api_fallback無し読取_discovery欠落時は更新と同じ失敗になる() {
    // Given
    let temp = TempDir::new().unwrap();

    // When
    let read: Result<(), CliError> = read_without_fallback(temp.path(), |_| unreachable!());
    let mutation: Result<(), CliError> = mutation(temp.path(), |_| unreachable!());

    // Then
    assert_eq!(read, mutation);
    assert_eq!(
        read,
        Err(CliError::Other(
            "この操作には Releash アプリの起動が必要です".to_string()
        ))
    );
}

#[test]
fn test_workflow_diagnosticsはdirectory有無をqueryへ写す() {
    // Given
    let cases = [
        (None, "/v1/workflow/diagnostics"),
        (Some("/tmp/x"), "/v1/workflow/diagnostics?dir=%2Ftmp%2Fx"),
    ];

    for (dir, expected_target) in cases {
        let temp = TempDir::new().unwrap();
        let server = write_diagnostics_server(temp.path(), expected_target);

        // When
        let value =
            read_without_fallback(temp.path(), |client| client.workflow_diagnostics(dir)).unwrap();

        // Then
        assert_eq!(value, serde_json::json!({}));
        server.join().unwrap();
    }
}

#[test]
fn test_local_api_status_errorのcli分類を維持する() {
    // Given
    let invalid_statuses = [400, 409, 422];

    // When
    let not_found = api_error(404, Some("missing"));
    let invalid = invalid_statuses.map(|status| api_error(status, Some("invalid")));
    let unauthorized = api_error(401, Some("unauthorized"));

    // Then
    assert!(matches!(not_found, CliError::NotFound(_)));
    assert!(invalid
        .into_iter()
        .all(|error| matches!(error, CliError::InvalidInput(_))));
    assert!(matches!(unauthorized, CliError::Other(_)));
}

#[test]
fn test_local_api読取_api利用不能時にfallbackする() {
    let temp = TempDir::new().unwrap();
    let identity_server = write_live_discovery(temp.path(), "secret");
    let value = read_with_fallback(
        temp.path(),
        |_| Err(ApiRequestError::Unavailable),
        || Ok("fallback"),
    )
    .unwrap();
    assert_eq!(value, "fallback");
    identity_server.join().unwrap();
}

#[test]
fn test_local_api読取_認証失敗時にfallbackしない() {
    let temp = TempDir::new().unwrap();
    let identity_server = write_live_discovery(temp.path(), "secret");
    let result: Result<(), CliError> = read_with_fallback(
        temp.path(),
        |_| Err(ApiRequestError::Cli(api_error(401, Some("bad token")))),
        || panic!("401 must not fall back"),
    );
    assert!(matches!(result, Err(CliError::Other(message)) if message.contains("認証に失敗")));
    identity_server.join().unwrap();
}

#[test]
fn test_local_api読取_不正discoveryでfallbackしない() {
    let temp = TempDir::new().unwrap();
    std::fs::write(local_api_discovery_path(temp.path()), "not-json").unwrap();
    let result: Result<(), CliError> = read_with_fallback(
        temp.path(),
        |_| unreachable!(),
        || panic!("malformed discovery must not fall back"),
    );
    assert!(
        matches!(result, Err(CliError::Other(message)) if message.contains("discovery file が不正"))
    );
}

#[test]
fn test_local_api_discovery_process情報参照不能をcatch_allで終了コード1にする() {
    // Given / When
    let error = discovery_error(LocalApiClientError::ProcessInformationUnavailable);

    // Then
    assert_eq!(
        error,
        CliError::Other(
            "プロセス情報を参照できないため、local API の接続先を確認できませんでした".to_string()
        )
    );
    assert_eq!(cli_error_exit_code(&error), 1);
    assert_eq!(
        cli_error_stderr(&error),
        "error: プロセス情報を参照できないため、local API の接続先を確認できませんでした"
    );
}
