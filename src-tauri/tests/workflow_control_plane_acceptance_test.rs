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
    AcceptanceNodeExecution, AcceptanceNodeExecutionStatus, AcceptanceNodeKind,
    AcceptanceWorkflowExecution, AcceptanceWorkflowExecutionStatus, AcceptanceWorkspaceNodeStatus,
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
            "#!/bin/sh\ntrap '' INT\ninitial_instruction=\n{initial_instruction_argument}\nif [ -n \"$initial_instruction\" ]; then\n  {{ printf '\\033[200~%s\\033[201~\\n' \"$initial_instruction\"; cat; }} | {command}\nelse\n  {command}\nfi\n"
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
    configured_host(root, input_lines)
}

fn configured_host(
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
    let config = AgentSessionTuiAcceptanceConfig {
        data_dir: root.join("releash-data"),
        claude_executable: Some(claude),
        codex_executable: Some(codex),
        provider_search_path: None,
        provider_refresh_search_path: None,
        claude_config_dir: root.join("claude-home"),
        codex_home: root.join("codex-home"),
    };
    WorkflowControlPlaneAcceptanceHost::start(config, app).unwrap()
}

fn owner(worktree_path: &str, session_id: &str) -> TerminalSurfaceOwnerV1 {
    TerminalSurfaceOwnerV1::Session {
        workspace_path: worktree_path.to_string(),
        session_id: session_id.to_string(),
    }
}

async fn receive_until(attachment: &mut TerminalSurfaceWireAttachment, needle: &str) {
    receive_until_all(attachment, &[needle]).await;
}

/// 1 つの accumulator で全 needle を待つ。複数の期待出力が同じ Snapshot や
/// 同じ出力 batch に載って届いても取りこぼさない。
async fn receive_until_all(attachment: &mut TerminalSurfaceWireAttachment, needles: &[&str]) {
    let mut output = String::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        while !needles.iter().all(|needle| output.contains(needle)) {
            match attachment.next().await.expect("Terminal Surface stream") {
                TerminalSurfaceStreamItemV1::Snapshot { surface } => {
                    output.push_str(&surface.terminal_surface.replay)
                }
                TerminalSurfaceStreamItemV1::Output { data, .. } => output.push_str(&data),
                _ => {}
            }
        }
    })
    .await;
    if result.is_err() {
        panic!("timed out waiting for {needles:?}; received: {output:?}");
    }
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
    // node を終端させる Stop では、commit 後の停止 effect が結果行の出力より先に
    // PTY を閉じることがある。stream 終了は commit 完了の証拠なので同期完了として扱う。
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = String::new();
        while !output.contains("releash-fixture-lifecycle-command-result:") {
            match terminal.next().await {
                Some(TerminalSurfaceStreamItemV1::Snapshot { surface }) => {
                    output.push_str(&surface.terminal_surface.replay)
                }
                Some(TerminalSurfaceStreamItemV1::Output { data, .. }) => output.push_str(&data),
                Some(_) => {}
                None => return,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for the lifecycle command result"));
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

async fn wait_for_agent_session_lifecycle(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    agent_session_id: &str,
    lifecycle: AcceptanceAgentSessionLifecycle,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if host
                .agent_session_lifecycle(agent_session_id)
                .await
                .unwrap()
                == Some(lifecycle)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("AgentSession must reach the expected lifecycle");
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

async fn emit_provider_working(
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
            "hook_event_name": "UserPromptSubmit"
        }),
    )
    .await;
}

async fn wait_for_node_count(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    execution_id: &str,
    count: usize,
) -> releash_lib::workflow_control_plane_acceptance::AcceptanceWorkflowExecution {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let execution = host.execution(execution_id).await.unwrap().unwrap();
            if execution.node_executions.len() == count {
                return execution;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(execution) => execution,
        Err(_) => panic!(
            "Workflow execution must project the expected Node count ({count}): {:?}",
            host.execution(execution_id).await
        ),
    }
}

fn leaf_nodes(execution: &AcceptanceWorkflowExecution) -> Vec<AcceptanceNodeExecution> {
    execution
        .node_executions
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                AcceptanceNodeKind::Session | AcceptanceNodeKind::Command
            )
        })
        .cloned()
        .collect()
}

