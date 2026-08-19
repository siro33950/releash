#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_tui_fixture::{fixture_process_shell_command, FixtureLifecycleCommand, FixturePlan};
use releash_lib::agent_session_tui_acceptance::{
    product_agent_session_invoke_handler, AcceptanceAgentSessionLifecycle,
    AcceptanceAgentSessionTreeParent, AcceptanceArchiveOutcome, AcceptanceHookWarning,
    AcceptanceOpenOutcome, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
    AgentSessionTuiAcceptanceHost as AgentSessionTuiAcceptanceComposition,
};
use releash_lib::terminal_surface::{
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1, TerminalSurfaceWireAttachment,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;

fn read_repository_file(relative_path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative_path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read repository file {}: {error}", path.display()))
}

#[test]
fn atui_050_production_composition_does_not_require_legacy_agent_runtime() {
    let composition = include_str!("../src/lib.rs");

    for removed in [
        "AgentBackendRegistry",
        "AgentSessionRuntimeUsecase",
        "compose_agent_session_runtime",
    ] {
        assert!(
            !composition.contains(removed),
            "legacy Agent runtime remains in production composition: {removed}"
        );
    }
}

#[test]
fn atui_050_production_agent_session_boundary_uses_only_canonical_names() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let roots = [manifest_dir.join("src"), manifest_dir.join("../src")];
    let mut violations = Vec::new();
    for root in roots {
        collect_temporary_agent_session_names(&root, &mut violations);
    }
    assert!(
        violations.is_empty(),
        "temporary ProviderAgentSession names remain:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_production_boundary_cannot_write_or_read_legacy_message_projections() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    collect_legacy_message_projection_symbols(&manifest_dir.join("src"), &mut violations);
    assert!(
        violations.is_empty(),
        "legacy Message projection symbols remain in production:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_application_shutdown_is_independent_of_legacy_agent_operations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    collect_removed_symbols(
        &manifest_dir.join("src"),
        &[
            "usecase::agent_session::operation",
            "agent_session_v1",
            "ShutdownTargetKindRecord::AgentSession",
            "shutdown_target_agent_session_lifecycle",
            "SessionLifecycleOperation",
        ],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "application shutdown still depends on legacy Agent operation ownership:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_provider_selection_has_no_implicit_default_or_model_config() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    collect_removed_symbols(
        &manifest_dir.join("src"),
        &[
            "default_agent_backend",
            "pub default: Option<String>",
            "pub models: Vec<String>",
        ],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "implicit Provider default or legacy model configuration remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_canonical_docs_define_agent_session_tui_state_ownership() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let canonical_docs = [
        "docs/architecture/README.md",
        "docs/architecture/GLOSSARY.md",
        "docs/architecture/INFRASTRUCTURE.md",
        "docs/architecture/TEST.md",
        "docs/domain-model/current-state.md",
        "docs/workflow-engine-model-boundary.md",
        "docs/workflow-engine-evolution-plan.md",
        "docs/workflow-yaml-syntax.md",
        "docs/spec/README.md",
    ]
    .into_iter()
    .map(read_repository_file)
    .collect::<Vec<_>>()
    .join("\n");

    for required in [
        "canonical語は `AgentSession`",
        "Releashは `Turn`、`Message`、`MessagePart`、`PermissionRequest` を所有しない",
        "Provider CLI / transcriptがconversationの正本",
        "`AgentSession`はlifecycleとTerminal ownershipを所有する",
        "Terminal Surfaceは`Workspace`または`AgentSession`に所有される",
        "`NodeExecution`は`AgentSession`を参照するが所有しない",
        "Workflow completionとAgentSession lifecycleは独立する",
        "Submit / Stop / Approval / ArtifactはWorkflowが所有する",
        "Provider lifecycleとProvider availabilityは別の境界である",
        "旧Agent GUI specは現行正本ではない",
    ] {
        assert!(
            canonical_docs.contains(required),
            "canonical docs do not state required ownership: {required}"
        );
    }

    assert!(
        !repository_root
            .join("docs/agent-model-selector-direction.md")
            .exists(),
        "obsolete Agent GUI model selector document remains canonical"
    );
}

#[test]
fn atui_050_canonical_docs_reject_removed_agent_runtime_contracts() {
    let architecture = read_repository_file("docs/architecture/README.md");
    let infrastructure = read_repository_file("docs/architecture/INFRASTRUCTURE.md");
    let test_contract = read_repository_file("docs/architecture/TEST.md");
    let workflow_yaml = read_repository_file("docs/workflow-yaml-syntax.md");

    assert!(
        !architecture.contains("agent_session/                  # Agent CLI のプロセスと wire"),
        "architecture index still describes the removed Agent wire infrastructure"
    );
    assert!(
        !infrastructure.contains("wire ↔ ドメインイベントの変換")
            && !infrastructure.contains("agent_session/                 # Agent CLI"),
        "infrastructure contract still describes the removed Agent wire runtime"
    );
    assert!(
        !test_contract.contains("Agent SDK"),
        "test contract still describes the removed Agent SDK runtime"
    );
    for removed in [
        "model: claude-opus-5",
        "permission: ask",
        "String request、permission mode を受け取る",
    ] {
        assert!(
            !workflow_yaml.contains(removed),
            "workflow YAML contract still exposes removed Releash-owned Agent configuration: {removed}"
        );
    }
    assert!(
        workflow_yaml.contains("provider: codex")
            && workflow_yaml.contains("`provider`: 必須。`claude` または `codex`"),
        "workflow YAML contract does not require an explicit Provider"
    );
}

#[test]
fn atui_050_legacy_agent_conversation_runtime_is_physically_removed() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let obsolete_paths = [
        "src/domain/agent_session/entities",
        "src/domain/agent_session/value_objects",
        "src/domain/agent_session/aggregates/session",
        "src/domain/agent_session/aggregates/backend_recovery_attempt.rs",
        "src/domain/agent_session/aggregates/backend_recovery_projection.rs",
        "src/domain/agent_session/aggregates/provider_establishment.rs",
        "src/domain/agent_session/aggregates/runtime_admission.rs",
        "src/domain/agent_session/aggregates/runtime_permission.rs",
        "src/domain/agent_session/aggregates/runtime_progress.rs",
        "src/domain/agent_session/aggregates/runtime_queue.rs",
        "src/domain/agent_session/aggregates/runtime_stream_buffer.rs",
        "src/domain/agent_session/aggregates/runtime_stream_retries.rs",
        "src/domain/agent_session/aggregates/runtime_stream_sequence.rs",
        "src/domain/agent_session/aggregates/runtime_streaming_delivery.rs",
        "src/domain/agent_session/aggregates/runtime_turn.rs",
        "src/domain/agent_session/aggregates/send_dispatches.rs",
        "src/domain/agent_session/events.rs",
        "src/domain/agent_session/gateway.rs",
        "src/domain/agent_session/storage.rs",
        "src/adaptor/gateway/local_event_store/agent_session_codec.rs",
        "src/infrastructure/platform/focus_tracker.rs",
        "src/usecase/notification/dto.rs",
    ];
    let remaining_paths = obsolete_paths
        .into_iter()
        .filter(|relative_path| manifest_dir.join(relative_path).exists())
        .collect::<Vec<_>>();
    assert!(
        remaining_paths.is_empty(),
        "legacy Agent conversation runtime modules remain:\n{}",
        remaining_paths.join("\n")
    );

    let mut violations = Vec::new();
    collect_removed_symbols(
        &manifest_dir.join("src"),
        &[
            "MessagePart",
            "PermissionRequest",
            "AgentSessionDomainEvent",
            "AgentSessionStateRecord",
            "AgentQueuedSendRecord",
            "AgentContextCarryStateRecord",
            "AgentTurnMetric",
            "AgentSessionNotificationUsecase",
            "FocusTracker",
        ],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "legacy Agent conversation runtime symbols remain:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_removed_notification_feature_has_no_configuration_or_ui_surface() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let removed_paths = [
        "src/types/webhook.ts",
        "src/hooks/useWebhookConfig.ts",
        "src-tauri/src/domain/notification",
        "src-tauri/src/usecase/notification",
        "src-tauri/src/adaptor/gateway/notification",
        "src-tauri/src/adaptor/controller/command/notification",
    ];
    let remaining_paths = removed_paths
        .into_iter()
        .filter(|relative_path| repository_root.join(relative_path).exists())
        .collect::<Vec<_>>();
    assert!(
        remaining_paths.is_empty(),
        "removed notification feature paths remain:\n{}",
        remaining_paths.join("\n")
    );

    for (relative_path, removed) in [
        ("src/components/panels/SettingsModal.tsx", "Notifications"),
        (
            "src-tauri/src/adaptor/gateway/app_config/config_models.rs",
            "NotifySection",
        ),
        (
            "src-tauri/src/domain/app_config/value_objects/mod.rs",
            "NotifyConfig",
        ),
    ] {
        let source = read_repository_file(relative_path);
        assert!(
            !source.contains(removed),
            "removed notification feature symbol remains in {relative_path}: {removed}"
        );
    }
}

