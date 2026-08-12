#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_tui_fixture::{fixture_process_shell_command, FixtureLifecycleCommand, FixturePlan};
use releash_lib::agent_session_tui_acceptance::{
    AcceptanceAgentSessionLifecycle, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
};
use releash_lib::terminal_surface::{
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1, TerminalSurfaceWireAttachment,
};
use releash_lib::workflow_control_plane_acceptance::{
    AcceptanceNodeExecutionStatus, AcceptanceWorkflowExecutionStatus,
    WorkflowControlPlaneAcceptanceHost,
};

fn provider_name(provider: AcceptanceProvider) -> &'static str {
    match provider {
        AcceptanceProvider::Claude => "claude",
        AcceptanceProvider::Codex => "codex",
    }
}

fn install_fixture_executable(
    directory: &Path,
    name: &str,
    provider: AcceptanceProvider,
    input_lines: usize,
) -> PathBuf {
    let executable = directory.join(name);
    let command = fixture_process_shell_command(&FixturePlan {
        input_lines,
        alternate_screen: true,
        emit_input_completion_marker: true,
        lifecycle_command: Some(FixtureLifecycleCommand {
            executable: env!("CARGO_BIN_EXE_releash").to_string(),
            arguments: vec![
                "hook".to_string(),
                "receive".to_string(),
                "--provider".to_string(),
                provider_name(provider).to_string(),
            ],
            environment: vec![],
        }),
        ..FixturePlan::new(name, vec![])
    });
    let initial_instruction_argument = match provider {
        AcceptanceProvider::Claude => "if [ \"$#\" -eq 3 ]; then initial_instruction=$3; fi",
        AcceptanceProvider::Codex => "if [ \"$#\" -eq 5 ]; then initial_instruction=$5; fi",
    };
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\ninitial_instruction=\n{initial_instruction_argument}\nif [ -n \"$initial_instruction\" ]; then\n  {{ printf '\\033[200~%s\\033[201~\\n' \"$initial_instruction\"; cat; }} | {command}\nelse\n  {command}\nfi\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
    }
    executable
}

fn host(
    root: &Path,
    input_lines: usize,
) -> WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime> {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = install_fixture_executable(
        &bin,
        "claude-workflow-fixture",
        AcceptanceProvider::Claude,
        input_lines,
    );
    let codex = install_fixture_executable(
        &bin,
        "codex-workflow-fixture",
        AcceptanceProvider::Codex,
        input_lines,
    );
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    WorkflowControlPlaneAcceptanceHost::start(
        AgentSessionTuiAcceptanceConfig {
            data_dir: root.join("releash-data"),
            claude_executable: Some(claude),
            codex_executable: Some(codex),
            provider_search_path: None,
            provider_refresh_search_path: None,
            claude_config_dir: root.join("claude-home"),
            codex_home: root.join("codex-home"),
        },
        app,
    )
    .unwrap()
}

fn owner(worktree_path: &str, session_id: &str) -> TerminalSurfaceOwnerV1 {
    TerminalSurfaceOwnerV1::Session {
        workspace_path: worktree_path.to_string(),
        session_id: session_id.to_string(),
    }
}