async fn wait_for_leaf_session_attachment(
    host: &WorkflowControlPlaneAcceptanceHost<tauri::test::MockRuntime>,
    execution_id: &str,
    leaf_index: usize,
) -> String {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let execution = host.execution(execution_id).await.unwrap().unwrap();
            if let Some(agent_session_id) = leaf_nodes(&execution)
                .get(leaf_index)
                .and_then(|leaf| leaf.agent_session_id.clone())
            {
                return agent_session_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(agent_session_id) => agent_session_id,
        Err(_) => panic!(
            "Leaf {leaf_index} must attach an AgentSession: {:?}",
            host.execution(execution_id).await
        ),
    }
}

#[derive(Clone, Copy)]
enum SignalOrder {
    SubmitThenStop,
    StopThenSubmit,
}

#[tokio::test(flavor = "multi_thread")]
async fn test_workflow_terminal_spawn失敗を実行収束経路からruntime_failureへ保存する() {
    // Given
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("terminal-spawn-failure-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = root.path().join("bin/claude-workflow-fixture");
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(executable, permissions).unwrap();
    }

    // When
    let execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let runtime_failure = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = host.workflow_log(&execution_id).await.unwrap();
            if let Some(event) = events
                .into_iter()
                .find(|event| event["event"] == "runtime_failure_observed")
            {
                return event;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RuntimeFailureObservedFact must be appended");

    // Then
    assert_eq!(runtime_failure["kind"], "session");
    let failure_reason = runtime_failure["reason"].as_str().unwrap();
    assert!(failure_reason.contains("workflow runtime activation failed"));
    assert!(failure_reason.contains("activate Workflow AgentSession 'agent-session-"));
    assert!(failure_reason.contains("kind=pty_spawn"));
    assert!(failure_reason.contains("Failed to spawn shell:"));
    assert!(failure_reason.contains("Permission denied"));

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fanout_受理した33個sessionは同一worktreeで全て起動するか実行前に拒否する() {
    // Given
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("default-cap-fanout-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 1);

    // When
    let execution_id = host
        .start_default_capacity_fanout_workflow(&worktree)
        .await
        .expect("33 child fanout must pass definition validation");
    let observed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let execution = host.execution(&execution_id).await.unwrap().unwrap();
            let leaves = leaf_nodes(&execution);
            let has_failed = leaves
                .iter()
                .any(|leaf| leaf.status == AcceptanceNodeExecutionStatus::Failed);
            let all_started = leaves.len() == 33
                && leaves.iter().all(|leaf| {
                    leaf.agent_session_id.as_deref().is_some_and(|session_id| {
                        host.terminal().get(owner(&worktree, session_id)).is_ok()
                    })
                });
            if has_failed || all_started {
                return execution;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("accepted fanout must either start every Session or expose its runtime failure");

    // Then
    let leaves = leaf_nodes(&observed);
    assert_eq!(leaves.len(), 33);
    assert!(leaves.iter().all(|leaf| {
        leaf.agent_session_id
            .as_deref()
            .is_some_and(|session_id| host.terminal().get(owner(&worktree, session_id)).is_ok())
    }));
    let failed = leaves
        .iter()
        .filter(|leaf| leaf.status == AcceptanceNodeExecutionStatus::Failed)
        .collect::<Vec<_>>();
    assert!(
        failed.is_empty(),
        "validation accepted fanout capacity that runtime cannot execute: {failed:?}"
    );
    let log = host.workflow_log(&execution_id).await.unwrap();
    let serialized_log = serde_json::to_string(&log).unwrap();
    assert!(!serialized_log.contains("kind=per_worktree_cap"));
    assert!(!serialized_log.contains("kind=total_cap"));

    host.shutdown().await.unwrap();
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
    let initial = wait_for_node_count(&host, &execution_id, 2).await;
    let first = leaf_nodes(&initial)[0].clone();
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

    let advanced = wait_for_node_count(&host, &execution_id, 3).await;
    let leaves = leaf_nodes(&advanced);
    assert_eq!(leaves[0].status, AcceptanceNodeExecutionStatus::Succeeded);
    assert_eq!(leaves[1].status, AcceptanceNodeExecutionStatus::Running);
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
        let initial = wait_for_node_count(&host, &execution_id, 2).await;
        let first = leaf_nodes(&initial)[0].clone();
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
        let partial_leaves = leaf_nodes(&partial);
        assert_eq!(partial_leaves.len(), 1);
        assert_eq!(
            partial_leaves[0].status,
            AcceptanceNodeExecutionStatus::Running
        );
        assert_ne!(
            partial_leaves[0].submit_received,
            partial_leaves[0].stop_received
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

        let advanced = wait_for_node_count(&host, &execution_id, 3).await;
        let advanced_leaves = leaf_nodes(&advanced);
        assert_eq!(
            advanced_leaves[0].status,
            AcceptanceNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            advanced_leaves[1].status,
            AcceptanceNodeExecutionStatus::Running
        );
        let next_session_id = wait_for_leaf_session_attachment(&host, &execution_id, 1).await;
        assert_ne!(next_session_id, first_session_id);
        wait_for_agent_session_lifecycle(
            &host,
            &first_session_id,
            AcceptanceAgentSessionLifecycle::Paused,
        )
        .await;

        assert!(host.submit(&first.id).await.is_err());
        let after_duplicates = host.execution(&execution_id).await.unwrap().unwrap();
        let after_duplicate_leaves = leaf_nodes(&after_duplicates);
        assert_eq!(after_duplicate_leaves.len(), 2);
        assert_eq!(after_duplicate_leaves[1].id, advanced_leaves[1].id);
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
        assert_eq!(
            host.agent_session_lifecycle(&session_id).await.unwrap(),
            Some(AcceptanceAgentSessionLifecycle::Open)
        );
        host.terminal()
            .write(
                terminal_owner.clone(),
                &format!("waiting-approval-follow-up-{index}\r"),
            )
            .unwrap();
        receive_until(
            &mut terminal,
            &format!("waiting-approval-follow-up-{index}"),
        )
        .await;
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
                SignalOrder::SubmitThenStop => AcceptanceNodeExecutionStatus::Failed,
                SignalOrder::StopThenSubmit => AcceptanceNodeExecutionStatus::Running,
            }
        );
        let duplicate_start = host_after
            .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
            .await
            .unwrap_err();
        assert!(
            duplicate_start.starts_with("HTTP 409:"),
            "{duplicate_start}"
        );
        host_after.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_042_retryは旧attemptを停止し新attemptのterminalを維持する() {
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
    wait_for_agent_session_lifecycle(
        &host,
        &old_session_id,
        AcceptanceAgentSessionLifecycle::Paused,
    )
    .await;
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
    assert_eq!(
        host.agent_session_lifecycle(&old_session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Paused)
    );
    assert!(host.terminal().get(old_owner).is_err());

    assert_eq!(new_attempt.attempt, 2);
    assert_eq!(new_attempt.status, AcceptanceNodeExecutionStatus::Running);
    assert!(!new_attempt.submit_received);
    assert!(!new_attempt.stop_received);
    assert_ne!(new_attempt.agent_session_id, old_attempt.agent_session_id);

    let new_session_id = new_attempt.agent_session_id.as_deref().unwrap();
    assert_eq!(
        host.agent_session_lifecycle(new_session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );
    let new_owner = owner(&worktree, new_session_id);
    let mut new_terminal = host
        .terminal()
        .attach("atui-042-retry-new".to_string(), new_owner.clone())
        .unwrap();
    receive_until(&mut new_terminal, "releash-fixture-input-complete-0").await;
    host.terminal()
        .write(new_owner.clone(), "follow-up-after-retry\r")
        .unwrap();
    receive_until(&mut new_terminal, "follow-up-after-retry").await;
    assert!(!host.terminal().get(new_owner).unwrap().is_exited);

    let after_terminal_input = host.execution(&execution_id).await.unwrap().unwrap();
    let current = after_terminal_input
        .node_executions
        .iter()
        .find(|node| node.id == new_attempt.id)
        .unwrap();
    assert_eq!(current.status, AcceptanceNodeExecutionStatus::Running);
    assert!(!current.submit_received);
    assert!(!current.stop_received);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1696_session起動木はretryを拒否しsubmitとstopで資源を解放する() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("standalone-session-tree");
    let provider_launch_root = root.path().join("releash-data/provider-launches");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 4);
    let session_id = host
        .launch_manual_agent_session(
            &worktree,
            AcceptanceProvider::Claude,
            "issue-1696-standalone",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&worktree, &session_id);
    let mut terminal = host
        .terminal()
        .attach("issue-1696-standalone".to_string(), terminal_owner.clone())
        .unwrap();
    host.terminal()
        .write(terminal_owner.clone(), "standalone-input\r")
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(&host, &mut terminal, &terminal_owner, "provider-issue-1696").await;
    assert!(host
        .agent_session_has_active_launch_binding(&session_id)
        .await
        .unwrap());
    assert_eq!(host.active_provider_process_count(), 1);
    assert!(provider_launch_root.read_dir().unwrap().next().is_some());
    host.submit(&session_id).await.unwrap();
    let before_retry = host.execution_direct(&session_id).await.unwrap().unwrap();
    assert_eq!(before_retry.node_executions.len(), 1);
    assert!(before_retry.node_executions[0].submit_received);
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );

    let local_api_error = host.retry(&session_id, &session_id).await.unwrap_err();
    assert!(
        local_api_error.starts_with("HTTP 400:"),
        "{local_api_error}"
    );
    assert_eq!(
        host.execution_direct(&session_id).await.unwrap().unwrap(),
        before_retry
    );
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );

    let tauri_error = host
        .retry_workspace_node_from_tauri(&worktree, &session_id)
        .await
        .unwrap_err();
    assert!(
        tauri_error.contains("invalid execution_id"),
        "{tauri_error}"
    );
    assert_eq!(
        host.execution_direct(&session_id).await.unwrap().unwrap(),
        before_retry
    );
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );

    emit_provider_stop(&host, &mut terminal, &terminal_owner, "provider-issue-1696").await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let paused = host.agent_session_lifecycle(&session_id).await.unwrap()
                == Some(AcceptanceAgentSessionLifecycle::Paused);
            let binding_released = !host
                .agent_session_has_active_launch_binding(&session_id)
                .await
                .unwrap();
            let launch_resources_released = provider_launch_root
                .read_dir()
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true);
            if paused
                && binding_released
                && launch_resources_released
                && host.active_provider_process_count() == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Submit と Stop の完了後に AgentSession の起動資源が解放される");
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Paused)
    );
    assert!(host.terminal().get(terminal_owner).is_err());
    assert!(!host
        .agent_session_has_active_launch_binding(&session_id)
        .await
        .unwrap());
    assert!(provider_launch_root
        .read_dir()
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true));
    assert_eq!(host.active_provider_process_count(), 0);

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1696_archive_restore後のstopはcache上のsession木へ届きattentionになる() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("standalone-archive-restore");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 4);
    let session_id = host
        .launch_manual_agent_session(
            &worktree,
            AcceptanceProvider::Codex,
            "issue-1696-archive-restore",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&worktree, &session_id);
    let mut terminal = host
        .terminal()
        .attach(
            "issue-1696-archive-restore".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    host.terminal()
        .write(terminal_owner.clone(), "archive-restore-input\r")
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1696-archive-restore",
    )
    .await;

    host.archive_agent_session(&session_id).await.unwrap();
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Archived)
    );
    assert!(host.execution_direct(&session_id).await.unwrap().is_some());
    host.restore_agent_session(&session_id).await.unwrap();
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );
    let mut restored_terminal = host
        .terminal()
        .attach(
            "issue-1696-archive-restore-restored".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    receive_until(&mut restored_terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut restored_terminal,
        &terminal_owner,
        "provider-issue-1696-archive-restore",
    )
    .await;
    let process_exited_count = host
        .execution_fact_event_types(&session_id)
        .unwrap()
        .iter()
        .filter(|event| event.as_str() == "process_exited")
        .count();

    emit_provider_stop(
        &host,
        &mut restored_terminal,
        &terminal_owner,
        "provider-issue-1696-archive-restore",
    )
    .await;

    let execution = host.execution_direct(&session_id).await.unwrap().unwrap();
    assert_eq!(
        execution.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Running
    );
    assert!(
        execution.node_executions[0].stop_received,
        "facts after restored Stop: {:?}",
        host.execution_fact_event_types(&session_id).unwrap()
    );
    assert_eq!(
        host.workspace_node_status(&session_id).unwrap(),
        Some(AcceptanceWorkspaceNodeStatus::Attention)
    );
    assert_eq!(
        host.execution_fact_event_types(&session_id)
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "process_exited")
            .count(),
        process_exited_count
    );

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1700_stopとworkingを何度往復してもrunning_nodeの分類と事実を更新する() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("repeated-stop-activity");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 8);
    let execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let running = wait_for_node_count(&host, &execution_id, 1).await;
    let node = running.node_executions[0].clone();
    let terminal_owner = owner(&worktree, node.agent_session_id.as_deref().unwrap());
    let mut terminal = host
        .terminal()
        .attach(
            "issue-1700-repeated-stop".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1700-repeated-stop",
    )
    .await;

    for expected_stop_count in 1..=3 {
        emit_provider_working(
            &host,
            &mut terminal,
            &terminal_owner,
            "provider-issue-1700-repeated-stop",
        )
        .await;
        assert_eq!(
            host.workspace_node_status(&node.id).unwrap(),
            Some(AcceptanceWorkspaceNodeStatus::Active)
        );
        assert_eq!(
            host.workspace_node_detail_status(&worktree, &node.id)
                .unwrap()
                .as_deref(),
            Some("active")
        );

        emit_provider_stop(
            &host,
            &mut terminal,
            &terminal_owner,
            "provider-issue-1700-repeated-stop",
        )
        .await;
        let after_stop = host.execution(&execution_id).await.unwrap().unwrap();
        assert_eq!(
            after_stop.node_executions[0].status,
            AcceptanceNodeExecutionStatus::Running
        );
        assert!(after_stop.node_executions[0].stop_received);
        assert!(!after_stop.node_executions[0].submit_received);
        assert_eq!(
            host.workspace_node_status(&node.id).unwrap(),
            Some(AcceptanceWorkspaceNodeStatus::Attention)
        );
        assert_eq!(
            host.workspace_node_detail_status(&worktree, &node.id)
                .unwrap()
                .as_deref(),
            Some("attention")
        );
        assert_eq!(
            host.execution_fact_event_types(&execution_id)
                .unwrap()
                .iter()
                .filter(|event| event.as_str() == "stop_received")
                .count(),
            expected_stop_count
        );
    }

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1700_waiting_approval_nodeのstopも活動分類をattentionへ戻す() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("waiting-approval-repeated-stop");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 5);
    let execution_id = host
        .start_approval_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let running = wait_for_node_count(&host, &execution_id, 1).await;
    let node = running.node_executions[0].clone();
    let terminal_owner = owner(&worktree, node.agent_session_id.as_deref().unwrap());
    let mut terminal = host
        .terminal()
        .attach(
            "issue-1700-waiting-approval-stop".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;
    associate_provider_session(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1700-waiting-approval",
    )
    .await;
    host.submit(&node.id).await.unwrap();
    emit_provider_stop(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1700-waiting-approval",
    )
    .await;
    let waiting = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        waiting.node_executions[0].status,
        AcceptanceNodeExecutionStatus::WaitingApproval
    );

    emit_provider_working(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1700-waiting-approval",
    )
    .await;
    assert_eq!(
        host.workspace_node_status(&node.id).unwrap(),
        Some(AcceptanceWorkspaceNodeStatus::Active)
    );
    emit_provider_stop(
        &host,
        &mut terminal,
        &terminal_owner,
        "provider-issue-1700-waiting-approval",
    )
    .await;

    let after_second_stop = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        after_second_stop.node_executions[0].status,
        AcceptanceNodeExecutionStatus::WaitingApproval
    );
    assert_eq!(
        host.workspace_node_status(&node.id).unwrap(),
        Some(AcceptanceWorkspaceNodeStatus::Attention)
    );
    assert_eq!(
        host.workspace_node_detail_status(&worktree, &node.id)
            .unwrap()
            .as_deref(),
        Some("attention")
    );
    assert_eq!(
        host.execution_fact_event_types(&execution_id)
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "stop_received")
            .count(),
        2
    );

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
            .filter(|event| event["event"] == "submit_received")
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

    assert!(host
        .submit_artifact(
            &node.id,
            "acceptance-result",
            serde_json::json!({"result": "terminal replacement"}),
        )
        .await
        .is_err());
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
async fn test_issue_1654_workflow完了時にproviderを停止しcheckpointからresumeできる() {
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
    host.submit(&node.id).await.unwrap();
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Completed,
    )
    .await;
    wait_for_agent_session_lifecycle(&host, &session_id, AcceptanceAgentSessionLifecycle::Paused)
        .await;

    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Paused),
    );
    assert!(host.terminal().get(terminal_owner.clone()).is_err());

    host.resume_agent_session(&session_id).await.unwrap();
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open),
    );
    let mut resumed_terminal = host
        .terminal()
        .attach(
            "workflow-completion-resume".to_string(),
            terminal_owner.clone(),
        )
        .unwrap();
    // 復元画面の replay は resumed fixture の起動出力で上書きされ得るため、
    // 同期は「resumed fixture が実際に入力を消費すること」で取る。echo と marker は
    // 同じ Snapshot / 出力 batch に載って届き得るため 1 回の accumulator で両方待つ。
    host.terminal()
        .write(terminal_owner.clone(), "follow-up-after-completion\r")
        .unwrap();
    receive_until_all(
        &mut resumed_terminal,
        &[
            "follow-up-after-completion",
            "releash-fixture-input-complete-0",
        ],
    )
    .await;
    send_hook(
        &host,
        &mut resumed_terminal,
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

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1654_execution_stop中はproviderを残して同じagent_sessionでresumeする() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("stop-resume-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 4);
    let execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let running = wait_for_node_count(&host, &execution_id, 1).await;
    let node = running.node_executions[0].clone();
    let session_id = node.agent_session_id.clone().unwrap();
    let terminal_owner = owner(&worktree, &session_id);
    let mut terminal = host
        .terminal()
        .attach("issue-1654-stop-resume".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut terminal, "releash-fixture-input-complete-0").await;

    host.stop(&execution_id).await.unwrap();
    let stopped = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(stopped.status, AcceptanceWorkflowExecutionStatus::Running);
    assert_eq!(
        stopped.node_executions[0].agent_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        host.agent_session_lifecycle(&session_id).await.unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );
    assert!(
        !host
            .terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .is_exited
    );

    host.resume(&execution_id).await.unwrap();
    let resumed = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        resumed.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Running
    );
    assert_eq!(
        resumed.node_executions[0].agent_session_id.as_deref(),
        Some(session_id.as_str())
    );
    host.terminal()
        .write(terminal_owner.clone(), "follow-up-after-workflow-resume\r")
        .unwrap();
    receive_until(&mut terminal, "follow-up-after-workflow-resume").await;

    host.abort(&execution_id).await.unwrap();
    wait_for_execution_status(
        &host,
        &execution_id,
        AcceptanceWorkflowExecutionStatus::Aborted,
    )
    .await;
    wait_for_agent_session_lifecycle(&host, &session_id, AcceptanceAgentSessionLifecycle::Paused)
        .await;
    let aborted = host.execution(&execution_id).await.unwrap().unwrap();
    assert_eq!(
        aborted.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Aborted
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_issue_1654_cap超過回数の終端後もworkflowと手動sessionを起動できる() {
    let root = tempfile::TempDir::new().unwrap();
    let worktree = root.path().join("cap-worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.to_string_lossy().into_owned();
    let host = host(root.path(), 1);

    for index in 0..33 {
        let execution_id = host
            .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
            .await
            .unwrap_or_else(|error| panic!("workflow {index} failed to start: {error}"));
        let running = wait_for_node_count(&host, &execution_id, 1).await;
        let node = running.node_executions[0].clone();
        let session_id = node.agent_session_id.clone().unwrap();
        host.abort(&execution_id).await.unwrap();
        wait_for_execution_status(
            &host,
            &execution_id,
            AcceptanceWorkflowExecutionStatus::Aborted,
        )
        .await;
        wait_for_agent_session_lifecycle(
            &host,
            &session_id,
            AcceptanceAgentSessionLifecycle::Paused,
        )
        .await;
        assert_eq!(
            host.execution(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .node_executions[0]
                .status,
            AcceptanceNodeExecutionStatus::Aborted
        );
    }

    let next_execution_id = host
        .start_auto_workflow(&worktree, AcceptanceProvider::Claude)
        .await
        .unwrap();
    let next = wait_for_node_count(&host, &next_execution_id, 1).await;
    assert_eq!(
        next.node_executions[0].status,
        AcceptanceNodeExecutionStatus::Running
    );
    let manual_session_id = host
        .launch_manual_agent_session(
            &worktree,
            AcceptanceProvider::Codex,
            "issue-1654-manual-after-cap",
        )
        .await
        .unwrap();
    assert_eq!(
        host.agent_session_lifecycle(&manual_session_id)
            .await
            .unwrap(),
        Some(AcceptanceAgentSessionLifecycle::Open)
    );

    host.abort(&next_execution_id).await.unwrap();
    host.shutdown().await.unwrap();
}