#[test]
fn atui_050_frontend_does_not_use_legacy_agent_status_projection() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let obsolete_paths = [
        "src/types/session.ts",
        "src/hooks/useWorkspaceStatus.ts",
        "src/hooks/useWorkspaceStatus.test.ts",
        "src/hooks/useWorktreeSessionStatuses.ts",
        "src/hooks/useWorktreeSessionStatuses.test.ts",
        "src/components/ui/agent-state-icon.tsx",
    ];
    let remaining_paths = obsolete_paths
        .into_iter()
        .filter(|relative_path| repository_root.join(relative_path).exists())
        .collect::<Vec<_>>();
    assert!(
        remaining_paths.is_empty(),
        "legacy Agent status frontend modules remain:\n{}",
        remaining_paths.join("\n")
    );

    let mut violations = Vec::new();
    collect_frontend_removed_symbols(
        &repository_root.join("src"),
        &[
            "workspace-status-changed",
            "session-status-changed",
            "list_workspace_statuses",
            "get_workspace_status",
            "list_session_statuses",
            "@/types/session",
            "AgentState",
            "agent_state",
            "AGENT_CONFIGS",
            "AgentType",
            "agentAutoApprove",
            "agentMaxConcurrent",
            "permissionMode: \"ask\"",
            "PROVIDER_AGENT_SESSION",
        ],
        &mut violations,
    );
    assert!(
        violations.is_empty(),
        "legacy Agent status projection remains in frontend:\n{}",
        violations.join("\n")
    );
}