async fn receive_until(attachment: &mut TerminalSurfaceWireAttachment, needle: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = String::new();
        while !output.contains(needle) {
            match attachment.next().await.expect("Terminal Surface stream") {
                TerminalSurfaceStreamItemV1::Snapshot { surface } => {
                    output.push_str(&surface.terminal_surface.replay)
                }
                TerminalSurfaceStreamItemV1::Output { data, .. } => output.push_str(&data),
                _ => {}
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"));
}

async fn send_hook(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    terminal: &mut TerminalSurfaceWireAttachment,
    owner: &TerminalSurfaceOwnerV1,
    payload: serde_json::Value,
) {
    host.terminal()
        .write(
            owner.clone(),
            &format!("releash-fixture-hook-json:{payload}\r"),
        )
        .unwrap();
    receive_until(terminal, "releash-fixture-lifecycle-command-result:").await;
}

async fn wait_for_execution_status(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    execution_id: &str,
    status: AcceptanceWorkflowExecutionStatus,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if host
                .execution(execution_id)
                .await
                .unwrap()
                .is_some_and(|execution| execution.status == status)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Workflow execution must reach the expected status");
}

async fn associate_provider_session(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    terminal: &mut TerminalSurfaceWireAttachment,
    owner: &TerminalSurfaceOwnerV1,
    provider_session_id: &str,
) {
    send_hook(
        host,
        terminal,
        owner,
        serde_json::json!({
            "session_id": provider_session_id,
            "transcript_path": format!("provider://fixture/{provider_session_id}"),
            "hook_event_name": "SessionStart"
        }),
    )
    .await;
}

async fn emit_provider_stop(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    terminal: &mut TerminalSurfaceWireAttachment,
    owner: &TerminalSurfaceOwnerV1,
    provider_session_id: &str,
) {
    send_hook(
        host,
        terminal,
        owner,
        serde_json::json!({
            "session_id": provider_session_id,
            "transcript_path": format!("provider://fixture/{provider_session_id}"),
            "hook_event_name": "Stop"
        }),
    )
    .await;
}

async fn wait_for_node_count(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    execution_id: &str,
    count: usize,
) -> releash_lib::workflow_control_plane_acceptance::AcceptanceWorkflowExecution {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let execution = host.execution(execution_id).await.unwrap().unwrap();
            if execution.node_executions.len() == count {
                return execution;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Workflow execution must project the expected Node count")
}

#[derive(Clone, Copy)]
enum SignalOrder {
    SubmitThenStop,
    StopThenSubmit,
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_040_同時submit_stopは競合を利用者へ返さず一度だけ収束する() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("concurrent-signal-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_auto_chain_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let initial = wait_for_node_count(&host, &execution_id, 1).await;
    let first = initial.node_executions[0].clone();
    let session_id = first.agent_session_id.clone().unwrap();
    let terminal_owner = owner(&worktree, &session_id);
    let mut terminal = host
        .terminal()
        .attach("atui-040-concurrent".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(&host, &mut terminal, &terminal_owner, "provider-concurrent").await;

    let (submit, ()) = tokio::join!(
        host.submit(&first.id),
        emit_provider_stop(&host, &mut terminal, &terminal_owner, "provider-concurrent",),
    );
    submit.unwrap();

    let advanced = wait_for_node_count(&host, &execution_id, 2).await;
    assert_eq!(
        advanced.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        advanced.node_executions[1].status,
        AcceptanceNodeExecutionStatus::Running
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_040_autoは両signal順序と重複に依存せず後続を一度だけ起動する() {
    for (index, order) in [SignalOrder::SubmitThenStop, SignalOrder::StopThenSubmit]
        .into_iter()
        .enumerate()
    {
        let root = tempfile::TempDir::new().unwrap();
        let worktree = root.path().join(format!("worktree-{index}"));
        std::fs::create_dir_all(&worktree).unwrap();
        let worktree = worktree.to_string_lossy().into_owned();
        let host = host(root.path(), 8);
        let execution_id = host
            .start_auto_chain_workflow(&worktree, AcceptanceProvider::Claude)
            .await
            .unwrap();
        let initial = wait_for_node_count(&host, &execution_id, 1).await;
        let first = initial.node_executions[0].clone();
        let first_session_id = first.agent_session_id.clone().unwrap();
        let first_owner = owner(&worktree, &first_session_id);
        let mut first_terminal = host
            .terminal()
            .attach(format!("atui-040-first-{index}"), first_owner.clone())
            .unwrap();
        receive_until(&mut first_terminal, "releash-fixture-input-complete-0").await;
        associate_provider_session(
            &host,
            &mut first_terminal,
            &first_owner,
            &format!("provider-auto-{index}"),
        )
        .await;

        match order {
            SignalOrder::SubmitThenStop => {
                host.submit(&first.id).await.unwrap();
                host.submit(&first.id).await.unwrap();
            }
            SignalOrder::StopThenSubmit => {
                emit_provider_stop(
                    &host,
                    &mut first_terminal,
                    &first_owner,
                    &format!("provider-auto-{index}"),
                )
                .await;
                emit_provider_stop(
                    &host,
                    &mut first_terminal,
                    &first_owner,
                    &format!("provider-auto-{index}"),
                )
                .await;
            }
        }

        let partial = host.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(partial.node_executions.len(), 1);
        assert_eq!(
            partial.node_executions[0].status,
            AcceptanceNodeExecutionStatus::Running
        );
        assert_ne!(
            partial.node_executions[0].submit_received,
            partial.node_executions[0].stop_received
        );

        match order {
            SignalOrder::SubmitThenStop => {
                emit_provider_stop(
                    &host,
                    &mut first_terminal,
                    &first_owner,
                    &format!("provider-auto-{index}"),
                )
                .await;
            }
            SignalOrder::StopThenSubmit => {
                host.submit(&first.id).await.unwrap();
            }
        }

        let advanced = wait_for_node_count(&host, &execution_id, 2).await;
        assert_eq!(
            advanced.node_executions[0].status,
            AcceptanceNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            advanced.node_executions[1].status,
            AcceptanceNodeExecutionStatus::Running
        );
        assert_ne!(
            advanced.node_executions[0].agent_session_id,
            advanced.node_executions[1].agent_session_id
        );

        host.submit(&first.id).await.unwrap();
        emit_provider_stop(
            &host,
            &mut first_terminal,
            &first_owner,
            &format!("provider-auto-{index}"),
        )
        .await;
        let after_duplicates = host.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(after_duplicates.node_executions.len(), 2);
        assert_eq!(
            after_duplicates.node_executions[1].id,
            advanced.node_executions[1].id
        );
        host.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_041_approvalは両signal成立後だけ対象nodeを承認できる() {
    for (index, order) in [SignalOrder::SubmitThenStop, SignalOrder::StopThenSubmit]
        .into_iter()
        .enumerate()
    {
        let root = tempfile::TempDir::new().unwrap();
        let worktree = root.path().join(format!("approval-worktree-{index}"));
        std::fs::create_dir_all(&worktree).unwrap();
        let worktree = worktree.to_string_lossy().into_owned();
        let host = host(root.path(), 8);
        let execution_id = host
            .start_approval_workflow(&worktree, AcceptanceProvider::Claude)
            .await
            .unwrap();
        let initial = wait_for_node_count(&host, &execution_id, 1).await;
        let node = initial.node_executions[0].clone();
        let session_id = node.agent_session_id.clone().unwrap();
        let terminal_owner = owner(&worktree, &session_id);
        let mut terminal = host
            .terminal()
            .attach(format!("atui-041-{index}"), terminal_owner.clone())
            .unwrap();
        receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
        associate_provider_session(
            &host,
            &mut terminal,
            &terminal_owner,
            &format!("provider-approval-{index}"),
        )
        .await;

        match order {
            SignalOrder::SubmitThenStop => {
                host.submit(&node.id).await.unwrap();
            }
            SignalOrder::StopThenSubmit => {
                emit_provider_stop(
                    &host,
                    &mut terminal,
                    &terminal_owner,
                    &format!("provider-approval-{index}"),
                )
                .await;
            }
        }
        let partial = host.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(
            partial.node_executions[0].status,
            AcceptanceNodeExecutionStatus::Running
        );
        assert!(!partial.node_executions[0].can_approve);

        match order {
            SignalOrder::SubmitThenStop => {
                emit_provider_stop(
                    &host,
                    &mut terminal,
                    &terminal_owner,
                    &format!("provider-approval-{index}"),
                )
                .await;
            }
            SignalOrder::StopThenSubmit => {
                host.submit(&node.id).await.unwrap();
            }
        }
        let waiting = host.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(
            waiting.node_executions[0].status,
            AcceptanceNodeExecutionStatus::WaitingApproval
        );
        assert!(waiting.node_executions[0].can_approve);
        assert!(host
            .approve(&execution_id, "other-node", &node.id)
            .await
            .is_err());
        host.approve(&execution_id, &node.node_name, &node.id)
            .await
            .unwrap();
        wait_for_execution_status(
            &host,
            &execution_id,
            AcceptanceWorkflowExecutionStatus::Completed,
        )
        .await;
        host.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_041_fanout兄弟は独立し全子成功時だけ一度完了する() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("fanout-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_approval_fanout_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let initial = wait_for_node_count(&host, &execution_id, 3).await;
    let children = initial
        .node_executions
        .iter()
        .filter(|node| node.agent_session_id.is_some())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    let first = children[0].clone();
    let second = children[1].clone();
    let first_owner = owner(&worktree, first.agent_session_id.as_deref().unwrap());
    let second_owner = owner(&worktree, second.agent_session_id.as_deref().unwrap());
    let mut first_terminal = host
        .terminal()
        .attach("atui-041-fanout-first".to_string(), first_owner.clone())
        .unwrap();
    let mut second_terminal = host
        .terminal()
        .attach("atui-041-fanout-second".to_string(), second_owner.clone())
        .unwrap();
    receive_until(&mut first_terminal, "releash-fixture-input-complete-0").await;
    receive_until(&mut second_terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut first_terminal,
        &first_owner,
        "provider-fanout-first",
    )
    .await;
    associate_provider_session(
        &host,
        &mut second_terminal,
        &second_owner,
        "provider-fanout-second",
    )
    .await;

    host.submit(&first.id).await.unwrap();
    emit_provider_stop(
        &host,
        &mut first_terminal,
        &first_owner,
        "provider-fanout-first",
    )
    .await;
    let first_waiting = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        first_waiting
            .node_executions
            .iter()
            .find(|node| node.id == first.id)
            .unwrap()
            .status,
        AcceptanceNodeExecutionStatus::WaitingApproval,
    );
    assert_eq!(
        first_waiting
            .node_executions
            .iter()
            .find(|node| node.id == second.id)
            .unwrap()
            .status,
        AcceptanceNodeExecutionStatus::Running,
    );
    host.approve(&execution_id, &first.node_name, &first.id)
        .await
        .unwrap();
    let one_succeeded = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        one_succeeded
            .node_executions
            .iter()
            .find(|node| node.id == second.id)
            .unwrap()
            .status,
        AcceptanceNodeExecutionStatus::Running,
    );
    assert_ne!(
        one_succeeded.status,
        AcceptanceWorkflowExecutionStatus::Completed
    );

    host.submit(&second.id).await.unwrap();
    emit_provider_stop(
        &host,
        &mut second_terminal,
        &second_owner,
        "provider-fanout-second",
    )
    .await;
    host.approve(&execution_id, &second.node_name, &second.id)
        .await
        .unwrap();
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;
    let completed = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        completed
            .node_executions
            .iter()
            .filter(|node| node.status == AcceptanceNodeExecutionStatus::Succeeded)
            .count(),
        3,
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_042_片側signalは再起動後も同じattemptへ復元される() {
    for (index, signal) in [SignalOrder::SubmitThenStop, SignalOrder::StopThenSubmit]
        .into_iter()
        .enumerate()
    {
        let root = tempfile::TempDir::new().unwrap();
        let worktree = root.path().join(format!("restart-worktree-{index}"));
        std::fs::create_dir_all(&worktree).unwrap();
        let worktree = worktree.to_string_lossy().into_owned();
        let host_before = host(root.path(), 5);
        let execution_id = host_before
            .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
            .await
            .unwrap();
        let initial = wait_for_node_count(&host_before, &execution_id, 1).await;
        let node = initial.node_executions[0].clone();
        let terminal_owner = owner(&worktree, node.agent_session_id.as_deref().unwrap());
        let mut terminal = host_before
            .terminal()
            .attach(format!("atui-042-restart-{index}"), terminal_owner.clone())
            .unwrap();
        receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
        associate_provider_session(
            &host_before,
            &mut terminal,
            &terminal_owner,
            &format!("provider-restart-{index}"),
        )
        .await;
        match signal {
            SignalOrder::SubmitThenStop => host_before.submit(&node.id).await.unwrap(),
            SignalOrder::StopThenSubmit => {
                emit_provider_stop(
                    &host_before,
                    &mut terminal,
                    &terminal_owner,
                    &format!("provider-restart-{index}"),
                )
                .await;
            }
        }
        let before = host_before.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(before.node_executions[0].id, node.id);
        assert_ne!(
            before.node_executions[0].submit_received,
            before.node_executions[0].stop_received
        );
        host_before.shutdown().await.unwrap();

        let host_after = host(root.path(), 5);
        host_after.recover_startup().await.unwrap();
        let recovered = host_after.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(recovered.node_executions[0].id, node.id);
        assert_eq!(
            recovered.node_executions[0].submit_received,
            before.node_executions[0].submit_received
        );
        assert_eq!(
            recovered.node_executions[0].stop_received,
            before.node_executions[0].stop_received
        );
        assert_eq!(
            recovered.node_executions[0].status,
            match signal {
                SignalOrder::SubmitThenStop => AcceptanceNodeExecutionStatus::Paused,
                SignalOrder::StopThenSubmit => AcceptanceNodeExecutionStatus::Running,
            }
        );
        host_after.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_042_retryは新attemptと新sessionを作り旧stopを混入させない() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("retry-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let initial = wait_for_node_count(&host, &execution_id, 1).await;
    let old_attempt = initial.node_executions[0].clone();
    let old_session_id = old_attempt.agent_session_id.clone().unwrap();
    let old_owner = owner(&worktree, &old_session_id);
    let mut old_terminal = host
        .terminal()
        .attach("atui-042-retry-old".to_string(), old_owner.clone())
        .unwrap();
    receive_until(&mut old_terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(&host, &mut old_terminal, &old_owner, "provider-retry-old").await;
    host.submit(&old_attempt.id).await.unwrap();
    let retryable = host.execution(&execution_id).await.unwrap().unwrap();
    assert!(retryable.node_executions[0].can_retry);

    host.retry(&execution_id, &old_attempt.id).await.unwrap();
    let retried = wait_for_node_count(&host, &execution_id, 2).await;
    let old_history = retried
        .node_executions
        .iter()
        .find(|node| node.id == old_attempt.id)
        .unwrap();
    let new_attempt = retried
        .node_executions
        .iter()
        .find(|node| node.id != old_attempt.id)
        .unwrap()
        .clone();
    assert_eq!(old_history.attempt, 1);
    assert_eq!(old_history.status, AcceptanceNodeExecutionStatus::Aborted);
    assert!(old_history.submit_received);
    assert!(!old_history.stop_received);
    assert_eq!(new_attempt.attempt, 2);
    assert_eq!(new_attempt.status, AcceptanceNodeExecutionStatus::Running);
    assert!(!new_attempt.submit_received);
    assert!(!new_attempt.stop_received);
    assert_ne!(new_attempt.agent_session_id, old_attempt.agent_session_id);

    emit_provider_stop(&host, &mut old_terminal, &old_owner, "provider-retry-old").await;
    let after_old_stop = host.execution(&execution_id).await.unwrap().unwrap();
    let current = after_old_stop
        .node_executions
        .iter()
        .find(|node| node.id == new_attempt.id)
        .unwrap();
    assert!(!current.submit_received);
    assert!(!current.stop_received);

    let new_owner = owner(&worktree, new_attempt.agent_session_id.as_deref().unwrap());
    let mut new_terminal = host
        .terminal()
        .attach("atui-042-retry-new".to_string(), new_owner.clone())
        .unwrap();
    receive_until(&mut new_terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(&host, &mut new_terminal, &new_owner, "provider-retry-new").await;
    host.submit(&new_attempt.id).await.unwrap();
    emit_provider_stop(&host, &mut new_terminal, &new_owner, "provider-retry-new").await;
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_042_別bindingのstopを拒否しinvalid_artifactはsubmitごと拒否する() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("artifact-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_artifact_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let initial = wait_for_node_count(&host, &execution_id, 1).await;
    let node = initial.node_executions[0].clone();
    let terminal_owner = owner(&worktree, node.agent_session_id.as_deref().unwrap());
    let mut terminal = host
        .terminal()
        .attach("atui-042-artifact".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-artifact-current",
    )
    .await;

    emit_provider_stop(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-artifact-stale",
    )
    .await;
    let after_stale_stop = host.execution(&execution_id).await.unwrap().unwrap();
    assert!(!after_stale_stop.node_executions[0].stop_received);

    assert!(host
        .submit_artifact(
            &node.id,
            "acceptance-result",
            serde_json::json!({"unexpected": "value"}),
        )
        .await
        .is_err());
    let after_invalid = host.execution(&execution_id).await.unwrap().unwrap();
    assert!(!after_invalid.node_executions[0].submit_received);
    assert!(!after_invalid.node_executions[0].has_artifact);

    host.submit_artifact(
        &node.id,
        "acceptance-result",
        serde_json::json!({"result": "ok"}),
    )
    .await
    .unwrap();
    let after_valid = host.execution(&execution_id).await.unwrap().unwrap();
    assert!(after_valid.node_executions[0].submit_received);
    assert!(after_valid.node_executions[0].has_artifact);
    emit_provider_stop(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-artifact-current",
    )
    .await;
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1626_active_attemptへの再submitはartifactを差し替える() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("resubmit-artifact-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_approval_artifact_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let initial = wait_for_node_count(&host, &execution_id, 1).await;
    let node = initial.node_executions[0].clone();
    let terminal_owner = owner(&worktree, node.agent_session_id.as_deref().unwrap());
    let mut terminal = host
        .terminal()
        .attach("issue-1626-resubmit".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(&host, &mut terminal, &terminal_owner, "provider-resubmit").await;

    host.submit_artifact(
        &node.id,
        "acceptance-result",
        serde_json::json!({"result": "first"}),
    )
    .await
    .unwrap();
    host.submit_artifact(
        &node.id,
        "acceptance-result",
        serde_json::json!({"result": "running replacement"}),
    )
    .await
    .unwrap();

    let after_running_replacement = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        after_running_replacement.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Running
    );
    assert_eq!(
        after_running_replacement.node_executions[0]
            .artifact
            .as_ref(),
        Some(&serde_json::json!({"result": "running replacement"}))
    );
    let after_running_log = host.workflow_log(&execution_id).await.unwrap();
    assert_eq!(
        after_running_log
            .iter()
            .filter(|event| event["event"] == "node_submit_received")
            .count(),
        1
    );
    assert_eq!(
        after_running_log
            .iter()
            .filter(|event| event["event"] == "artifact_produced")
            .count(),
        2
    );

    assert!(host
        .submit_artifact(
            &node.id,
            "acceptance-result",
            serde_json::json!({"unexpected": "invalid replacement"}),
        )
        .await
        .is_err());
    let after_invalid = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        after_invalid.node_executions[0].artifact.as_ref(),
        Some(&serde_json::json!({"result": "running replacement"}))
    );
    assert_eq!(
        host.workflow_log(&execution_id)
            .await
            .unwrap()
            .iter()
            .filter(|event| event["event"] == "artifact_produced")
            .count(),
        2
    );

    emit_provider_stop(&host, &mut terminal, &terminal_owner, "provider-resubmit").await;
    let waiting = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        waiting.node_executions[0].status,
        AcceptanceNodeExecutionStatus::WaitingApproval
    );

    host.submit_artifact(
        &node.id,
        "acceptance-result",
        serde_json::json!({"result": "waiting replacement"}),
    )
    .await
    .unwrap();
    let after_waiting_replacement = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        after_waiting_replacement.node_executions[0].status,
        AcceptanceNodeExecutionStatus::WaitingApproval
    );
    assert_eq!(
        after_waiting_replacement.node_executions[0]
            .artifact
            .as_ref(),
        Some(&serde_json::json!({"result": "waiting replacement"}))
    );

    host.approve(&execution_id, &node.node_name, &node.id)
        .await
        .unwrap();
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;
    let completed = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        completed.node_executions[0].artifact.as_ref(),
        Some(&serde_json::json!({"result": "waiting replacement"}))
    );
    assert_eq!(
        host.workflow_log(&execution_id)
            .await
            .unwrap()
            .iter()
            .filter(|event| event["event"] == "artifact_produced")
            .count(),
        3
    );

    host.submit_artifact(
        &node.id,
        "acceptance-result",
        serde_json::json!({"result": "terminal replacement"}),
    )
    .await
    .unwrap();
    let after_terminal_submit = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        after_terminal_submit.node_executions[0].artifact.as_ref(),
        Some(&serde_json::json!({"result": "waiting replacement"}))
    );
    assert_eq!(
        host.workflow_log(&execution_id)
            .await
            .unwrap()
            .iter()
            .filter(|event| event["event"] == "artifact_produced")
            .count(),
        3
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_042_workflow完了後もagent_sessionとptyを維持し追加stopで再進行しない() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 6);

    let execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let running = host.execution(&execution_id).await.unwrap().unwrap();
    let node = running
        .node_executions
        .iter()
        .find(|node| node.status == AcceptanceNodeExecutionStatus::Running)
        .unwrap_or_else(|| panic!("running Agent Node was not projected: {running:?}"))
        .clone();
    let session_id = node.agent_session_id.clone().unwrap();
    let terminal_owner = owner(&worktree, &session_id);
    let mut terminal = host
        .terminal()
        .attach(
            "workflow-completion-retention".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;

    send_hook(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "provider-session-1",
            "transcript_path": "provider://fixture/provider-session-1",
            "hook_event_name": "SessionStart"
        }),
    )
    .await;
    host.submit(&node.id).await.unwrap();
    send_hook(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "provider-session-1",
            "transcript_path": "provider://fixture/provider-session-1",
            "hook_event_name": "Stop"
        }),
    )
    .await;
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;

    let sequence_after_completion = host
        .terminal()
        .get(terminal_owner.clone())
        .unwrap()
        .terminal_surface
        .sequence;
    tokio::task::yield_now().await;
    assert_eq!(
        host.terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .terminal_surface
            .sequence,
        sequence_after_completion,
        "Workflow completion must not inject another Provider input",
    );
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open),
    );
    assert!(
        !host
            .terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .is_exited
    );

    host.terminal()
        .write(terminal_owner.clone(), "follow-up-after-completion\r")
        .unwrap();
    receive_until(&mut terminal, "follow-up-after-completion").await;
    send_hook(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "provider-session-1",
            "transcript_path": "provider://fixture/provider-session-1",
            "hook_event_name": "Stop"
        }),
    )
    .await;

    let completed = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        completed.status,
        AcceptanceWorkflowExecutionStatus::Completed
    );
    assert_eq!(completed.node_executions.len(), 1);
    assert_eq!(
        completed.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Succeeded
    );
    assert!(!host.terminal().get(terminal_owner).unwrap().is_exited);
    host.shutdown().await.unwrap();
}
