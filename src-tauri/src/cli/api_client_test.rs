use std::io::{Read, Write};
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::adaptor::controller::api::{self, test_support as api_test_support};
use crate::adaptor::gateway::workflow::schema::{
    CommandSpec, NodeDefinition, NodeKind, WorkflowDefinitionYaml,
};
use crate::adaptor::gateway::workflow::storage;
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
    let (_workflow_usecase, runtime, gateway) = api_test_support::usecases(query_data.path());
    gateway.resolve_workflows_from(
        workflows.path().to_path_buf(),
        workflows.path().to_path_buf(),
    );
    let binding = match crate::infrastructure::local_api::LocalApiServerBinding::bind(
        client_data.path().to_path_buf(),
    ) {
        Ok(binding) => binding,
        Err(error)
            if error.to_string().contains("Operation not permitted")
                || error.to_string().contains("Permission denied") =>
        {
            eprintln!("skipping loopback test because bind is forbidden: {error}");
            return;
        }
        Err(error) => panic!("the local API must bind to loopback: {error}"),
    };
    let router = api::build_router(
        Arc::new(
            crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
                query_data.path().to_path_buf(),
                Some(workflows.path().to_path_buf()),
            )
            .unwrap(),
        ),
        runtime,
        binding.bearer_token(),
        binding.terminal_bearer_token(),
        None,
        None,
    );
    let server_runtime = tokio::runtime::Runtime::new().unwrap();
    let server = binding.start(router, server_runtime.handle());

    let status: serde_json::Value = serde_json::from_str(
        &workflow::cmd_status(client_data.path(), execution_id, true).unwrap(),
    )
    .unwrap();
    assert_eq!(status["id"], execution_id);

    gateway.bind_node_execution("00000000-0000-4000-8000-000000000456", execution_id);

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

    let commands = gateway.commands.lock().unwrap();
    assert_eq!(commands.outputs.len(), 1);
    assert_eq!(
        commands.outputs[0].node_execution_id,
        "00000000-0000-4000-8000-000000000456"
    );
    drop(commands);

    server.shutdown();
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
