use releashd::test_support::terminal_subscription_acceptance::TerminalSubscription as TerminalSurfaceWireAttachment;
#[path = "agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_tui_fixture::{fixture_process_shell_command, FixtureLifecycleCommand, FixturePlan};
use releashd::test_support::agent_session_tui_acceptance::{
    AcceptanceAgentSessionLifecycle, AcceptanceAgentSessionTreeLocation, AcceptanceArchiveOutcome,
    AcceptanceHookWarning, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
    AgentSessionTuiAcceptanceHost as AgentSessionTuiAcceptanceComposition,
};
use releashd::test_support::terminal_surface::{
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionSelection {
    agent_session_id: String,
    node_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionHistoryPage {
    items: Vec<releashd::test_support::agent_session_tui_acceptance::AcceptanceHistoryCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAvailabilitySnapshot {
    providers: Vec<ProviderAvailabilityItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAvailabilityItem {
    provider: AcceptanceProvider,
    default_executable: String,
    configured_executable: Option<String>,
    effective_executable: String,
    available: bool,
    resolved_executable: Option<String>,
    unavailable_reason: Option<String>,
}

struct AgentSessionTuiAcceptanceHost {
    composition: AgentSessionTuiAcceptanceComposition,
    client: releashd::test_support::client_api_acceptance::NativeClient,
}

impl AgentSessionTuiAcceptanceHost {
    fn start(config: AgentSessionTuiAcceptanceConfig) -> Result<Self, String> {
        let composition = AgentSessionTuiAcceptanceComposition::start(config)?;
        let client = releashd::test_support::client_api_acceptance::connect_client(
            composition.client_endpoint(),
        );
        Ok(Self {
            composition,
            client,
        })
    }

    fn invoke<T: DeserializeOwned>(
        &self,
        command: &str,
        body: serde_json::Value,
    ) -> Result<T, String> {
        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                releashd::test_support::client_api_acceptance::request_client(
                    &self.client,
                    command,
                    body,
                ),
            )
        })
        .map_err(|error| error.to_string())?;
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    fn terminal(
        &self,
    ) -> &releashd::test_support::terminal_subscription_acceptance::TerminalSubscriptionHarness
    {
        self.composition.terminal()
    }

    fn stop_local_api(&self) -> Result<(), String> {
        self.composition.stop_local_api()
    }

    fn restart_local_api(&self) -> Result<(), String> {
        self.composition.restart_local_api()
    }

    fn hook_warnings(&self) -> Result<Vec<AcceptanceHookWarning>, String> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Snapshot {
            warnings: Vec<AcceptanceHookWarning>,
            read_errors: Vec<String>,
        }
        let snapshot: Snapshot = self.read_state("provider-hook-health")?;
        if !snapshot.read_errors.is_empty() {
            return Err(snapshot.read_errors.join("; "));
        }
        Ok(snapshot.warnings)
    }

    fn hook_health_marker_contents(&self) -> Result<Vec<String>, String> {
        self.composition.hook_health_marker_contents()
    }
    fn read_state<T: DeserializeOwned>(&self, target: &str) -> Result<T, String> {
        let value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                releashd::test_support::client_api_acceptance::read_state(&self.client, target),
            )
        })
        .map_err(|e| e.to_string())?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    fn available_providers(&self) -> Vec<AcceptanceProvider> {
        self.read_state("providers")
            .expect("available Provider subscription")
    }

    fn provider_availability(&self) -> Result<ProviderAvailabilitySnapshot, String> {
        self.read_state("provider-availability")
    }

    fn update_provider_executable(
        &self,
        provider: AcceptanceProvider,
        executable: &Path,
    ) -> Result<(), String> {
        self.invoke(
            "update_provider_executable",
            serde_json::json!({
                "provider": provider_name(provider),
                "executable": executable.to_string_lossy(),
            }),
        )
    }

    fn reset_provider_executable(&self, provider: AcceptanceProvider) -> Result<(), String> {
        self.invoke(
            "reset_provider_executable",
            serde_json::json!({ "provider": provider_name(provider) }),
        )
    }

    fn refresh_provider_availability(&self) -> Result<(), String> {
        self.invoke("refresh_provider_availability", serde_json::json!({}))
    }

    #[allow(clippy::too_many_arguments)]
    async fn launch_standalone(
        &self,
        workspace_identity: &str,
        worktree_path: &str,
        provider: AcceptanceProvider,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<String, String> {
        let selection: SessionSelection = self.invoke(
            "create_agent_session",
            serde_json::json!({
                "workspaceIdentity": workspace_identity,
                "worktreePath": worktree_path,
                "provider": provider_name(provider),
                "rows": rows,
                "cols": cols,
                "callerRequestId": caller_request_id,
            }),
        )?;
        assert_eq!(selection.node_id, selection.agent_session_id);
        Ok(selection.agent_session_id)
    }

    async fn launch_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<String, String> {
        self.composition
            .launch_workflow(
                worktree_path,
                provider,
                workflow_execution_id,
                node_execution_id,
                initial_instruction,
            )
            .await
    }

    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<
        Option<releashd::test_support::agent_session_tui_acceptance::AcceptanceAgentSession>,
        String,
    > {
        let target = format!(
            "agent-session:{}:{agent_session_id}",
            agent_session_id.len()
        );
        match releashd::test_support::client_api_acceptance::read_state(&self.client, &target).await
        {
            Ok(value) => serde_json::from_value(value).map_err(|e| e.to_string()),
            Err(error) if error.code == connectrpc::ErrorCode::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn list_history(
        &self,
        worktree_path: &str,
        limit: usize,
    ) -> Result<
        Vec<releashd::test_support::agent_session_tui_acceptance::AcceptanceHistoryCandidate>,
        String,
    > {
        let count = limit.to_string();
        self.read_state::<AgentSessionHistoryPage>(&format!(
            "session-history:{}:{worktree_path}{}:{count}",
            worktree_path.len(),
            count.len()
        ))
        .map(|page| page.items)
    }

    #[allow(clippy::too_many_arguments)]
    async fn resume_history(
        &self,
        workspace_identity: &str,
        worktree_path: &str,
        provider: AcceptanceProvider,
        provider_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<String, String> {
        let selection: SessionSelection = self.invoke(
            "resume_agent_session_history_candidate",
            serde_json::json!({
                "workspaceIdentity": workspace_identity,
                "worktreePath": worktree_path,
                "provider": provider_name(provider),
                "providerSessionId": provider_session_id,
                "rows": rows,
                "cols": cols,
                "callerRequestId": caller_request_id,
            }),
        )?;
        assert_eq!(selection.node_id, selection.agent_session_id);
        Ok(selection.agent_session_id)
    }

    async fn archive(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AcceptanceArchiveOutcome, String> {
        self.invoke(
            "archive_agent_session",
            serde_json::json!({
                "agentSessionId": agent_session_id,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn restore(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<String, String> {
        let selection: SessionSelection = self.invoke("restore_agent_session", serde_json::json!({
            "agentSessionId": agent_session_id, "rows": rows, "cols": cols, "callerRequestId": caller_request_id,
        }))?;
        assert_eq!(selection.node_id, selection.agent_session_id);
        Ok(selection.agent_session_id)
    }

    async fn resume_session_node(&self, node_execution_id: &str) -> Result<(), String> {
        self.composition
            .resume_session_node(node_execution_id)
            .await
    }

    async fn delete(&self, agent_session_id: &str, caller_request_id: &str) -> Result<(), String> {
        self.invoke(
            "delete_agent_session",
            serde_json::json!({
                "agentSessionId": agent_session_id,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn wait_until_lifecycle(
        &self,
        agent_session_id: &str,
        expected: AcceptanceAgentSessionLifecycle,
    ) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.get(agent_session_id).await.is_ok_and(|session| {
                    session.is_some_and(|session| session.lifecycle == expected)
                }) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| format!("timed out waiting for AgentSession lifecycle {expected:?}"))
    }

    async fn wait_until_removed(&self, agent_session_id: &str) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .get(agent_session_id)
                    .await
                    .is_ok_and(|session| session.is_none())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "timed out waiting for AgentSession removal".to_string())
    }

    async fn wait_until_exited(
        &self,
        workspace_identity: &str,
        agent_session_id: &str,
    ) -> Result<(), String> {
        self.composition
            .wait_until_exited(workspace_identity, agent_session_id)
            .await
    }

    async fn shutdown(self) -> Result<(), String> {
        drop(self.client);
        self.composition.shutdown().await
    }
}

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
        lifecycle_command: Some(FixtureLifecycleCommand {
            executable: std::path::Path::new(env!("CARGO_BIN_EXE_releashd"))
                .with_file_name("releash")
                .to_string_lossy()
                .into_owned(),
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

fn host(root: &Path, input_lines: usize) -> (AgentSessionTuiAcceptanceHost, PathBuf, PathBuf) {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = install_fixture_executable(
        &bin,
        "claude-fixture",
        AcceptanceProvider::Claude,
        input_lines,
    );
    let codex = install_fixture_executable(
        &bin,
        "codex-fixture",
        AcceptanceProvider::Codex,
        input_lines,
    );
    let claude_home = root.join("claude-home");
    let codex_home = root.join("codex-home");
    let host = AgentSessionTuiAcceptanceHost::start(AgentSessionTuiAcceptanceConfig {
        data_dir: root.join("releash-data"),
        claude_executable: Some(claude),
        codex_executable: Some(codex),
        provider_search_path: None,
        provider_refresh_search_path: None,
        claude_config_dir: claude_home.clone(),
        codex_home: codex_home.clone(),
    })
    .unwrap();
    (host, claude_home, codex_home)
}

fn owner(workspace: &str, session_id: &str) -> TerminalSurfaceOwnerV1 {
    TerminalSurfaceOwnerV1::Session {
        workspace_path: workspace.to_string(),
        session_id: session_id.to_string(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_provider利用可否_初期化と変更をproduction境界で確認する() {
    // Given
    let root = tempfile::TempDir::new().unwrap();
    let login_shell_bin = root.path().join("login-shell-bin");
    std::fs::create_dir(&login_shell_bin).unwrap();
    let refreshed_login_shell_bin = root.path().join("refreshed-login-shell-bin");
    std::fs::create_dir(&refreshed_login_shell_bin).unwrap();
    let executable =
        install_fixture_executable(&login_shell_bin, "claude", AcceptanceProvider::Claude, 4);
    let host = AgentSessionTuiAcceptanceHost::start(AgentSessionTuiAcceptanceConfig {
        data_dir: root.path().join("data"),
        claude_executable: None,
        codex_executable: None,
        provider_search_path: Some(login_shell_bin.into_os_string()),
        provider_refresh_search_path: Some(refreshed_login_shell_bin.clone().into_os_string()),
        claude_config_dir: root.path().join("claude-home"),
        codex_home: root.path().join("codex-home"),
    })
    .unwrap();

    // When
    let snapshot = host.provider_availability().unwrap();
    let unknown = host.invoke::<()>(
        "update_provider_executable",
        serde_json::json!({ "provider": "unknown", "executable": "agent" }),
    );
    let blank = host.invoke::<()>(
        "update_provider_executable",
        serde_json::json!({ "provider": "claude", "executable": "  " }),
    );
    let available = host.available_providers();

    // Then
    assert!(unknown.is_err());
    assert!(blank.is_err());
    assert_eq!(snapshot.providers.len(), 2);
    let claude = snapshot
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Claude)
        .unwrap();
    assert!(claude.available);
    assert_eq!(
        claude.resolved_executable.as_deref(),
        Some(executable.to_string_lossy().as_ref())
    );
    assert_eq!(claude.unavailable_reason, None);
    let codex = snapshot
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert!(!codex.available);
    assert_eq!(codex.resolved_executable, None);
    assert_eq!(codex.unavailable_reason.as_deref(), Some("not_found"));
    assert_eq!(available, vec![AcceptanceProvider::Claude]);

    // Given
    let refreshed_codex = install_fixture_executable(
        &refreshed_login_shell_bin,
        "codex",
        AcceptanceProvider::Codex,
        4,
    );
    // When
    host.refresh_provider_availability().unwrap();
    let refreshed = host.provider_availability().unwrap();
    let available = host.available_providers();
    // Then
    let claude = refreshed
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Claude)
        .unwrap();
    assert!(!claude.available);
    let codex = refreshed
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert_eq!(
        codex.resolved_executable.as_deref(),
        Some(refreshed_codex.to_string_lossy().as_ref())
    );
    assert_eq!(available, vec![AcceptanceProvider::Codex]);

    // Given
    let replacement = install_fixture_executable(
        root.path(),
        "non-standard-codex",
        AcceptanceProvider::Codex,
        4,
    );
    // When
    host.update_provider_executable(AcceptanceProvider::Codex, &replacement)
        .unwrap();
    let updated = host.provider_availability().unwrap();
    let available = host.available_providers();
    // Then
    let codex = updated
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert!(codex.available);
    assert_eq!(
        codex.configured_executable.as_deref(),
        Some(replacement.to_string_lossy().as_ref())
    );
    assert_eq!(
        codex.resolved_executable.as_deref(),
        Some(replacement.to_string_lossy().as_ref())
    );
    assert!(available.contains(&AcceptanceProvider::Codex));

    // Given
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    // When
    let standalone_id = host
        .launch_standalone(
            "workspace-atui-025",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "atui-025-standalone",
        )
        .await
        .unwrap();
    let standalone_owner = owner("workspace-atui-025", &standalone_id);
    let mut standalone = host
        .terminal()
        .subscribe("atui-025-standalone".to_string(), standalone_owner.clone())
        .await
        .unwrap();
    receive_until(&mut standalone, "non-standard-codex").await;
    let workflow_launch = host
        .launch_workflow(
            &workspace,
            AcceptanceProvider::Codex,
            "00000000-0000-4000-8000-000000000025",
            "atui-025-node",
            "verify shared registry",
        )
        .await;
    // Then
    assert!(workflow_launch.is_ok());

    // Given
    std::fs::remove_file(&replacement).unwrap();
    // When
    host.refresh_provider_availability().unwrap();
    let refreshed = host.provider_availability().unwrap();
    let standalone_rejected = host
        .launch_standalone(
            "workspace-atui-025",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "atui-025-unavailable",
        )
        .await;
    let workflow_rejected = host
        .launch_workflow(
            &workspace,
            AcceptanceProvider::Codex,
            "00000000-0000-4000-8000-000000000026",
            "atui-025-node-unavailable",
            "must be rejected before creation",
        )
        .await;
    let retained = host.get(&standalone_id).await.unwrap();
    // Then
    let codex = refreshed
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert!(!codex.available);
    assert_eq!(codex.unavailable_reason.as_deref(), Some("not_found"));
    assert!(standalone_rejected.is_err());
    assert!(workflow_rejected.is_err());
    assert!(retained.is_some());

    // When
    host.terminal()
        .write(standalone_owner, "still-running\r")
        .unwrap();
    receive_until(&mut standalone, "received-0:still-running").await;

    host.reset_provider_executable(AcceptanceProvider::Codex)
        .unwrap();
    let reset = host.provider_availability().unwrap();
    // Then
    let codex = reset
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert_eq!(codex.configured_executable, None);
    assert_eq!(codex.default_executable, "codex");
    assert_eq!(codex.effective_executable, "codex");

    // When
    let shutdown = host.shutdown().await;
    // Then
    assert!(shutdown.is_ok());
}

fn fixture_label(provider: AcceptanceProvider) -> &'static str {
    match provider {
        AcceptanceProvider::Claude => "claude-fixture",
        AcceptanceProvider::Codex => "codex-fixture",
    }
}

async fn receive_until(attachment: &mut TerminalSurfaceWireAttachment, needle: &str) {
    let mut output = String::new();
    tokio::time::timeout(Duration::from_secs(10), async {
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
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}; terminal={output:?}"));
}

async fn send_session_start(
    host: &AgentSessionTuiAcceptanceHost,
    attachment: &mut TerminalSurfaceWireAttachment,
    terminal_owner: &TerminalSurfaceOwnerV1,
    provider_session_id: &str,
) {
    let input = format!("releash-fixture-session-start:{provider_session_id}");
    host.terminal()
        .write(terminal_owner.clone(), &format!("{input}\r"))
        .unwrap();
    receive_until(attachment, "releash-fixture-lifecycle-command-result:").await;
}

async fn emit_session_start(
    host: &AgentSessionTuiAcceptanceHost,
    attachment: &mut TerminalSurfaceWireAttachment,
    terminal_owner: &TerminalSurfaceOwnerV1,
    agent_session_id: &str,
    provider_session_id: &str,
) {
    send_session_start(host, attachment, terminal_owner, provider_session_id).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if host
                .get(agent_session_id)
                .await
                .unwrap()
                .is_some_and(|session| {
                    session.provider_session_id.as_deref() == Some(provider_session_id)
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SessionStart Hook must associate the Provider session ID");
}

async fn emit_raw_hook_payload(
    host: &AgentSessionTuiAcceptanceHost,
    attachment: &mut TerminalSurfaceWireAttachment,
    terminal_owner: &TerminalSurfaceOwnerV1,
    payload: serde_json::Value,
) {
    host.terminal()
        .write(
            terminal_owner.clone(),
            &format!("releash-fixture-hook-json:{payload}\r"),
        )
        .unwrap();
    receive_until(attachment, "releash-fixture-lifecycle-command-result:").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider選択からarchive_restore_deleteまで旧messageなしで成立する() {
    for provider in [AcceptanceProvider::Claude, AcceptanceProvider::Codex] {
        let root = tempfile::TempDir::new().unwrap();
        let workspace = root.path().join("worktree");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.to_string_lossy().into_owned();
        let (host, _, _) = host(root.path(), 8);

        assert!(host.available_providers().contains(&provider));
        let session_id = host
            .launch_standalone("workspace-1", &workspace, provider, 24, 80, "launch")
            .await
            .unwrap();
        assert_eq!(
            host.get(&session_id).await.unwrap().unwrap().tree_location,
            AcceptanceAgentSessionTreeLocation {
                tree_id: session_id.clone(),
                node_execution_id: session_id.clone(),
            }
        );
        let terminal_owner = owner("workspace-1", &session_id);
        use releashd::test_support::client_api_acceptance::rpc;
        let client_id = format!("terminal-wire-{session_id}");
        let mut stream = host
            .client
            .open_state_stream(rpc::OpenStateStreamRequest {
                client_id: client_id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .unwrap();
        host.client
            .start_state_subscription(rpc::StartStateSubscriptionRequest {
                client_id: client_id.clone(),
                target: "terminal".into(),
                args: vec!["workspace-1".into(), session_id.clone()],
                subscription_id: client_id,
                ..Default::default()
            })
            .await
            .unwrap();
        let snapshot = tokio::time::timeout(
            Duration::from_secs(5),
            stream.message::<rpc::StateSubscriptionEvent>(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .to_owned_message();
        assert!(matches!(
            snapshot.event,
            Some(rpc::state_subscription_event::Event::Snapshot(_))
        ));
        drop(stream);

        let mut attached = host
            .terminal()
            .subscribe("acceptance-first".to_string(), terminal_owner.clone())
            .await
            .unwrap();
        receive_until(&mut attached, fixture_label(provider)).await;
        host.terminal()
            .write(terminal_owner.clone(), "permission-approved\r")
            .unwrap();
        receive_until(&mut attached, "received-0:permission-approved").await;
        drop(attached);

        let mut reloaded = host
            .terminal()
            .subscribe("acceptance-reload".to_string(), terminal_owner.clone())
            .await
            .unwrap();
        receive_until(&mut reloaded, "received-0:permission-approved").await;
        emit_session_start(
            &host,
            &mut reloaded,
            &terminal_owner,
            &session_id,
            &format!("provider-{provider:?}"),
        )
        .await;

        assert_eq!(
            host.archive(&session_id, "archive").await.unwrap(),
            AcceptanceArchiveOutcome::Archived
        );
        assert_eq!(
            host.get(&session_id).await.unwrap().unwrap().lifecycle,
            AcceptanceAgentSessionLifecycle::Archived
        );
        assert_eq!(
            host.restore(&session_id, 24, 80, "restore").await.unwrap(),
            session_id
        );
        assert_eq!(
            host.get(&session_id).await.unwrap().unwrap().lifecycle,
            AcceptanceAgentSessionLifecycle::Paused
        );
        assert!(host.terminal().get(terminal_owner.clone()).is_err());
        host.resume_session_node(&session_id).await.unwrap();
        assert!(
            !host
                .terminal()
                .get(terminal_owner.clone())
                .unwrap()
                .is_exited
        );
        assert_eq!(
            host.archive(&session_id, "archive-again").await.unwrap(),
            AcceptanceArchiveOutcome::Archived
        );
        host.delete(&session_id, "delete").await.unwrap();
        assert!(host.get(&session_id).await.unwrap().is_none());
        assert!(host.terminal().get(terminal_owner).is_err());
        host.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_workflow初期指示は一度だけで追加質問もterminalを操作できる() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 4);
    let session_id = host
        .launch_workflow(
            &workspace,
            AcceptanceProvider::Claude,
            "00000000-0000-4000-8000-000000001696",
            "node-execution-1",
            "system policy\n\nimplement once",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&workspace, &session_id);
    let mut attached = host
        .terminal()
        .subscribe("workflow-session".to_string(), terminal_owner.clone())
        .await
        .unwrap();
    receive_until(
        &mut attached,
        "received-0:system policy\\n\\nimplement once",
    )
    .await;

    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().tree_location,
        AcceptanceAgentSessionTreeLocation {
            tree_id: "00000000-0000-4000-8000-000000001696".to_string(),
            node_execution_id: "node-execution-1".to_string(),
        }
    );
    assert!(
        !host
            .terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .is_exited
    );
    host.terminal()
        .write(terminal_owner, "follow-up-question\r")
        .unwrap();
    receive_until(&mut attached, "received-1:follow-up-question").await;
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_subagentを無視し複数turnのstop後もagent_sessionとptyを維持する() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 6);
    let session_id = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "multi-turn-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner("workspace-1", &session_id);
    let mut terminal = host
        .terminal()
        .subscribe("multi-turn-session".to_string(), terminal_owner.clone())
        .await
        .unwrap();
    receive_until(&mut terminal, fixture_label(AcceptanceProvider::Claude)).await;

    emit_raw_hook_payload(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "subagent-session",
            "transcript_path": "provider://fixture/subagent-session",
            "hook_event_name": "SessionStart",
            "agent_id": "subagent-1"
        }),
    )
    .await;
    assert!(host
        .get(&session_id)
        .await
        .unwrap()
        .unwrap()
        .provider_session_id
        .is_none());

    emit_session_start(
        &host,
        &mut terminal,
        &terminal_owner,
        &session_id,
        "root-session",
    )
    .await;
    for turn in 1..=2 {
        emit_raw_hook_payload(
            &host,
            &mut terminal,
            &terminal_owner,
            serde_json::json!({
                "session_id": "root-session",
                "transcript_path": "provider://fixture/root-session",
                "hook_event_name": "Stop",
                "turn": turn
            }),
        )
        .await;
    }

    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Open
    );
    assert!(
        !host
            .terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .is_exited
    );
    host.terminal()
        .write(terminal_owner, "follow-up-after-stops\r")
        .unwrap();
    receive_until(&mut terminal, "received-4:follow-up-after-stops").await;
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_hook配送失敗でもhealthを記録せずprocessと後続送信を維持する() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 5);
    let session_id = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "hook-health-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner("workspace-1", &session_id);
    let mut terminal = host
        .terminal()
        .subscribe("hook-health-session".to_string(), terminal_owner.clone())
        .await
        .unwrap();
    receive_until(&mut terminal, fixture_label(AcceptanceProvider::Claude)).await;
    emit_session_start(
        &host,
        &mut terminal,
        &terminal_owner,
        &session_id,
        "hook-health-root",
    )
    .await;

    host.stop_local_api().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    emit_raw_hook_payload(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "hook-health-root",
            "transcript_path": "provider://fixture/hook-health-root",
            "hook_event_name": "Stop"
        }),
    )
    .await;

    let warnings = host.hook_warnings().unwrap();
    assert_eq!(
        warnings.len(),
        0,
        "hook health markers: {:?}",
        host.hook_health_marker_contents().unwrap()
    );
    assert!(host.hook_health_marker_contents().unwrap().is_empty());
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Open
    );
    assert!(
        !host
            .terminal()
            .get(terminal_owner.clone())
            .unwrap()
            .is_exited
    );

    host.restart_local_api().unwrap();
    emit_raw_hook_payload(
        &host,
        &mut terminal,
        &terminal_owner,
        serde_json::json!({
            "session_id": "hook-health-root",
            "transcript_path": "provider://fixture/hook-health-root",
            "hook_event_name": "SessionStart"
        }),
    )
    .await;
    assert!(host.hook_warnings().unwrap().is_empty());

    host.terminal()
        .write(terminal_owner, "follow-up-after-hook-recovery\r")
        .unwrap();
    receive_until(&mut terminal, "received-3:follow-up-after-hook-recovery").await;
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider利用不可とduplicate所有を永続境界で拒否する() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 2);
    let first = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "first",
        )
        .await
        .unwrap();
    let second = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "second",
        )
        .await
        .unwrap();
    let first_owner = owner("workspace-1", &first);
    let mut first_terminal = host
        .terminal()
        .subscribe("duplicate-first".to_string(), first_owner.clone())
        .await
        .unwrap();
    receive_until(
        &mut first_terminal,
        fixture_label(AcceptanceProvider::Claude),
    )
    .await;
    emit_session_start(
        &host,
        &mut first_terminal,
        &first_owner,
        &first,
        "same-provider-id",
    )
    .await;
    let second_owner = owner("workspace-1", &second);
    let mut second_terminal = host
        .terminal()
        .subscribe("duplicate-second".to_string(), second_owner.clone())
        .await
        .unwrap();
    receive_until(
        &mut second_terminal,
        fixture_label(AcceptanceProvider::Claude),
    )
    .await;
    send_session_start(
        &host,
        &mut second_terminal,
        &second_owner,
        "same-provider-id",
    )
    .await;
    assert!(host
        .get(&second)
        .await
        .unwrap()
        .unwrap()
        .provider_session_id
        .is_none());
    host.shutdown().await.unwrap();

    let unavailable_root = tempfile::TempDir::new().unwrap();
    let unavailable = AgentSessionTuiAcceptanceHost::start(AgentSessionTuiAcceptanceConfig {
        data_dir: unavailable_root.path().join("data"),
        claude_executable: Some(unavailable_root.path().join("missing-claude")),
        codex_executable: Some(unavailable_root.path().join("missing-codex")),
        provider_search_path: None,
        provider_refresh_search_path: None,
        claude_config_dir: unavailable_root.path().join("claude"),
        codex_home: unavailable_root.path().join("codex"),
    })
    .unwrap();
    assert!(unavailable.available_providers().is_empty());
    assert!(unavailable
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "unavailable",
        )
        .await
        .is_err());
    unavailable.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_process終了はprovider_idの有無に応じてpausedまたはgcになる() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 1);

    let paused = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "paused-launch",
        )
        .await
        .unwrap();
    let paused_owner = owner("workspace-1", &paused);
    let mut paused_terminal = host
        .terminal()
        .subscribe("paused-session".to_string(), paused_owner.clone())
        .await
        .unwrap();
    receive_until(
        &mut paused_terminal,
        fixture_label(AcceptanceProvider::Claude),
    )
    .await;
    emit_session_start(
        &host,
        &mut paused_terminal,
        &paused_owner,
        &paused,
        "resume-id",
    )
    .await;
    host.wait_until_exited("workspace-1", &paused)
        .await
        .unwrap();
    host.wait_until_lifecycle(&paused, AcceptanceAgentSessionLifecycle::Paused)
        .await
        .unwrap();
    assert_eq!(
        host.get(&paused).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Paused
    );
    host.resume_session_node(&paused).await.unwrap();

    let gc = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "gc-launch",
        )
        .await
        .unwrap();
    let gc_owner = owner("workspace-1", &gc);
    let mut gc_terminal = host
        .terminal()
        .subscribe("gc-session".to_string(), gc_owner.clone())
        .await
        .unwrap();
    receive_until(&mut gc_terminal, fixture_label(AcceptanceProvider::Codex)).await;
    host.terminal()
        .write(gc_owner.clone(), "exit-now\r")
        .unwrap();
    host.wait_until_removed(&gc).await.unwrap();
    assert!(host.get(&gc).await.unwrap().is_none());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider履歴はmetadataだけを列挙し新しいsessionとして復帰する() {
    // Given
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, claude_home, codex_home) = host(root.path(), 6);

    let claude_project = workspace
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let claude_project = claude_home.join("projects").join(claude_project);
    std::fs::create_dir_all(&claude_project).unwrap();
    std::fs::write(
        claude_project.join("claude-history.jsonl"),
        "{\"message\":\"transcript body must not be read\"}\n",
    )
    .unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();
    let connection = rusqlite::Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, updated_at INTEGER, name TEXT, first_user_message TEXT);\
             INSERT INTO threads VALUES ('codex-history', '{}', 20, NULL, NULL);",
            workspace.replace('\'', "''")
        ))
        .unwrap();
    drop(connection);

    let deleted = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "deleted-launch",
        )
        .await
        .unwrap();
    let deleted_owner = owner("workspace-1", &deleted);
    let mut deleted_terminal = host
        .terminal()
        .subscribe("history-deleted".to_string(), deleted_owner.clone())
        .await
        .unwrap();
    receive_until(
        &mut deleted_terminal,
        fixture_label(AcceptanceProvider::Claude),
    )
    .await;
    emit_session_start(
        &host,
        &mut deleted_terminal,
        &deleted_owner,
        &deleted,
        "claude-history",
    )
    // When
    .await;
    // Then
    assert_eq!(
        host.archive(&deleted, "deleted-archive").await.unwrap(),
        AcceptanceArchiveOutcome::Archived
    );
    host.delete(&deleted, "deleted-delete").await.unwrap();

    let candidates = host.list_history(&workspace, 10).await.unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|candidate| {
        candidate.provider == AcceptanceProvider::Claude
            && candidate.provider_session_id == "claude-history"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.provider == AcceptanceProvider::Codex
            && candidate.provider_session_id == "codex-history"
            && candidate.updated_at_ms == 20_000
    }));

    let resumed_claude = host
        .resume_history(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            "claude-history",
            24,
            80,
            "claude-history-resume",
        )
        .await
        .unwrap();
    assert_ne!(resumed_claude, deleted);
    let resumed_claude = host.get(&resumed_claude).await.unwrap().unwrap();
    assert_eq!(
        resumed_claude.lifecycle,
        AcceptanceAgentSessionLifecycle::Open
    );
    assert_eq!(
        resumed_claude.provider_session_id.as_deref(),
        Some("claude-history")
    );

    let resumed_codex = host
        .resume_history(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Codex,
            "codex-history",
            24,
            80,
            "codex-history-resume",
        )
        .await
        .unwrap();
    assert_eq!(
        host.get(&resumed_codex)
            .await
            .unwrap()
            .unwrap()
            .provider_session_id
            .as_deref(),
        Some("codex-history")
    );
    assert!(host.list_history(&workspace, 10).await.unwrap().is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider_id不明でも確認なしで停止してarchiveし明示deleteできる() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 4);
    let session_id = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "unknown-id-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner("workspace-1", &session_id);

    assert!(host.delete(&session_id, "open-delete").await.is_err());
    assert_eq!(
        host.archive(&session_id, "unknown-id-archive")
            .await
            .unwrap(),
        AcceptanceArchiveOutcome::Archived
    );
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Archived
    );
    assert!(host.terminal().get(terminal_owner.clone()).is_err());

    host.delete(&session_id, "unknown-id-delete").await.unwrap();
    assert!(host.get(&session_id).await.unwrap().is_none());
    assert!(host.terminal().get(terminal_owner).is_err());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider実行fileが無くてもrestoreできresumeだけ失敗する() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 4);
    let session_id = host
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "restore-failure-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner("workspace-1", &session_id);
    let mut terminal = host
        .terminal()
        .subscribe("restore-failure".to_string(), terminal_owner.clone())
        .await
        .unwrap();
    receive_until(&mut terminal, fixture_label(AcceptanceProvider::Claude)).await;
    emit_session_start(
        &host,
        &mut terminal,
        &terminal_owner,
        &session_id,
        "restore-failure-provider-id",
    )
    .await;
    assert_eq!(
        host.archive(&session_id, "restore-failure-archive")
            .await
            .unwrap(),
        AcceptanceArchiveOutcome::Archived
    );
    std::fs::remove_file(root.path().join("bin/claude-fixture")).unwrap();

    assert_eq!(
        host.restore(&session_id, 24, 80, "restore-without-provider")
            .await
            .unwrap(),
        session_id
    );
    assert!(host.terminal().get(terminal_owner).is_err());
    assert!(host.resume_session_node(&session_id).await.is_err());
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Paused
    );
    host.shutdown().await.unwrap();
}