#[test]
fn atui_050_legacy_agent_only_dependencies_are_removed() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let package: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repository_root.join("package.json"))
            .expect("read frontend package manifest"),
    )
    .expect("parse frontend package manifest");
    let dependencies = package["dependencies"]
        .as_object()
        .expect("frontend dependencies object");
    for removed in ["@anthropic-ai/sdk", "@lobehub/icons-static-svg"] {
        assert!(
            !dependencies.contains_key(removed),
            "legacy Agent GUI dependency remains: {removed}"
        );
    }

    let cargo_manifest = std::fs::read_to_string(repository_root.join("src-tauri/Cargo.toml"))
        .expect("read Rust package manifest");
    assert!(
        !cargo_manifest
            .lines()
            .any(|line| line.starts_with("ignore = ")),
        "legacy mention gateway dependency remains: ignore"
    );
}

fn collect_removed_symbols(path: &Path, removed: &[&str], violations: &mut Vec<String>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).expect("read source directory") {
            collect_removed_symbols(&entry.expect("source entry").path(), removed, violations);
        }
        return;
    }
    if path.extension().and_then(|value| value.to_str()) != Some("rs") {
        return;
    }
    let source = std::fs::read_to_string(path).expect("read source file");
    for symbol in removed {
        if source.contains(symbol) {
            violations.push(format!("{}: {symbol}", path.display()));
        }
    }
}

