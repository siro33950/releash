#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_tui_fixture::{fixture_process_shell_command, FixtureLifecycleCommand, FixturePlan};
use releash_lib::agent_session_tui_acceptance::{
    product_agent_session_invoke_handler, AcceptanceAgentSessionLifecycle,
    AcceptanceAgentSessionOrigin, AcceptanceArchiveOutcome, AcceptanceHookWarning,
    AcceptanceOpenOutcome, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
    AgentSessionTuiAcceptanceHost as AgentSessionTuiAcceptanceComposition,
};
use releash_lib::terminal_surface::{
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1, TerminalSurfaceWireAttachment,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchDistribution {
    count: usize,
    p50: f64,
    p95: f64,
    max: f64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeterministicLaunchPerformanceReport {
    schema_version: u8,
    source: &'static str,
    provider: &'static str,
    warm_runs: usize,
    total_to_first_provider_byte_ms: LaunchDistribution,
    phase_ms: std::collections::BTreeMap<String, LaunchDistribution>,
}

fn launch_distribution(mut samples: Vec<f64>) -> LaunchDistribution {
    samples.sort_by(f64::total_cmp);
    let middle = samples.len() / 2;
    let p50 = if samples.len().is_multiple_of(2) {
        (samples[middle - 1] + samples[middle]) / 2.0
    } else {
        samples[middle]
    };
    LaunchDistribution {
        count: samples.len(),
        p50,
        p95: samples[(samples.len() * 95).div_ceil(100) - 1],
        max: *samples.last().expect("non-empty launch samples"),
    }
}

fn write_actual_provider_launch_report(
    provider: &'static str,
    total_to_first_visible_ms: f64,
    samples: Vec<
        releash_lib::agent_session_tui_acceptance::AcceptanceTerminalLaunchPerformanceSample,
    >,
) {
    let phase_ms = samples
        .into_iter()
        .map(|sample| (sample.phase, sample.duration_ms))
        .collect::<std::collections::BTreeMap<_, _>>();
    let report = serde_json::json!({
        "schemaVersion": 1,
        "source": "provider-observation",
        "provider": provider,
        "totalToFirstVisibleMs": total_to_first_visible_ms,
        "phaseMs": phase_ms,
    });
    let report_json = serde_json::to_string_pretty(&report).unwrap();
    println!("{report_json}");
    if let Some(path) = std::env::var_os("RELEASH_ACTUAL_PROVIDER_LAUNCH_REPORT_PATH") {
        std::fs::write(path, format!("{report_json}\n")).unwrap();
    }
}

static TERMINAL_LAUNCH_PERFORMANCE_GATE_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionListPage {
    items: Vec<releash_lib::agent_session_tui_acceptance::AcceptanceAgentSession>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionHistoryPage {
    items: Vec<releash_lib::agent_session_tui_acceptance::AcceptanceHistoryCandidate>,
}

struct AgentSessionTuiAcceptanceHost {
    composition: AgentSessionTuiAcceptanceComposition<tauri::test::MockRuntime>,
}

impl AgentSessionTuiAcceptanceHost {
    fn start(config: AgentSessionTuiAcceptanceConfig) -> Result<Self, String> {
        let app = tauri::test::mock_builder()
            .invoke_handler(product_agent_session_invoke_handler())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| error.to_string())?;
        AgentSessionTuiAcceptanceComposition::start(config, app)
            .map(|composition| Self { composition })
    }

    fn invoke<T: DeserializeOwned>(
        &self,
        command: &str,
        body: serde_json::Value,
    ) -> Result<T, String> {
        tauri::test::get_ipc_response(
            self.composition.window(),
            tauri::webview::InvokeRequest {
                cmd: command.to_string(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .expect("valid Tauri invoke URL"),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .map_err(|error| error.to_string())?
        .deserialize::<T>()
        .map_err(|error| error.to_string())
    }

    fn terminal(&self) -> &releash_lib::terminal_surface::TerminalSurfaceRuntime {
        self.composition.terminal()
    }

    fn stop_local_api(&self) -> Result<(), String> {
        self.composition.stop_local_api()
    }

    fn restart_local_api(&self) -> Result<(), String> {
        self.composition.restart_local_api()
    }

    fn hook_warnings(&self) -> Result<Vec<AcceptanceHookWarning>, String> {
        self.invoke("list_provider_hook_health_warnings", serde_json::json!({}))
    }

    fn hook_health_marker_contents(&self) -> Result<Vec<String>, String> {
        self.composition.hook_health_marker_contents()
    }

    fn start_terminal_launch_performance_collection(&self) {
        self.composition
            .start_terminal_launch_performance_collection();
    }

    fn take_terminal_launch_performance_samples(
        &self,
    ) -> Vec<releash_lib::agent_session_tui_acceptance::AcceptanceTerminalLaunchPerformanceSample>
    {
        self.composition.take_terminal_launch_performance_samples()
    }

    fn available_providers(&self) -> Vec<AcceptanceProvider> {
        self.invoke(
            "list_available_provider_agent_session_providers",
            serde_json::json!({}),
        )
        .expect("available Provider command")
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
        self.invoke(
            "create_provider_agent_session",
            serde_json::json!({
                "workspaceIdentity": workspace_identity,
                "worktreePath": worktree_path,
                "provider": provider_name(provider),
                "rows": rows,
                "cols": cols,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn launch_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
        workflow_execution_id: &str,
        node_execution_id: &str,
    ) -> Result<String, String> {
        self.composition
            .launch_workflow(
                worktree_path,
                provider,
                workflow_execution_id,
                node_execution_id,
            )
            .await
    }

    async fn dispatch_initial_instruction(
        &self,
        agent_session_id: &str,
        node_execution_id: &str,
        instruction: &str,
    ) -> Result<(), String> {
        self.composition
            .dispatch_initial_instruction(agent_session_id, node_execution_id, instruction)
            .await
    }

    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<releash_lib::agent_session_tui_acceptance::AcceptanceAgentSession>, String>
    {
        self.invoke(
            "get_provider_agent_session",
            serde_json::json!({ "agentSessionId": agent_session_id }),
        )
    }

    async fn list(
        &self,
        workspace_identity: &str,
    ) -> Result<Vec<releash_lib::agent_session_tui_acceptance::AcceptanceAgentSession>, String>
    {
        self.invoke::<AgentSessionListPage>(
            "list_provider_agent_sessions",
            serde_json::json!({
                "workspaceIdentity": workspace_identity,
                "lifecycle": null,
                "origin": null,
                "limit": 100,
                "afterSessionId": null,
            }),
        )
        .map(|page| page.items)
    }

    async fn list_history(
        &self,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<releash_lib::agent_session_tui_acceptance::AcceptanceHistoryCandidate>, String>
    {
        self.invoke::<AgentSessionHistoryPage>(
            "list_provider_agent_session_history",
            serde_json::json!({
                "worktreePath": worktree_path,
                "limit": limit,
                "after": null,
            }),
        )
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
        self.invoke(
            "resume_provider_agent_session_history_candidate",
            serde_json::json!({
                "workspaceIdentity": workspace_identity,
                "worktreePath": worktree_path,
                "provider": provider_name(provider),
                "providerSessionId": provider_session_id,
                "rows": rows,
                "cols": cols,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn archive(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AcceptanceArchiveOutcome, String> {
        self.invoke(
            "archive_provider_agent_session",
            serde_json::json!({
                "agentSessionId": agent_session_id,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn open(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AcceptanceOpenOutcome, String> {
        self.invoke_open_command(
            "open_provider_agent_session",
            agent_session_id,
            rows,
            cols,
            caller_request_id,
        )
    }

    async fn restore(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AcceptanceOpenOutcome, String> {
        self.invoke_open_command(
            "restore_provider_agent_session",
            agent_session_id,
            rows,
            cols,
            caller_request_id,
        )
    }

    async fn resume(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AcceptanceOpenOutcome, String> {
        self.invoke_open_command(
            "resume_provider_agent_session",
            agent_session_id,
            rows,
            cols,
            caller_request_id,
        )
    }

    fn invoke_open_command(
        &self,
        command: &str,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AcceptanceOpenOutcome, String> {
        self.invoke(
            command,
            serde_json::json!({
                "agentSessionId": agent_session_id,
                "rows": rows,
                "cols": cols,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn delete(&self, agent_session_id: &str, caller_request_id: &str) -> Result<(), String> {
        self.invoke(
            "delete_provider_agent_session",
            serde_json::json!({
                "agentSessionId": agent_session_id,
                "callerRequestId": caller_request_id,
            }),
        )
    }

    async fn confirm_archive_fallback_delete(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), String> {
        self.invoke(
            "confirm_provider_agent_session_archive_delete",
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
    std::fs::write(&executable, format!("#!/bin/sh\n{command}\n")).unwrap();
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
        claude_executable: claude,
        codex_executable: codex,
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

fn fixture_label(provider: AcceptanceProvider) -> &'static str {
    match provider {
        AcceptanceProvider::Claude => "claude-fixture",
        AcceptanceProvider::Codex => "codex-fixture",
    }
}

fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}

fn resolve_executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} executable is required"))
}

fn install_actual_provider_host(root: &Path) -> AgentSessionTuiAcceptanceHost {
    let bin = root.join("actual-bin");
    let claude_config_dir = root.join("claude-home");
    let codex_home = root.join("codex-home");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&claude_config_dir).unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();

    let releash_dev = bin.join("releash-dev");
    std::fs::write(
        &releash_dev,
        format!(
            "#!/bin/sh\nexec \"{}\" \"$@\"\n",
            env!("CARGO_BIN_EXE_releash")
        ),
    )
    .unwrap();
    set_executable(&releash_dev);

    let claude = bin.join("claude-actual");
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\nexport PATH=\"{}:$PATH\"\nexec \"{}\" \"$@\"\n",
            bin.display(),
            resolve_executable("claude").display(),
        ),
    )
    .unwrap();
    set_executable(&claude);

    let source_auth =
        PathBuf::from(std::env::var_os("HOME").expect("HOME is required")).join(".codex/auth.json");
    std::fs::copy(&source_auth, codex_home.join("auth.json")).unwrap_or_else(|error| {
        panic!(
            "copy installed Codex auth from {}: {error}",
            source_auth.display()
        )
    });
    let codex = bin.join("codex-actual");
    std::fs::write(
        &codex,
        format!(
            "#!/bin/sh\nexport PATH=\"{}:$PATH\"\nexport CODEX_HOME=\"{}\"\nexec \"{}\" \"$@\"\n",
            bin.display(),
            codex_home.display(),
            resolve_executable("codex").display(),
        ),
    )
    .unwrap();
    set_executable(&codex);

    AgentSessionTuiAcceptanceHost::start(AgentSessionTuiAcceptanceConfig {
        data_dir: root.join("releash-data"),
        claude_executable: claude,
        codex_executable: codex,
        claude_config_dir,
        codex_home,
    })
    .unwrap()
}

fn normalized_terminal_output(bytes: &[u8]) -> String {
    #[derive(Clone, Copy)]
    enum EscapeState {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = EscapeState::Text;
    let mut normalized = String::new();
    for &byte in bytes {
        state = match state {
            EscapeState::Text if byte == 0x1b => EscapeState::Escape,
            EscapeState::Text => {
                if byte.is_ascii_alphanumeric() {
                    normalized.push(char::from(byte).to_ascii_lowercase());
                }
                EscapeState::Text
            }
            EscapeState::Escape if byte == b'[' => EscapeState::Csi,
            EscapeState::Escape if byte == b']' => EscapeState::Osc,
            EscapeState::Escape => EscapeState::Text,
            EscapeState::Csi if (0x40..=0x7e).contains(&byte) => EscapeState::Text,
            EscapeState::Csi => EscapeState::Csi,
            EscapeState::Osc if byte == 0x07 => EscapeState::Text,
            EscapeState::Osc if byte == 0x1b => EscapeState::OscEscape,
            EscapeState::Osc => EscapeState::Osc,
            EscapeState::OscEscape if byte == b'\\' => EscapeState::Text,
            EscapeState::OscEscape => EscapeState::Osc,
        };
    }
    normalized
}

async fn receive_until_any_normalized(
    attachment: &mut TerminalSurfaceWireAttachment,
    output: &mut Vec<u8>,
    needles: &[&str],
    timeout: Duration,
) -> usize {
    let needles = needles
        .iter()
        .map(|needle| needle.to_ascii_lowercase())
        .collect::<Vec<_>>();
    tokio::time::timeout(timeout, async {
        loop {
            match attachment.next().await.expect("Terminal Surface stream") {
                TerminalSurfaceStreamItemV1::Snapshot { surface } => {
                    output.extend_from_slice(surface.terminal_surface.replay.as_bytes())
                }
                TerminalSurfaceStreamItemV1::Output { data, .. } => {
                    output.extend_from_slice(data.as_bytes())
                }
                _ => {}
            }
            let normalized = normalized_terminal_output(output);
            if let Some(index) = needles
                .iter()
                .position(|needle| normalized.contains(needle))
            {
                return index;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for {needles:?}: {}",
            String::from_utf8_lossy(output)
        )
    })
}

async fn wait_for_provider_session_id(
    host: &AgentSessionTuiAcceptanceHost,
    agent_session_id: &str,
    terminal: &mut TerminalSurfaceWireAttachment,
    output: &mut Vec<u8>,
) -> String {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(provider_session_id) = host
                .get(agent_session_id)
                .await
                .unwrap()
                .and_then(|session| session.provider_session_id)
            {
                return provider_session_id;
            }
            if let Ok(item) = tokio::time::timeout(Duration::from_millis(50), terminal.next()).await
            {
                match item.expect("Terminal Surface stream") {
                    TerminalSurfaceStreamItemV1::Snapshot { surface } => {
                        output.extend_from_slice(surface.terminal_surface.replay.as_bytes());
                    }
                    TerminalSurfaceStreamItemV1::Output { data, .. } => {
                        output.extend_from_slice(data.as_bytes());
                    }
                    _ => {}
                }
            }
        }
    })
    .await;
    match result {
        Ok(provider_session_id) => provider_session_id,
        Err(_) => panic!(
            "timed out waiting for root Provider SessionStart; terminal={}; warnings={:?}; markers={:?}",
            String::from_utf8_lossy(output),
            host.hook_warnings(),
            host.hook_health_marker_contents(),
        ),
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
        let terminal_owner = owner("workspace-1", &session_id);
        let mut attached = host
            .terminal()
            .attach("acceptance-first".to_string(), terminal_owner.clone())
            .unwrap();
        receive_until(&mut attached, fixture_label(provider)).await;
        host.terminal()
            .write(terminal_owner.clone(), "permission-approved\r")
            .unwrap();
        receive_until(&mut attached, "received-0:permission-approved").await;
        drop(attached);

        let mut reloaded = host
            .terminal()
            .attach("acceptance-reload".to_string(), terminal_owner.clone())
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
            AcceptanceOpenOutcome::Restored
        );
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
            "workflow-execution-1",
            "node-execution-1",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&workspace, &session_id);
    let mut attached = host
        .terminal()
        .attach("workflow-session".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut attached, fixture_label(AcceptanceProvider::Claude)).await;

    host.dispatch_initial_instruction(
        &session_id,
        "node-execution-1",
        "system policy\n\nimplement once",
    )
    .await
    .unwrap();
    receive_until(
        &mut attached,
        "received-0:system policy\\n\\nimplement once",
    )
    .await;
    host.dispatch_initial_instruction(&session_id, "node-execution-1", "must not repeat")
        .await
        .unwrap();

    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().origin,
        AcceptanceAgentSessionOrigin::WorkflowNode {
            workflow_execution_id: "workflow-execution-1".to_string(),
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
        .attach("multi-turn-session".to_string(), terminal_owner.clone())
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
async fn test_atui_030_hook配送失敗をapp警告にしprocessを止めず後続成功で解除する() {
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
        .attach("hook-health-session".to_string(), terminal_owner.clone())
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
        1,
        "hook health markers: {:?}",
        host.hook_health_marker_contents().unwrap()
    );
    assert_eq!(warnings[0].provider, AcceptanceProvider::Claude);
    assert_eq!(warnings[0].reason, "local_api_unavailable");
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
        .attach("duplicate-first".to_string(), first_owner.clone())
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
        .attach("duplicate-second".to_string(), second_owner.clone())
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
        claude_executable: unavailable_root.path().join("missing-claude"),
        codex_executable: unavailable_root.path().join("missing-codex"),
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
    assert!(unavailable.list("workspace-1").await.unwrap().is_empty());
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
        .attach("paused-session".to_string(), paused_owner.clone())
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
    assert_eq!(
        host.resume(&paused, 24, 80, "resume").await.unwrap(),
        AcceptanceOpenOutcome::Resumed
    );

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
        .attach("gc-session".to_string(), gc_owner.clone())
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
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, updated_at INTEGER);\
             INSERT INTO threads VALUES ('codex-history', '{}', 20);",
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
        .attach("history-deleted".to_string(), deleted_owner.clone())
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
    .await;
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
async fn test_atui_030_app再起動後の自動resume失敗はpausedとなり手動resumeで復帰する() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (first, claude_home, codex_home) = host(root.path(), 4);
    let session_id = first
        .launch_standalone(
            "workspace-1",
            &workspace,
            AcceptanceProvider::Claude,
            24,
            80,
            "restart-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner("workspace-1", &session_id);
    let mut terminal = first
        .terminal()
        .attach("restart-session".to_string(), terminal_owner.clone())
        .unwrap();
    receive_until(&mut terminal, fixture_label(AcceptanceProvider::Claude)).await;
    emit_session_start(
        &first,
        &mut terminal,
        &terminal_owner,
        &session_id,
        "restart-provider-id",
    )
    .await;
    first.shutdown().await.unwrap();

    let claude_executable = root.path().join("bin/claude-fixture");
    std::fs::remove_file(&claude_executable).unwrap();
    let restarted = AgentSessionTuiAcceptanceHost::start(AgentSessionTuiAcceptanceConfig {
        data_dir: root.path().join("releash-data"),
        claude_executable: claude_executable.clone(),
        codex_executable: root.path().join("bin/codex-fixture"),
        claude_config_dir: claude_home,
        codex_home,
    })
    .unwrap();
    assert_eq!(
        restarted.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Open,
        "clean app shutdown must not be persisted as a Provider process exit"
    );
    assert_eq!(
        restarted
            .open(&session_id, 24, 80, "restart-open")
            .await
            .unwrap(),
        AcceptanceOpenOutcome::Paused
    );
    assert_eq!(
        restarted.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Paused
    );

    install_fixture_executable(
        &root.path().join("bin"),
        "claude-fixture",
        AcceptanceProvider::Claude,
        4,
    );
    assert_eq!(
        restarted
            .resume(&session_id, 24, 80, "restart-manual-resume")
            .await
            .unwrap(),
        AcceptanceOpenOutcome::Resumed
    );
    assert_eq!(
        restarted.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Open
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider_id不明のarchiveは確認まで保持し確認後に縮退deleteする() {
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
        AcceptanceArchiveOutcome::DeleteConfirmationRequired
    );
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

    host.confirm_archive_fallback_delete(&session_id, "unknown-id-confirm")
        .await
        .unwrap();
    assert!(host.get(&session_id).await.unwrap().is_none());
    assert!(host.terminal().get(terminal_owner).is_err());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_restore失敗はarchivedを維持する() {
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
        .attach("restore-failure".to_string(), terminal_owner.clone())
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

    assert!(host
        .restore(&session_id, 24, 80, "restore-failure")
        .await
        .is_err());
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Archived
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "dedicated 30 warm-run Terminal launch performance harness"]
async fn test_terminal_launch_deterministic_fixture_reports_30_warm_runs() {
    let _gate = TERMINAL_LAUNCH_PERFORMANCE_GATE_LOCK.lock().await;
    const WARM_RUNS: usize = 30;
    const PHASES: &[&str] = &[
        "terminal.launch.command_ingress",
        "terminal.launch.availability_and_lock",
        "terminal.launch.durable_create_commit",
        "terminal.launch.launch_file_materialize",
        "terminal.launch.checkpoint_lookup",
        "terminal.launch.child_environment",
        "terminal.launch.pty_open_and_spawn",
        "terminal.launch.output_reader_ready",
        "terminal.launch.first_provider_byte",
    ];

    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let (host, _, _) = host(root.path(), 0);

    let warmup_id = host
        .launch_standalone(
            "performance-workspace",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "performance-warmup",
        )
        .await
        .unwrap();
    let mut warmup = host
        .terminal()
        .attach(
            "performance-warmup".to_string(),
            owner("performance-workspace", &warmup_id),
        )
        .unwrap();
    receive_until(&mut warmup, fixture_label(AcceptanceProvider::Codex)).await;
    drop(warmup);

    host.start_terminal_launch_performance_collection();
    let mut totals = Vec::with_capacity(WARM_RUNS);
    for index in 0..WARM_RUNS {
        let started_at = std::time::Instant::now();
        let session_id = host
            .launch_standalone(
                "performance-workspace",
                &workspace,
                AcceptanceProvider::Codex,
                24,
                80,
                &format!("performance-run-{index}"),
            )
            .await
            .unwrap();
        let mut terminal = host
            .terminal()
            .attach(
                format!("performance-run-{index}"),
                owner("performance-workspace", &session_id),
            )
            .unwrap();
        receive_until(&mut terminal, fixture_label(AcceptanceProvider::Codex)).await;
        totals.push(started_at.elapsed().as_secs_f64() * 1_000.0);
    }

    let samples = host.take_terminal_launch_performance_samples();
    let mut samples_by_phase = std::collections::BTreeMap::<String, Vec<f64>>::new();
    for sample in samples {
        samples_by_phase
            .entry(sample.phase)
            .or_default()
            .push(sample.duration_ms);
    }
    assert_eq!(samples_by_phase.len(), PHASES.len());
    let phase_ms = PHASES
        .iter()
        .map(|phase| {
            let samples = samples_by_phase
                .remove(*phase)
                .unwrap_or_else(|| panic!("missing launch phase {phase}"));
            assert_eq!(samples.len(), WARM_RUNS, "phase {phase}");
            ((*phase).to_string(), launch_distribution(samples))
        })
        .collect();
    assert!(samples_by_phase.is_empty());
    let report = DeterministicLaunchPerformanceReport {
        schema_version: 1,
        source: "deterministic-fixture",
        provider: "fixture",
        warm_runs: WARM_RUNS,
        total_to_first_provider_byte_ms: launch_distribution(totals),
        phase_ms,
    };
    let report_json = serde_json::to_string_pretty(&report).unwrap();
    println!("{report_json}");
    if let Some(path) = std::env::var_os("RELEASH_TERMINAL_LAUNCH_REPORT_PATH") {
        std::fs::write(path, format!("{report_json}\n")).unwrap();
    }

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Claude Code production AgentSession gate"]
async fn test_atui_030_実claudeをproduction経路で起動しroot_session_startからarchiveする() {
    let _gate = TERMINAL_LAUNCH_PERFORMANCE_GATE_LOCK.lock().await;
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("claude-worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let host = install_actual_provider_host(root.path());
    host.start_terminal_launch_performance_collection();
    let launch_started_at = std::time::Instant::now();
    let session_id = host
        .launch_standalone(
            &workspace,
            &workspace,
            AcceptanceProvider::Claude,
            30,
            100,
            "actual-claude-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&workspace, &session_id);
    let mut terminal = host
        .terminal()
        .attach("actual-claude".to_string(), terminal_owner.clone())
        .unwrap();
    let mut output = Vec::new();

    receive_until_any_normalized(
        &mut terminal,
        &mut output,
        &["yesitrustthisfolder"],
        Duration::from_secs(30),
    )
    .await;
    write_actual_provider_launch_report(
        "claude",
        launch_started_at.elapsed().as_secs_f64() * 1_000.0,
        host.take_terminal_launch_performance_samples(),
    );
    host.terminal().write(terminal_owner.clone(), "\r").unwrap();
    let provider_session_id =
        wait_for_provider_session_id(&host, &session_id, &mut terminal, &mut output).await;
    assert!(!provider_session_id.is_empty());
    assert!(host
        .hook_warnings()
        .unwrap()
        .iter()
        .all(|warning| warning.provider != AcceptanceProvider::Claude));
    assert_eq!(
        host.archive(&session_id, "actual-claude-archive")
            .await
            .unwrap(),
        AcceptanceArchiveOutcome::Archived
    );
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Archived
    );
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Codex CLI production AgentSession gate"]
async fn test_atui_030_実codexをproduction経路でtrustしroot_session_startからarchiveする() {
    let _gate = TERMINAL_LAUNCH_PERFORMANCE_GATE_LOCK.lock().await;
    let root = tempfile::TempDir::new().unwrap();
    let workspace = root.path().join("codex-worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
    let host = install_actual_provider_host(root.path());

    host.start_terminal_launch_performance_collection();
    let launch_started_at = std::time::Instant::now();
    let untrusted_session_id = host
        .launch_standalone(
            &workspace,
            &workspace,
            AcceptanceProvider::Codex,
            30,
            100,
            "actual-codex-untrusted-launch",
        )
        .await
        .unwrap();
    let untrusted_owner = owner(&workspace, &untrusted_session_id);
    let mut untrusted_terminal = host
        .terminal()
        .attach(
            "actual-codex-untrusted".to_string(),
            untrusted_owner.clone(),
        )
        .unwrap();
    let mut untrusted_output = Vec::new();
    receive_until_any_normalized(
        &mut untrusted_terminal,
        &mut untrusted_output,
        &["doyoutrustthecontentsofthisdirectory"],
        Duration::from_secs(30),
    )
    .await;
    write_actual_provider_launch_report(
        "codex",
        launch_started_at.elapsed().as_secs_f64() * 1_000.0,
        host.take_terminal_launch_performance_samples(),
    );
    assert!(host.hook_warnings().unwrap().iter().any(|warning| {
        warning.provider == AcceptanceProvider::Codex
            && warning.reason == "codex_hook_delivery_unconfirmed"
    }));
    host.terminal()
        .write(untrusted_owner.clone(), "\r")
        .unwrap();
    receive_until_any_normalized(
        &mut untrusted_terminal,
        &mut untrusted_output,
        &["hooksneedreview"],
        Duration::from_secs(30),
    )
    .await;
    host.terminal()
        .write(untrusted_owner.clone(), "\x1b[B\r")
        .unwrap();
    receive_until_any_normalized(
        &mut untrusted_terminal,
        &mut untrusted_output,
        &["pressentertoconfirm"],
        Duration::from_secs(30),
    )
    .await;
    host.terminal()
        .write(untrusted_owner.clone(), "\r")
        .unwrap();
    receive_until_any_normalized(
        &mut untrusted_terminal,
        &mut untrusted_output,
        &["modelgpt56solmodeltochangedirectory"],
        Duration::from_secs(30),
    )
    .await;
    assert!(host
        .get(&untrusted_session_id)
        .await
        .unwrap()
        .unwrap()
        .provider_session_id
        .is_none());
    assert_eq!(
        host.archive(&untrusted_session_id, "actual-codex-untrusted-archive")
            .await
            .unwrap(),
        AcceptanceArchiveOutcome::DeleteConfirmationRequired
    );
    host.confirm_archive_fallback_delete(&untrusted_session_id, "actual-codex-untrusted-delete")
        .await
        .unwrap();

    let session_id = host
        .launch_standalone(
            &workspace,
            &workspace,
            AcceptanceProvider::Codex,
            30,
            100,
            "actual-codex-trusted-launch",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&workspace, &session_id);
    let mut terminal = host
        .terminal()
        .attach("actual-codex-trusted".to_string(), terminal_owner.clone())
        .unwrap();
    let mut output = Vec::new();
    let startup = receive_until_any_normalized(
        &mut terminal,
        &mut output,
        &[
            "skipuntilnextversion",
            "openaicodexv01450",
            "modelgpt56solmodeltochangedirectory",
        ],
        Duration::from_secs(30),
    )
    .await;
    if startup == 0 {
        host.terminal()
            .write(terminal_owner.clone(), "\x1b[B\r")
            .unwrap();
    }
    receive_until_any_normalized(
        &mut terminal,
        &mut output,
        &["modelgpt56solmodeltochangedirectory"],
        Duration::from_secs(30),
    )
    .await;
    host.terminal()
        .write(terminal_owner.clone(), "Reply with exactly ok.")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    host.terminal().write(terminal_owner.clone(), "\r").unwrap();
    let provider_session_id =
        wait_for_provider_session_id(&host, &session_id, &mut terminal, &mut output).await;
    assert!(!provider_session_id.is_empty());
    assert!(host
        .hook_warnings()
        .unwrap()
        .iter()
        .all(|warning| warning.provider != AcceptanceProvider::Codex));
    assert_eq!(
        host.archive(&session_id, "actual-codex-archive")
            .await
            .unwrap(),
        AcceptanceArchiveOutcome::Archived
    );
    assert_eq!(
        host.get(&session_id).await.unwrap().unwrap().lifecycle,
        AcceptanceAgentSessionLifecycle::Archived
    );
    host.shutdown().await.unwrap();
}