fn collect_frontend_removed_symbols(path: &Path, removed: &[&str], violations: &mut Vec<String>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).expect("read frontend source directory") {
            collect_frontend_removed_symbols(
                &entry.expect("frontend source entry").path(),
                removed,
                violations,
            );
        }
        return;
    }
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("ts" | "tsx")
    ) {
        return;
    }
    let source = std::fs::read_to_string(path).expect("read frontend source file");
    for symbol in removed {
        if source.contains(symbol) {
            violations.push(format!("{}: {symbol}", path.display()));
        }
    }
}

fn collect_legacy_message_projection_symbols(path: &Path, violations: &mut Vec<String>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).expect("read source directory") {
            collect_legacy_message_projection_symbols(
                &entry.expect("source entry").path(),
                violations,
            );
        }
        return;
    }
    if path.extension().and_then(|value| value.to_str()) != Some("rs") {
        return;
    }
    let source = std::fs::read_to_string(path).expect("read source file");
    for removed in [
        "MessageProjectionRecord",
        "MessageProjectionMutation",
        "MessageProjectionByIdentity",
        "MessageProjectionPage",
        "LocalStateMutation::MessageProjection",
        "AgentMessageProjectionRecord",
        "AgentContentBlobRecord",
        "AgentSessionMetadataRecord",
    ] {
        if source.contains(removed) {
            violations.push(format!("{}: {removed}", path.display()));
        }
    }
}

fn collect_temporary_agent_session_names(path: &Path, violations: &mut Vec<String>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).expect("read source directory") {
            collect_temporary_agent_session_names(&entry.expect("source entry").path(), violations);
        }
        return;
    }
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("rs" | "ts" | "tsx")
    ) {
        return;
    }
    let source = std::fs::read_to_string(path).expect("read source file");
    for temporary in [
        "ProviderAgentSession",
        "provider_agent_session",
        "provider-agent-session",
        "PROVIDER_AGENT_SESSION",
        "Provider AgentSession",
        "ProviderAgentInitialInstruction",
        "ProviderAgentWorkflowSessionLaunch",
        "PreparedProviderAgentLaunch",
        "DurableProviderAgentLaunch",
    ] {
        if source.contains(temporary) {
            violations.push(format!("{}: {temporary}", path.display()));
        }
    }
}

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
struct AgentSessionHistoryPage {
    items: Vec<releash_lib::agent_session_tui_acceptance::AcceptanceHistoryCandidate>,
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
            "list_available_agent_session_providers",
            serde_json::json!({}),
        )
        .expect("available Provider command")
    }

    fn provider_availability(&self) -> Result<ProviderAvailabilitySnapshot, String> {
        self.invoke("get_provider_availability", serde_json::json!({}))
    }

    fn update_provider_executable(
        &self,
        provider: AcceptanceProvider,
        executable: &Path,
    ) -> Result<ProviderAvailabilitySnapshot, String> {
        self.invoke(
            "update_provider_executable",
            serde_json::json!({
                "provider": provider_name(provider),
                "executable": executable.to_string_lossy(),
            }),
        )
    }

    fn reset_provider_executable(
        &self,
        provider: AcceptanceProvider,
    ) -> Result<ProviderAvailabilitySnapshot, String> {
        self.invoke(
            "reset_provider_executable",
            serde_json::json!({ "provider": provider_name(provider) }),
        )
    }

    fn refresh_provider_availability(&self) -> Result<ProviderAvailabilitySnapshot, String> {
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
        self.invoke(
            "create_agent_session",
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
            "get_agent_session",
            serde_json::json!({ "agentSessionId": agent_session_id }),
        )
    }

    async fn list_history(
        &self,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<releash_lib::agent_session_tui_acceptance::AcceptanceHistoryCandidate>, String>
    {
        self.invoke::<AgentSessionHistoryPage>(
            "list_agent_session_history",
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
        )
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
    ) -> Result<AcceptanceOpenOutcome, String> {
        self.invoke_open_command(
            "restore_agent_session",
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
            "resume_agent_session",
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
            "delete_agent_session",
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
            "confirm_agent_session_archive_delete",
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
async fn test_atui_025_初期化した全providerの利用可否と理由をproduction境界から取得する() {
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

    let snapshot = host.provider_availability().unwrap();

    assert!(host
        .invoke::<ProviderAvailabilitySnapshot>(
            "update_provider_executable",
            serde_json::json!({ "provider": "unknown", "executable": "agent" }),
        )
        .is_err());
    assert!(host
        .invoke::<ProviderAvailabilitySnapshot>(
            "update_provider_executable",
            serde_json::json!({ "provider": "claude", "executable": "  " }),
        )
        .is_err());

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
    assert_eq!(host.available_providers(), vec![AcceptanceProvider::Claude]);

    let refreshed_codex = install_fixture_executable(
        &refreshed_login_shell_bin,
        "codex",
        AcceptanceProvider::Codex,
        4,
    );
    let refreshed = host.refresh_provider_availability().unwrap();
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
    assert_eq!(host.available_providers(), vec![AcceptanceProvider::Codex]);

    let replacement = install_fixture_executable(
        root.path(),
        "non-standard-codex",
        AcceptanceProvider::Codex,
        4,
    );
    let updated = host
        .update_provider_executable(AcceptanceProvider::Codex, &replacement)
        .unwrap();
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
    assert!(host
        .available_providers()
        .contains(&AcceptanceProvider::Codex));

    let workspace = root.path().join("worktree");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace = workspace.to_string_lossy().into_owned();
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
        .attach("atui-025-standalone".to_string(), standalone_owner.clone())
        .unwrap();
    receive_until(&mut standalone, "non-standard-codex").await;
    assert!(host
        .launch_workflow(
            &workspace,
            AcceptanceProvider::Codex,
            "atui-025-workflow",
            "atui-025-node",
            "verify shared registry",
        )
        .await
        .is_ok());

    std::fs::remove_file(&replacement).unwrap();
    let refreshed = host.refresh_provider_availability().unwrap();
    let codex = refreshed
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert!(!codex.available);
    assert_eq!(codex.unavailable_reason.as_deref(), Some("not_found"));
    assert!(host
        .launch_standalone(
            "workspace-atui-025",
            &workspace,
            AcceptanceProvider::Codex,
            24,
            80,
            "atui-025-unavailable",
        )
        .await
        .is_err());
    assert!(host
        .launch_workflow(
            &workspace,
            AcceptanceProvider::Codex,
            "atui-025-workflow-unavailable",
            "atui-025-node-unavailable",
            "must be rejected before creation",
        )
        .await
        .is_err());
    assert!(host.get(&standalone_id).await.unwrap().is_some());

    host.terminal()
        .write(standalone_owner, "still-running\r")
        .unwrap();
    receive_until(&mut standalone, "received-0:still-running").await;

    let reset = host
        .reset_provider_executable(AcceptanceProvider::Codex)
        .unwrap();
    let codex = reset
        .providers
        .iter()
        .find(|item| item.provider == AcceptanceProvider::Codex)
        .unwrap();
    assert_eq!(codex.configured_executable, None);
    assert_eq!(codex.default_executable, "codex");
    assert_eq!(codex.effective_executable, "codex");

    host.shutdown().await.unwrap();
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
        claude_executable: Some(claude),
        codex_executable: Some(codex),
        provider_search_path: None,
        provider_refresh_search_path: None,
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
            "system policy\n\nimplement once",
        )
        .await
        .unwrap();
    let terminal_owner = owner(&workspace, &session_id);
    let mut attached = host
        .terminal()
        .attach("workflow-session".to_string(), terminal_owner.clone())
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
        host.get(&session_id).await.unwrap().unwrap().tree_parent,
        Some(AcceptanceAgentSessionTreeParent {
            tree_id: "workflow-execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
        })
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
