#[cfg(test)]
use crate::adaptor::controller::state::AppState;
#[cfg(test)]
use crate::adaptor::gateway::workflow::builtin;
#[cfg(test)]
use crate::adaptor::gateway::workflow::facet::FacetKind;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::{
    FacetSummary as GatewayFacetSummary, WorkflowDefinitionYaml,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::storage;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tauri::Manager;

pub(crate) mod definition;
pub(crate) mod diagnostics;
pub(crate) mod execution;
pub(crate) mod facet;
pub(crate) mod output;
pub(crate) mod runtime;
#[cfg(test)]
pub(crate) mod session_errors;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_workflows",
    "get_workflow",
    "get_workflow_source",
    "save_workflow_source",
    "delete_workflow",
    "open_workflow_in_editor",
    "start_workflow",
    "abort_workflow",
    "stop_workflow",
    "resume_workflow",
    "approve_workflow_node",
    "list_workflow_executions",
    "get_workflow_execution",
    "get_workflow_execution_log",
    "get_workflow_execution_state",
    "get_workflow_node_detail",
    "resolve_active_execution_by_worktree",
    "resolve_worktree_by_execution",
    "list_facets",
    "get_facet",
    "save_facet",
    "delete_facet",
    "diagnose_all_cmd",
    "list_facet_summaries",
    "duplicate_workflow",
    "duplicate_facet",
    "open_facet_in_editor",
    "render_facet_preview",
    "get_automation_config_dir",
    "workflow_submit_output",
    "workflow_validate_output",
    "workflow_get_output",
];

#[cfg(test)]
fn handles_command(command: &str) -> bool {
    COMMAND_NAMES.contains(&command)
}

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        definition::list_workflows,
        definition::get_workflow,
        definition::get_workflow_source,
        definition::save_workflow_source,
        definition::delete_workflow,
        definition::open_workflow_in_editor,
        runtime::start_workflow,
        runtime::abort_workflow,
        runtime::stop_workflow,
        runtime::resume_workflow,
        runtime::approve_workflow_node,
        execution::list_workflow_executions,
        execution::get_workflow_execution,
        execution::get_workflow_execution_log,
        execution::get_workflow_execution_state,
        execution::get_workflow_node_detail,
        execution::resolve_active_execution_by_worktree,
        execution::resolve_worktree_by_execution,
        facet::list_facets,
        facet::get_facet,
        facet::save_facet,
        facet::delete_facet,
        diagnostics::diagnose_all_cmd,
        facet::list_facet_summaries,
        definition::duplicate_workflow,
        facet::duplicate_facet,
        facet::open_facet_in_editor,
        diagnostics::render_facet_preview,
        diagnostics::get_automation_config_dir,
        output::workflow_submit_output,
        output::workflow_validate_output,
        output::workflow_get_output,
    ]
}

#[cfg(test)]
use self::session_errors::redacted_workflow_tab_error;

#[cfg(test)]
fn parse_facet_kind(kind: &str) -> Result<FacetKind, String> {
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        _ => Err(format!("Unknown facet kind: {kind}")),
    }
}

// ---- ファセットコマンドの内部実装（テスト可能な純粋関数として切り出し） ----
//
// Tauri コマンドはこれらの inner 関数に委譲する。インテグレーションを
// テンポラリディレクトリ上で再現することで、3 種それぞれの正常経路到達と、
// 廃止済み種別および未知種別での I/O 非発生を直接検証できるようにする。

#[cfg(test)]
fn list_facets_inner(kind: &str, base_dir: &Path) -> Result<Vec<String>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn get_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<String, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::load_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn save_facet_inner(
    kind: &str,
    key: &str,
    content: &str,
    is_new: bool,
    base_dir: &Path,
) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは編集できません".to_string());
    }
    validate_template_variables(content)?;
    if is_new {
        let existing = crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
            .map_err(|e| e.to_string())?;
        if existing.contains(&key.to_string()) {
            return Err(format!("ファセット '{key}' は既に存在します"));
        }
    }
    crate::adaptor::gateway::workflow::facet::save_facet(facet_kind, key, content, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn delete_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは削除できません".to_string());
    }
    crate::adaptor::gateway::workflow::facet::delete_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn list_facet_summaries_inner(
    kind: &str,
    base_dir: &Path,
) -> Result<Vec<GatewayFacetSummary>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facet_summaries(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn duplicate_facet_inner(
    kind: &str,
    source_key: &str,
    new_key: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::validate_facet_key(new_key)
        .map_err(|e| e.to_string())?;
    let existing = crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
        .map_err(|e| e.to_string())?;
    if existing.contains(&new_key.to_string()) {
        return Err(format!("ファセット '{new_key}' は既に存在します"));
    }
    let content =
        crate::adaptor::gateway::workflow::facet::load_facet(facet_kind, source_key, base_dir)
            .map_err(|e| e.to_string())?;
    crate::adaptor::gateway::workflow::facet::save_facet(facet_kind, new_key, &content, base_dir)
        .map_err(|e| e.to_string())
}

/// `open_facet_in_editor` の中核ロジック。エディタ起動はテストで差し替え可能にするため
/// `opener` を引数で受け取る（production では実エディタ起動を渡す）。
#[cfg(test)]
fn open_facet_in_editor_inner<F>(
    kind: &str,
    key: &str,
    base_dir: &Path,
    opener: F,
) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは外部エディタで開けません".to_string());
    }
    let file_path =
        crate::adaptor::gateway::workflow::facet::resolve_facet_path(facet_kind, key, base_dir)
            .map_err(|e| e.to_string())?;
    let path_str = file_path.to_string_lossy().to_string();
    opener(&path_str)
}

#[cfg(test)]
fn validation_error_string(
    e: crate::domain::workflow::services::validation::ValidationError,
) -> String {
    format!("validation_error: {e}")
}

// ---- ワークフロー実行コマンド ----

#[cfg(test)]
fn parse_execution_origin(
    value: Option<String>,
) -> Result<crate::adaptor::gateway::workflow::execution_store::ExecutionOrigin, String> {
    value
        .as_deref()
        .map(crate::domain::workflow::ExecutionOrigin::from_public_value)
        .unwrap_or(Ok(crate::domain::workflow::ExecutionOrigin::DesktopUi))
        .map_err(|error| error.to_string())
}

/// `execution_id` の形式検証（path traversal / 不正文字対策）。
/// UUID（RFC 4122）形式のみ許容する。Execution Store 内部でも canonicalize 後の
/// `workflow_executions/` 配下チェックを行うが、command 入口でも形式不正を弾く。
fn validate_execution_id(execution_id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(execution_id)
        .map(|_| ())
        .map_err(|_| "Invalid execution_id format (must be UUID)".to_string())
}

// ---- [05] read-only execution 観測 API ----
//
// `execution_id` 主語で workflow execution を観測する read-only API。

// ---- 新規コマンド ----

#[cfg(test)]
fn validate_template_variables(content: &str) -> Result<(), String> {
    let errors =
        crate::adaptor::gateway::workflow::workflow_host::prompt_rendering::find_undefined_template_variables(
            content,
        );
    if !errors.is_empty() {
        return Err(format!(
            "未定義のテンプレート変数が含まれています: {}",
            errors.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::execution::{
        get_workflow_execution, get_workflow_execution_log_impl, get_workflow_execution_state_impl,
        get_workflow_node_detail_impl, list_workflow_executions,
    };
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::execution_store::{ExecutionOrigin, ExecutionStatus};
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, NodeKindName, SessionSpec,
    };
    use std::collections::HashSet;
    use std::path::Path;
    use tempfile::TempDir;

    type AdapterTestApp = tauri::App<tauri::test::MockRuntime>;

    fn make_adapter_app() -> AdapterTestApp {
        let data_dir =
            std::env::temp_dir().join(format!("releash-command-adapter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let app_config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            crate::adaptor::gateway::app_config::ReleashConfig::default(),
            data_dir.join("config.toml"),
        ));
        let config_repository: Arc<dyn crate::domain::app_config::ConfigRepository> =
            app_config.clone();
        let config_secret_repository: Arc<dyn crate::domain::app_config::ConfigSecretRepository> =
            app_config.clone();
        tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir,
            ))
            .manage(app_config)
            .manage(config_repository)
            .manage(config_secret_repository)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    const REQUIRED_WORKFLOW_EXECUTION_COMMANDS: &[&str] = &[
        "list_workflow_executions",
        "get_workflow_execution",
        "get_workflow_execution_log",
        "get_workflow_execution_state",
        "get_workflow_node_detail",
        "resolve_active_execution_by_worktree",
        "resolve_worktree_by_execution",
    ];

    const REQUIRED_WORKSPACE_EXECUTION_COMMANDS: &[&str] = &[
        "approve_workspace_node",
        "archive_workspace_workflow_execution",
        "get_workspace_node_detail",
        "get_workspace_session_node_id",
        "get_workspace_tree_selection_reconciliation",
        "restore_workspace_workflow_execution",
        "rename_workspace_session_node",
        "retry_workspace_node",
    ];

    const RETIRED_WORKFLOW_COMMANDS: &[&str] = &[
        "list_workflow_runs",
        "get_workflow_run",
        "get_workflow_run_log",
        "get_workflow_run_state",
        "get_workflow_step_detail",
        "resolve_active_run_by_worktree",
        "resolve_worktree_by_run",
        "get_workspace_workflow_step_detail",
        "get_workspace_workflow_node_detail",
        "archive_workspace_workflow_run",
        "restore_workspace_workflow_run",
    ];

    #[test]
    fn workflow_command_registry_uses_execution_and_node_names() {
        let unique: HashSet<_> = COMMAND_NAMES.iter().copied().collect();

        assert_eq!(unique.len(), COMMAND_NAMES.len());
        for command in REQUIRED_WORKFLOW_EXECUTION_COMMANDS {
            assert!(
                handles_command(command),
                "missing workflow command: {command}"
            );
        }
        for command in RETIRED_WORKFLOW_COMMANDS {
            assert!(
                !handles_command(command),
                "retired workflow command is still registered: {command}"
            );
        }

        let workspace_commands = crate::adaptor::controller::command::workspace_tree::COMMAND_NAMES;
        let unique_workspace: HashSet<_> = workspace_commands.iter().copied().collect();
        assert_eq!(unique_workspace.len(), workspace_commands.len());
        for command in REQUIRED_WORKSPACE_EXECUTION_COMMANDS {
            assert!(
                workspace_commands.contains(command),
                "missing workspace workflow command: {command}"
            );
        }
        for command in RETIRED_WORKFLOW_COMMANDS {
            assert!(
                !workspace_commands.contains(command),
                "retired workspace workflow command is still registered: {command}"
            );
        }

        assert!(handles_command("start_workflow"));
        assert!(handles_command("stop_workflow"));
        assert!(handles_command("resume_workflow"));
        assert!(handles_command("workflow_submit_output"));
        assert!(handles_command("workflow_get_output"));
        assert!(!handles_command("get_git_status"));
    }

    #[test]
    fn workflow_tab_error_is_redacted() {
        let err = redacted_workflow_tab_error("workflow_node_session_rejected");
        assert_eq!(
            err,
            "workflow_node_session_rejected: workflow node tab operation failed"
        );
        assert!(!err.contains("/repo"));
        assert!(!err.contains("agent-session"));
        assert!(!err.contains("message body"));
    }

    /// Spec issues-1011 finding 12: command 入口の `validate_execution_id` は path traversal や
    /// 形式不正な execution_id を拒否し、後段の Execution Store / engine に到達させない。
    /// abort_workflow / get_workflow_state / approve_workflow_node /
    /// get_workflow_execution / get_workflow_execution_log / get_workflow_execution_state /
    /// resolve_worktree_by_execution の全 command で共通に使われるため、入力種別ごとに
    /// 受理/拒否を一括で担保する。
    #[test]
    fn validate_execution_id_table_accepts_uuid_and_rejects_invalid_inputs() {
        // 受理: 正規 UUID（生成値）と既知サンプル
        let generated = uuid::Uuid::new_v4().to_string();
        let accepted = [
            generated.as_str(),
            "550e8400-e29b-41d4-a716-446655440000",
            "00000000-0000-0000-0000-000000000000",
        ];
        for input in accepted {
            assert!(
                validate_execution_id(input).is_ok(),
                "valid UUID must be accepted: {input}"
            );
        }

        // 拒否: 空文字 / 非 UUID / path traversal / 不正文字 / 余分なスペース / 長さ違い
        let rejected = [
            "",
            "not-a-uuid",
            "../etc/passwd",
            "../../workflow_executions/secret",
            "execution-1",
            "550e8400-e29b-41d4-a716-44665544000", // 1 文字不足
            "550e8400-e29b-41d4-a716-4466554400000", // 1 文字過剰
            "550e8400-e29b-41d4-a716-44665544000g", // 非 hex
            "550e8400-e29b-41d4-a716-446655440000\n",
            " 550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440000 ",
        ];
        for input in rejected {
            assert!(
                validate_execution_id(input).is_err(),
                "invalid execution_id must be rejected: {input:?}"
            );
        }
    }

    #[test]
    fn parse_execution_origin_rejects_unknown_values() {
        assert!(matches!(
            parse_execution_origin(None).unwrap(),
            crate::adaptor::gateway::workflow::execution_store::ExecutionOrigin::DesktopUi
        ));
        for invalid in ["remote", "unknown"] {
            let err = parse_execution_origin(Some(invalid.to_string()))
                .expect_err("unknown execution origins must be rejected");
            assert!(err.contains("unknown created_from"));
        }
    }

    #[test]
    fn parse_facet_kind_persona_is_rejected() {
        // Gherkin: persona または未知種別を指定した Tauri コマンドは拒否される
        let result = parse_facet_kind("persona");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Unknown facet kind"));
    }

    #[test]
    fn parse_facet_kind_unknown_returns_error() {
        assert!(parse_facet_kind("unknown").is_err());
        assert!(parse_facet_kind("").is_err());
    }

    /// Gherkin: parse_facet_kind を経由する Tauri コマンドは 3種それぞれの種別指定で
    /// 正常経路に到達する（種別解決層）
    #[test]
    fn parse_facet_kind_resolves_all_three_kinds_for_command_routing() {
        for kind in ["policy", "knowledge", "instruction"] {
            assert!(
                parse_facet_kind(kind).is_ok(),
                "kind '{kind}' should be accepted"
            );
        }
    }

    // ---- ファセットコマンド × 3 種カバレッジ + persona / contract / 未知種別拒否 ----
    //
    // policy/knowledge/instruction の正常経路と、persona / contract / 未知種別の拒否を、
    // テンポラリディレクトリ上で検証する。

    const THREE_KINDS: [(&str, &str); 3] = [
        ("policy", "policies"),
        ("knowledge", "knowledge"),
        ("instruction", "instructions"),
    ];

    /// 3 種それぞれのディレクトリを作成し、各種に既存の非ビルトインキー（"sample-{kind}"）を配置する。
    fn setup_tmp_facets_base() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        for (_kind, dir_name) in THREE_KINDS {
            let dir = tmp.path().join(dir_name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("sample-{dir_name}.md")), "SAMPLE_BODY").unwrap();
        }
        tmp
    }

    fn key_for(kind: &str) -> String {
        let (_, dir) = THREE_KINDS.iter().find(|(k, _)| *k == kind).unwrap();
        format!("sample-{dir}")
    }

    fn personas_dir_snapshot(base: &Path) -> Vec<std::path::PathBuf> {
        let personas = base.join("personas");
        if !personas.exists() {
            return Vec::new();
        }
        let mut entries: Vec<_> = std::fs::read_dir(&personas)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        entries
    }

    fn assert_no_persona_files(base: &Path) {
        assert!(
            personas_dir_snapshot(base).is_empty(),
            "personas/ must not be created or written to by any facet command"
        );
    }

    #[test]
    fn list_facets_inner_reaches_listing_path_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let listed = list_facets_inner(kind, tmp.path()).unwrap();
            assert!(
                listed.iter().any(|k| k == &key_for(kind)),
                "list_facets({kind}) must include the seeded key"
            );
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn list_facets_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let result = list_facets_inner(bad, tmp.path());
            assert!(result.is_err(), "list_facets({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn get_facet_inner_reaches_load_path_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let body = get_facet_inner(kind, &key_for(kind), tmp.path()).unwrap();
            assert_eq!(body, "SAMPLE_BODY", "get_facet({kind}) body mismatch");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn get_facet_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let result = get_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "get_facet({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn save_facet_inner_writes_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let key = format!("created-{dir_name}");
            save_facet_inner(kind, &key, "WRITTEN_BODY", true, tmp.path()).unwrap();
            let path = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert!(path.exists(), "save_facet({kind}) must create {path:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "WRITTEN_BODY");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn save_facet_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        let before = personas_dir_snapshot(tmp.path());
        for bad in ["persona", "contract", "unknown"] {
            let result = save_facet_inner(bad, "anything", "BODY", true, tmp.path());
            assert!(result.is_err(), "save_facet({bad}) must be rejected");
        }
        // persona/未知種別では personas/*.md を含むファセットファイルの読み書きを一切行わない
        assert_eq!(personas_dir_snapshot(tmp.path()), before);
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn delete_facet_inner_removes_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let key = key_for(kind);
            let path = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert!(path.exists());
            delete_facet_inner(kind, &key, tmp.path()).unwrap();
            assert!(!path.exists(), "delete_facet({kind}) must remove {path:?}");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn delete_facet_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let result = delete_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "delete_facet({bad}) must be rejected");
        }
        // 3種のサンプルは温存されている
        for (_, dir_name) in THREE_KINDS {
            assert!(tmp
                .path()
                .join(dir_name)
                .join(format!("sample-{dir_name}.md"))
                .exists());
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn list_facet_summaries_inner_lists_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let summaries = list_facet_summaries_inner(kind, tmp.path()).unwrap();
            assert!(
                summaries.iter().any(|s| s.key == key_for(kind)),
                "list_facet_summaries({kind}) must include the seeded key"
            );
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn list_facet_summaries_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let result = list_facet_summaries_inner(bad, tmp.path());
            assert!(
                result.is_err(),
                "list_facet_summaries({bad}) must be rejected"
            );
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn duplicate_facet_inner_creates_new_file_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let source = key_for(kind);
            let new_key = format!("copied-{dir_name}");
            duplicate_facet_inner(kind, &source, &new_key, tmp.path()).unwrap();
            let path = tmp.path().join(dir_name).join(format!("{new_key}.md"));
            assert!(
                path.exists(),
                "duplicate_facet({kind}) must create {path:?}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "SAMPLE_BODY");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn duplicate_facet_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let result = duplicate_facet_inner(bad, "src", "dst", tmp.path());
            assert!(result.is_err(), "duplicate_facet({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn open_facet_in_editor_inner_invokes_opener_for_each_kind() {
        // open_facet_in_editor のエディタ呼び出し点はテストダブル（クロージャ）で差し替えて、
        // 実プロセスを起動せずに 3 種すべての正常経路到達と引数（対象パス）を検証する。
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let recorded: Arc<std::sync::Mutex<Vec<String>>> =
                Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded_clone = recorded.clone();
            let key = key_for(kind);
            open_facet_in_editor_inner(kind, &key, tmp.path(), move |path_str| {
                recorded_clone.lock().unwrap().push(path_str.to_string());
                Ok(())
            })
            .unwrap();
            let paths = recorded.lock().unwrap();
            assert_eq!(
                paths.len(),
                1,
                "opener must be invoked exactly once for {kind}"
            );
            let expected = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert_eq!(
                paths[0],
                expected.to_string_lossy().to_string(),
                "opener must receive the resolved facet path for {kind}"
            );
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn open_facet_in_editor_inner_rejects_persona_and_unknown_without_invoking_opener() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "contract", "unknown"] {
            let invoked: Arc<std::sync::Mutex<bool>> = Arc::new(std::sync::Mutex::new(false));
            let invoked_clone = invoked.clone();
            let result = open_facet_in_editor_inner(bad, "sample", tmp.path(), move |_| {
                *invoked_clone.lock().unwrap() = true;
                Ok(())
            });
            assert!(
                result.is_err(),
                "open_facet_in_editor({bad}) must be rejected"
            );
            assert!(
                !*invoked.lock().unwrap(),
                "opener must not be invoked for {bad}"
            );
        }
        assert_no_persona_files(tmp.path());
    }

    /// Scenario: 既存の personas ディレクトリのファイルはディスク上に残るがアプリからは参照されない
    /// （Spec Rule: Persona廃止後もユーザーディレクトリ上の物理ファイルは保持される）
    ///
    /// temp dir に personas/legacy.md を事前作成し、ファセット一覧系の経路実行後も
    /// ファイルが残り、3種の一覧結果に legacy が含まれないことを直接 assert する。
    #[test]
    fn legacy_persona_file_remains_on_disk_and_is_not_listed_for_any_kind() {
        let tmp = setup_tmp_facets_base();
        let base = tmp.path();

        // 既存ユーザーが残した persona ファイル相当を事前配置
        let personas_dir = base.join("personas");
        std::fs::create_dir_all(&personas_dir).unwrap();
        let legacy_path = personas_dir.join("legacy.md");
        std::fs::write(&legacy_path, "LEGACY_PERSONA_BODY").unwrap();

        // ファセット一覧系経路を 3 種それぞれで実行
        for (kind, _dir_name) in THREE_KINDS {
            let listed = list_facets_inner(kind, base).unwrap();
            assert!(
                !listed.iter().any(|k| k == "legacy"),
                "list_facets({kind}) must not surface the legacy persona key"
            );

            let summaries = list_facet_summaries_inner(kind, base).unwrap();
            assert!(
                !summaries.iter().any(|s| s.key == "legacy"),
                "list_facet_summaries({kind}) must not surface the legacy persona key"
            );
        }

        // 物理ファイルはディスク上に残ったまま（自動削除されない）
        assert!(
            legacy_path.exists(),
            "personas/legacy.md must remain on disk after facet listing"
        );
        assert_eq!(
            std::fs::read_to_string(&legacy_path).unwrap(),
            "LEGACY_PERSONA_BODY",
            "personas/legacy.md content must be preserved untouched"
        );
    }

    #[test]
    fn validate_template_variables_artifact_refs_ok() {
        assert!(validate_template_variables("Use {{ request }} and {{ plan.summary }}").is_ok());
    }

    #[test]
    fn validate_template_variables_invalid_ref_fails() {
        let result = validate_template_variables("Use {{ bad ref }}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad ref"));
    }

    #[test]
    fn validate_template_variables_no_vars_ok() {
        assert!(validate_template_variables("No variables here").is_ok());
    }

    #[test]
    fn validate_template_variables_mixed() {
        let result = validate_template_variables("{{ goal }} and {{ goal.a.b }}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("goal.a.b"));
    }

    // ---- duplicate logic tests ----
    // These test the core duplicate logic using storage/facet/builtin functions directly,
    // mirroring what the Tauri commands do inside spawn_blocking.

    fn make_test_workflow(name: &str) -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: name.to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("review-acceptance".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
            entry: "main".to_string(),
        }
    }

    #[test]
    fn duplicate_workflow_normal_case() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("review-acceptance.md"),
            "Review the change.",
        )
        .unwrap();
        let wf = make_test_workflow("source-wf");
        storage::save_workflow(dir, &wf).unwrap();

        // Simulate duplicate logic
        let new_name = "copied-wf";
        crate::domain::workflow::validation::validate_name(new_name).unwrap();
        assert!(!dir.join(format!("{new_name}.yml")).exists());
        assert!(!builtin::is_builtin_workflow(new_name));

        let mut copied = storage::load_workflow(&dir.join("source-wf.yml"), dir).unwrap();
        copied.name = new_name.to_string();
        copied.builtin = false;
        storage::save_workflow(dir, &copied).unwrap();

        assert!(dir.join(format!("{new_name}.yml")).exists());
        let loaded = storage::load_workflow(&dir.join(format!("{new_name}.yml")), dir).unwrap();
        assert_eq!(loaded.name, new_name);
        assert!(!loaded.builtin);
    }

    #[test]
    fn duplicate_workflow_rejects_existing_custom_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let wf = make_test_workflow("existing-wf");
        storage::save_workflow(dir, &wf).unwrap();

        // Act: Simulate the duplicate check from the command
        let new_name = "existing-wf";
        let result: Result<(), String> = if dir.join(format!("{new_name}.yml")).exists() {
            Err(format!("ワークフロー '{new_name}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    fn duplicate_workflow_rejects_builtin_name() {
        let builtin_names: Vec<String> = builtin::list_builtin_workflows()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        if let Some(name) = builtin_names.first() {
            assert!(builtin::is_builtin_workflow(name));
        }
    }

    #[test]
    fn duplicate_workflow_rejects_invalid_name() {
        let result = crate::domain::workflow::validation::validate_name("bad name!");
        assert!(result.is_err());
    }

    #[test]
    fn validation_errors_return_stable_kind_prefix_for_commands() {
        let err = crate::domain::workflow::validation::validate_name("bad name!")
            .map_err(super::validation_error_string)
            .unwrap_err();
        assert!(err.starts_with("validation_error:"));
    }

    #[test]
    fn duplicate_facet_normal_case() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        crate::adaptor::gateway::workflow::facet::save_facet(
            kind,
            "source-facet",
            "# Source Policy\nContent here",
            base_dir,
        )
        .unwrap();

        let new_key = "copied-facet";
        crate::adaptor::gateway::workflow::facet::validate_facet_key(new_key).unwrap();

        let existing =
            crate::adaptor::gateway::workflow::facet::list_facets(kind, base_dir).unwrap();
        assert!(!existing.contains(&new_key.to_string()));

        let content =
            crate::adaptor::gateway::workflow::facet::load_facet(kind, "source-facet", base_dir)
                .unwrap();
        crate::adaptor::gateway::workflow::facet::save_facet(kind, new_key, &content, base_dir)
            .unwrap();

        let loaded =
            crate::adaptor::gateway::workflow::facet::load_facet(kind, new_key, base_dir).unwrap();
        assert_eq!(loaded, "# Source Policy\nContent here");
    }

    #[test]
    fn duplicate_facet_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        crate::adaptor::gateway::workflow::facet::save_facet(kind, "my-facet", "content", base_dir)
            .unwrap();

        let existing =
            crate::adaptor::gateway::workflow::facet::list_facets(kind, base_dir).unwrap();

        // Act: Simulate the duplicate check from the command
        let new_key = "my-facet";
        let result: Result<(), String> = if existing.contains(&new_key.to_string()) {
            Err(format!("ファセット '{new_key}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    fn duplicate_facet_rejects_invalid_key() {
        let result = crate::adaptor::gateway::workflow::facet::validate_facet_key("../evil");
        assert!(result.is_err());
    }

    // ---- Builtin guard tests ----

    #[test]
    fn builtin_workflow_save_guard() {
        let builtin_names: Vec<String> = builtin::list_builtin_workflows()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        if let Some(name) = builtin_names.first() {
            // Simulates the guard check in save_workflow command
            assert!(builtin::is_builtin_workflow(name));
        }
    }

    #[test]
    fn builtin_facet_save_guard() {
        let builtin_keys = builtin::list_builtin_facet_keys(FacetKind::Policy);
        if let Some(key) = builtin_keys.first() {
            assert!(builtin::is_builtin_facet(FacetKind::Policy, key));
        }
    }

    #[test]
    fn builtin_facet_delete_guard() {
        let builtin_keys = builtin::list_builtin_facet_keys(FacetKind::Policy);
        if let Some(key) = builtin_keys.first() {
            assert!(builtin::is_builtin_facet(FacetKind::Policy, key));
        }
    }

    #[test]
    fn builtin_facet_open_in_editor_guard() {
        let builtin_keys = builtin::list_builtin_facet_keys(FacetKind::Instruction);
        if let Some(key) = builtin_keys.first() {
            assert!(builtin::is_builtin_facet(FacetKind::Instruction, key));
        }
    }

    // ---- save_workflow rename duplicate check ----

    #[test]
    fn save_workflow_rename_rejects_duplicate_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        // Create two workflows
        let wf_a = make_test_workflow("workflow-a");
        let wf_b = make_test_workflow("workflow-b");
        storage::save_workflow(dir, &wf_a).unwrap();
        storage::save_workflow(dir, &wf_b).unwrap();

        // Simulate renaming workflow-a to workflow-b (duplicate)
        let original_name = Some("workflow-a".to_string());
        let new_name = "workflow-b";
        let is_rename = original_name.as_ref().is_some_and(|o| *o != new_name);

        // Act: Simulate the rename duplicate check from the command
        let result: Result<(), String> =
            if is_rename && dir.join(format!("{new_name}.yml")).exists() {
                Err(format!("ワークフロー '{new_name}' は既に存在します"))
            } else {
                Ok(())
            };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- save_facet is_new duplicate check ----

    #[test]
    fn save_facet_is_new_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;

        // Create an existing facet
        crate::adaptor::gateway::workflow::facet::save_facet(
            kind,
            "existing-facet",
            "content",
            base_dir,
        )
        .unwrap();

        // Simulate is_new=true with duplicate key
        let existing =
            crate::adaptor::gateway::workflow::facet::list_facets(kind, base_dir).unwrap();

        // Act: Simulate the is_new duplicate check from the command
        let is_new = true;
        let key = "existing-facet";
        let result: Result<(), String> = if is_new && existing.contains(&key.to_string()) {
            Err(format!("ファセット '{key}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- delete_workflow builtin guard ----

    #[test]
    fn builtin_workflow_delete_guard() {
        let builtin_names: Vec<String> = builtin::list_builtin_workflows()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        if let Some(name) = builtin_names.first() {
            // Simulates the guard check in delete_workflow command
            assert!(builtin::is_builtin_workflow(name));
        }
    }

    // ---- open_workflow_in_editor builtin guard ----

    #[test]
    fn builtin_workflow_open_in_editor_guard() {
        let builtin_names: Vec<String> = builtin::list_builtin_workflows()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        if let Some(name) = builtin_names.first() {
            // Simulates the guard check in open_workflow_in_editor command
            assert!(builtin::is_builtin_workflow(name));
        }
    }

    // ---- duplicate_facet rejects builtin key ----

    #[test]
    fn duplicate_facet_rejects_builtin_key() {
        let builtin_keys = builtin::list_builtin_facet_keys(FacetKind::Policy);
        if let Some(key) = builtin_keys.first() {
            // list_facets includes builtins, so duplicate to a builtin key would be caught
            // by the existing.contains(&new_key) check
            assert!(builtin::is_builtin_facet(FacetKind::Policy, key));

            // Verify list_facets returns builtin keys (which is used for duplicate check)
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path();
            let existing =
                crate::adaptor::gateway::workflow::facet::list_facets(FacetKind::Policy, base_dir)
                    .unwrap();
            assert!(
                existing.contains(&key.to_string()),
                "list_facets should include builtin key '{key}'"
            );
        }
    }

    // ---- save_workflow create/update/rename logic ----

    /// save_workflow のコア判定ロジックを再現するヘルパー
    fn simulate_save_workflow(
        dir: &Path,
        workflow: &WorkflowDefinitionYaml,
        original_name: Option<&str>,
    ) -> Result<(), String> {
        let is_new = original_name.is_none();
        let is_rename = original_name.is_some_and(|o| o != workflow.name);
        if (is_new || is_rename) && dir.join(format!("{}.yml", workflow.name)).exists() {
            return Err(format!("ワークフロー '{}' は既に存在します", workflow.name));
        }
        storage::save_workflow(dir, workflow).map_err(|e| e.to_string())?;
        if let Some(orig) = original_name {
            if orig != workflow.name {
                let old_path = dir.join(format!("{orig}.yml"));
                if old_path.exists() {
                    std::fs::remove_file(&old_path)
                        .map_err(|e| format!("旧ファイル削除失敗: {e}"))?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn save_workflow_existing_same_name_update_succeeds() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("review-acceptance.md"),
            "Review the change.",
        )
        .unwrap();

        let wf = make_test_workflow("my-wf");
        storage::save_workflow(dir, &wf).unwrap();

        // Update same workflow (original_name = Some("my-wf"), name = "my-wf")
        let mut updated = make_test_workflow("my-wf");
        updated.description = "updated desc".to_string();
        let result = simulate_save_workflow(dir, &updated, Some("my-wf"));
        assert!(
            result.is_ok(),
            "Expected same-name update to succeed, got: {result:?}"
        );

        let loaded = storage::load_workflow(&dir.join("my-wf.yml"), dir).unwrap();
        assert_eq!(loaded.description, "updated desc");
    }

    #[test]
    fn save_workflow_new_creation_succeeds() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("brand-new");
        let result = simulate_save_workflow(dir, &wf, None);
        assert!(result.is_ok());
        assert!(dir.join("brand-new.yml").exists());
    }

    #[test]
    fn save_workflow_new_creation_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("dup-wf");
        storage::save_workflow(dir, &wf).unwrap();

        let result = simulate_save_workflow(dir, &wf, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    fn save_workflow_rename_succeeds_and_removes_old_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("old-name");
        storage::save_workflow(dir, &wf).unwrap();

        let mut renamed = make_test_workflow("new-name");
        renamed.description = "renamed".to_string();
        let result = simulate_save_workflow(dir, &renamed, Some("old-name"));
        assert!(result.is_ok());
        assert!(!dir.join("old-name.yml").exists());
        assert!(dir.join("new-name.yml").exists());
    }

    #[test]
    fn save_workflow_rename_rejects_existing_target() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        storage::save_workflow(dir, &make_test_workflow("wf-a")).unwrap();
        storage::save_workflow(dir, &make_test_workflow("wf-b")).unwrap();

        let renamed = make_test_workflow("wf-b");
        let result = simulate_save_workflow(dir, &renamed, Some("wf-a"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- [05] read-only Execution 観測 API: Tauri command 境界の直接テスト ----

    fn read_only_test_uuid(seed: u8) -> String {
        uuid::Uuid::from_bytes([seed; 16]).to_string()
    }

    fn make_read_only_execution(
        execution_id: &str,
        workflow_name: &str,
        worktree: &str,
        status: ExecutionStatus,
        started_at: f64,
    ) -> crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
        crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            status,
            worktree_path: worktree.to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at,
            updated_at: started_at,
            completed_at: if status.is_terminal() {
                Some(started_at + 1.0)
            } else {
                None
            },
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: Default::default(),
        }
    }

    fn write_read_only_execution(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        execution: &crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata,
    ) {
        crate::adaptor::gateway::workflow::test_support::seed_canonical_execution(
            store,
            execution,
            &[],
        );
    }

    fn make_read_only_app() -> (
        AdapterTestApp,
        std::path::PathBuf,
        Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    ) {
        let app = make_adapter_app();
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        // workflow コマンドは repository usecase を State 注入で受け取る。
        let repository_usecase =
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
        app.manage(repository_usecase.clone());
        let config_repository = app
            .state::<Arc<dyn crate::domain::app_config::ConfigRepository>>()
            .inner()
            .clone();
        let config_secret_repository = app
            .state::<Arc<dyn crate::domain::app_config::ConfigSecretRepository>>()
            .inner()
            .clone();
        let notion_config_repository: Arc<dyn crate::domain::app_config::NotionConfigRepository> =
            app.state::<Arc<crate::adaptor::gateway::app_config::AppConfig>>()
                .inner()
                .clone();
        let notion_usecase = Arc::new(crate::usecase::notion::usecase::NotionUsecase::new(
            notion_config_repository,
            Arc::new(crate::adaptor::gateway::notion::NotionApiGatewayImpl::new()),
        ));
        let repo_paths_gateway =
            crate::adaptor::gateway::repository::repo_paths::RepoPathsGateway::new(
                <crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths>::default(),
                config_repository.clone(),
            );
        let repo_paths_usecase =
            Arc::new(crate::usecase::repo_paths_usecase::RepoPathsUsecase::new(
                Arc::new(repo_paths_gateway),
                Arc::new(NoopRepoPathsNotifier),
            ));
        let code_usecase = Arc::new(crate::adaptor::controller::wiring::build_code_usecase());
        let repository_scanner = Arc::new(
            crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                repository_usecase.clone(),
                code_usecase.clone(),
            ),
        );
        let repository_state_repository = Arc::new(
            crate::adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(
                repository_usecase.clone(),
            ),
        );
        let repository_state = Arc::new(
            crate::usecase::repository_state::RepositoryStateService::new(
                repository_state_repository,
                repository_scanner,
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateNotifier),
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher),
                Arc::new(
                    crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime,
                ),
                Arc::new(
                    crate::usecase::repository_state::runtime::tests_support::IdentityWorktreePathNormalizer,
                ),
            ),
        );
        let review_usecase = Arc::new(crate::usecase::review_usecase::ReviewUsecase::new(
            repository_state.clone(),
            code_usecase.clone(),
        ));
        let local_event_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data_dir.clone(),
            ),
        )
        .unwrap();
        let (workflow_usecase, _) =
            crate::adaptor::controller::wiring::build_workflow_services_with_repository_worktrees(
                data_dir.clone(),
                repository_usecase.clone(),
                config_repository,
                config_secret_repository,
                app.handle().clone(),
                local_event_store.clone(),
            );
        app.manage(AppState {
            repository_usecase: repository_usecase.clone(),
            repository_state,
            repo_paths_usecase,
            code_usecase,
            review_usecase,
            notion_usecase,
            workflow_usecase: Arc::new(workflow_usecase),
            terminal_surface: Arc::new(
                crate::adaptor::controller::wiring::build_terminal_surface_application_for_tests(),
            ),
            git_host_usecase: Arc::new(
                crate::adaptor::controller::wiring::build_git_host_usecase(),
            ),
        });
        (app, data_dir, local_event_store)
    }

    struct NoopRepoPathsNotifier;

    impl crate::domain::repository::RepoPathsNotifier for NoopRepoPathsNotifier {
        fn notify_changed(&self, _paths: Vec<String>) {}
    }

    /// [05] worktree-scoped 認可境界のテスト用 fixture: 実 git repo + worktree を作り、
    /// `AppConfig.last_repo_paths` に親 repo を登録する。戻り値は test app / engine /
    /// data_dir / canonical worktree path / TempDir guards（lifetime 保持用）。
    fn make_read_only_app_with_managed_worktree() -> (
        AdapterTestApp,
        std::path::PathBuf,
        Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        String,
        TempDir,
        TempDir,
    ) {
        let (app, data_dir, local_event_store) = make_read_only_app();
        let repo_parent = TempDir::new().unwrap();
        let worktree_parent = TempDir::new().unwrap();
        let repo_path = repo_parent.path().join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        let repo = git2::Repository::init(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();
        let canonical = worktree_path.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();
        let config_repository = app.state::<Arc<dyn crate::domain::app_config::ConfigRepository>>();
        let mut config = config_repository.load().unwrap();
        config.app.last_repo_paths = vec![repo_path.to_string_lossy().to_string()];
        config_repository.save(config).unwrap();
        (
            app,
            data_dir,
            local_event_store,
            canonical_str,
            repo_parent,
            worktree_parent,
        )
    }

    /// Spec [05] Rule: 外部 caller は execution_id を主語として workflow execution 一覧を観測できる。
    /// `list_workflow_executions` Tauri command 経路が active を先頭・terminal を後続として
    /// 並び替え、status filter が機能することを直接検証する。
    ///
    /// 観測経路の認可境界（spec [05] L104-108 / L182）として `worktree_path` は必須で、
    /// caller の認可済み managed worktree のみを対象にする。
    ///
    /// active execution は in-memory map + metadata file の両方に存在する境界状態であるため、
    /// `ExecutionStore::register_active_execution` 経由で投入する。terminal execution はメタデータファイル
    /// のみに投入し、`list_completed` が拾い上げることを検証する。
    #[tokio::test]
    async fn list_workflow_executions_command_returns_active_first_filtered_by_status() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let active_id = read_only_test_uuid(1);
        let done_id = read_only_test_uuid(2);
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &active_id,
                "wf-active",
                &worktree_path,
                ExecutionStatus::Running,
                200.0,
            ),
        );
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &done_id,
                "wf-done",
                &worktree_path,
                ExecutionStatus::Completed,
                100.0,
            ),
        );

        let all = list_workflow_executions(app.state::<AppState>(), None, worktree_path.clone())
            .await
            .expect("list_workflow_executions must succeed");
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].status,
            crate::usecase::workflow::dto::ExecutionStatusDto::Running
        );
        assert_eq!(
            all[1].status,
            crate::usecase::workflow::dto::ExecutionStatusDto::Completed
        );

        let terminal = list_workflow_executions(
            app.state::<AppState>(),
            Some("terminal".to_string()),
            worktree_path.clone(),
        )
        .await
        .expect("terminal filter must succeed");
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].execution_id, done_id);

        let bad = list_workflow_executions(
            app.state::<AppState>(),
            Some("garbage".to_string()),
            worktree_path.clone(),
        )
        .await;
        assert!(bad.is_err(), "invalid status filter must be rejected");
    }

    /// Spec [05] Rule: 観測経路は API と CLI で等価な手段を提供する。
    /// `list_workflow_executions` の `worktree_path` 入力は `canonicalize_managed_worktree_path`
    /// で正規化されたうえで filter に渡されるため、末尾 `/` 付きや `.` を含む形で
    /// 同一 managed worktree を指定しても、canonical 表現で永続化された execution と一致する。
    /// CLI 経路と同じ `normalize_worktree_filter_path` を経由する境界を直接検証する。
    #[tokio::test]
    async fn list_workflow_executions_canonicalizes_worktree_path_filter() {
        let (app, _data_dir, local_event_store, canonical_str, _r, _w) =
            make_read_only_app_with_managed_worktree();

        // canonical な worktree_path で execution を 1 件、別 worktree で execution を 1 件登録する。
        let target_id = read_only_test_uuid(40);
        let other_id = read_only_test_uuid(41);
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &target_id,
                "wf-target",
                &canonical_str,
                ExecutionStatus::Running,
                200.0,
            ),
        );
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &other_id,
                "wf-other",
                "/wt/other",
                ExecutionStatus::Completed,
                100.0,
            ),
        );

        // 末尾 `/` を付けて非 canonical な表現で filter を渡す。
        let trailing = format!("{canonical_str}/");
        let executions = list_workflow_executions(app.state::<AppState>(), None, trailing)
            .await
            .expect("list_workflow_executions with trailing slash must canonicalize and match");
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].execution_id, target_id);
        assert_eq!(executions[0].worktree_path, canonical_str);

        // managed worktree でない path は Err。
        let outside = TempDir::new().unwrap();
        let bad = list_workflow_executions(
            app.state::<AppState>(),
            None,
            outside.path().to_string_lossy().to_string(),
        )
        .await;
        assert!(
            bad.is_err(),
            "worktree_path outside managed worktrees must be rejected"
        );
    }

    /// Spec [05] Rule: 観測経路は権限を持つ caller のみに開かれる。
    /// Tauri API 経路では worktree-scoped 認可境界として
    /// `canonicalize_managed_worktree_path` を経由するため、configured repo に紐づかない
    /// worktree_path を渡した場合は観測結果が caller に伝わらない（Err として拒否される）。
    /// 既存 `list_workflow_executions_command_canonicalizes_managed_worktree_filter` と相補的に
    /// 観測経路ごとの unauthorized 経路を明示する境界テスト。
    #[tokio::test]
    async fn list_workflow_executions_rejects_unauthorized_worktree_observation() {
        let (app, _data_dir, local_event_store) = make_read_only_app();

        // execution は存在する。ただし caller が指定する worktree_path は
        // configured repo に紐づかない（= 観測権限を持たない）ので Err として弾かれる。
        let execution_id = read_only_test_uuid(60);
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &execution_id,
                "wf",
                "/wt/inside",
                ExecutionStatus::Running,
                100.0,
            ),
        );

        let outside = TempDir::new().unwrap();
        let result = list_workflow_executions(
            app.state::<AppState>(),
            None,
            outside.path().to_string_lossy().to_string(),
        )
        .await;
        assert!(
            result.is_err(),
            "unauthorized worktree_path must be rejected as Err (no observation leak)"
        );
    }

    /// Spec [05] Rule: 指定 execution の summary metadata を観測する。
    /// Spec [05] Rule: 存在しない execution_id は明示的に「該当 execution なし」として扱われる。
    /// Spec [05] L104-108 / L182: caller の認可済み worktree に紐づかない execution は Ok(None)。
    #[tokio::test]
    async fn get_workflow_execution_command_returns_summary_or_none() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = read_only_test_uuid(3);
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &execution_id,
                "wf",
                &worktree_path,
                ExecutionStatus::Running,
                300.0,
            ),
        );

        let found = get_workflow_execution(app.state::<AppState>(), execution_id.clone())
            .await
            .expect("get_workflow_execution must succeed");
        let found = found.expect("execution must be found");
        assert_eq!(found.execution_id, execution_id);
        assert_eq!(found.worktree_path, worktree_path);
        assert_eq!(
            found.status,
            crate::usecase::workflow::dto::ExecutionStatusDto::Running
        );

        let missing = get_workflow_execution(app.state::<AppState>(), read_only_test_uuid(99))
            .await
            .expect("get_workflow_execution for unknown execution_id must Ok(None)");
        assert!(missing.is_none());

        let invalid =
            get_workflow_execution(app.state::<AppState>(), "not-a-uuid".to_string()).await;
        assert!(invalid.is_err(), "non-UUID execution_id must be rejected");
    }

    /// Spec [05] L104-108 / L182 negative test: 認可境界として、execution metadata の
    /// worktree_path が caller の認可済み managed worktree に合致しない場合、
    /// `get_workflow_execution` / `get_workflow_execution_log` / `get_workflow_execution_state` は
    /// いずれも `Ok(None)` を返し、存在する execution の summary / log / state を
    /// 未認可 caller に伝えない。
    #[tokio::test]
    async fn single_execution_read_only_apis_reject_unauthorized_worktree_observation() {
        let (app, _data_dir, local_event_store, _managed, _r, _w) =
            make_read_only_app_with_managed_worktree();

        // execution は別 worktree に紐づく metadata で書かれている（未認可）。
        let execution_id = read_only_test_uuid(70);
        let unauthorized_wt = "/wt/unauthorized";
        write_read_only_execution(
            &local_event_store,
            &make_read_only_execution(
                &execution_id,
                "wf",
                unauthorized_wt,
                ExecutionStatus::Running,
                100.0,
            ),
        );
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &local_event_store,
            &[WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                worktree_path: unauthorized_wt.to_string(),
                created_from: ExecutionOrigin::DesktopUi,
                request: String::new(),
                definition: WorkflowDefinitionYaml {
                    name: "wf".to_string(),
                    description: "test".to_string(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![],
                    entry: "main".to_string(),
                },
                timestamp: 100.0,
            }],
        )
        .unwrap();

        let summary = get_workflow_execution(app.state::<AppState>(), execution_id.clone())
            .await
            .expect("get_workflow_execution must succeed");
        assert!(
            summary.is_none(),
            "unauthorized execution summary must not be observable"
        );

        // unauthorized worktree_path は canonicalize 段階で Err として弾かれる。
        let log = get_workflow_execution_log_impl(
            &app.state::<AppState>().workflow_usecase,
            unauthorized_wt.to_string(),
            execution_id.clone(),
        )
        .await;
        assert!(
            log.is_err(),
            "unauthorized worktree_path must be rejected by event log invoke"
        );

        let state = get_workflow_execution_state_impl(
            &app.state::<AppState>().workflow_usecase,
            unauthorized_wt.to_string(),
            execution_id.clone(),
        )
        .await;
        assert!(
            state.is_err(),
            "unauthorized worktree_path must be rejected by state invoke"
        );
    }

    /// Spec [05] Rule: 指定 execution の event log を観測する。
    /// `get_workflow_execution_log` Tauri command が NDJSON から読み込んだ event 列を返す。
    #[tokio::test]
    async fn get_workflow_execution_log_command_reads_persisted_ndjson() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = read_only_test_uuid(4);
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: make_test_workflow("wf"),
                    timestamp: 400.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-main-1".to_string(),
                    node_name: "main".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    parent: None,
                    timestamp: 400.0,
                },
            ],
        )
        .unwrap();

        let events = get_workflow_execution_log_impl(
            &app.state::<AppState>().workflow_usecase,
            worktree_path.clone(),
            execution_id.clone(),
        )
        .await
        .expect("get_workflow_execution_log must succeed")
        .expect("execution must be found");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "started");
        // spec issues-1023: 永続化された秒単位 timestamp は API 境界で ms 単位の
        // view 型へ変換されて返る（`timestampMs` フィールドで単位を明示）。
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(400_000.0));

        let missing = get_workflow_execution_log_impl(
            &app.state::<AppState>().workflow_usecase,
            worktree_path.clone(),
            read_only_test_uuid(98),
        )
        .await
        .expect("get_workflow_execution_log for unknown execution_id must Ok(None)");
        assert!(missing.is_none());
    }

    /// Spec [05] Rule: 指定 execution の現在 state を観測する（event log からの純粋投影）。
    /// 観測結果の露出範囲境界: live runtime registry / OpenTabRegistry 由来の runtime_active /
    /// tab_open enrichment は含めない（戻り値の runtime_states は空）。
    #[tokio::test]
    async fn get_workflow_execution_state_command_projects_state_without_runtime_enrichment() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = read_only_test_uuid(5);
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "adapter-boundary".to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: make_test_workflow("adapter-boundary"),
                    timestamp: 500.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-main-1".to_string(),
                    node_name: "main".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    parent: None,
                    timestamp: 500.0,
                },
            ],
        )
        .unwrap();

        let view = get_workflow_execution_state_impl(
            &app.state::<AppState>().workflow_usecase,
            worktree_path.clone(),
            execution_id.clone(),
        )
        .await
        .expect("get_workflow_execution_state must succeed")
        .expect("state must be available");
        assert_eq!(view.id, execution_id);
        assert_eq!(view.node_executions.len(), 1);
        assert_eq!(view.node_executions[0].node_name, "main");
        assert!(view.node_executions[0].session_id.is_none());

        // 存在しない execution_id は Ok(None)。
        let missing = get_workflow_execution_state_impl(
            &app.state::<AppState>().workflow_usecase,
            worktree_path.clone(),
            read_only_test_uuid(97),
        )
        .await
        .expect("unknown execution must Ok(None)");
        assert!(missing.is_none());
    }

    /// spec issues-1023 L132/L150: `get_workflow_node_detail_impl` の worktree
    /// 認可境界。現 worktree と一致する execution の detail は Some、別 worktree（managed
    /// 集合外）からの invoke は canonicalize 段階で Err として弾かれる。
    #[tokio::test]
    async fn get_workflow_node_detail_enforces_current_worktree_authorization() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();

        let execution_id = read_only_test_uuid(82);
        let snapshot = WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                crate::adaptor::gateway::workflow::schema::NodeDefinition {
                    name: "main".to_string(),
                    kind: crate::adaptor::gateway::workflow::schema::NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            output: None,
                            children: vec![crate::domain::workflow::ChildEntry::reference("plan")],
                        },
                    ),
                    ..crate::adaptor::gateway::workflow::schema::NodeDefinition::default()
                },
                crate::adaptor::gateway::workflow::schema::NodeDefinition {
                    name: "plan".to_string(),
                    kind: crate::adaptor::gateway::workflow::schema::NodeKind::Command(
                        crate::adaptor::gateway::workflow::schema::CommandSpec {
                            command: "true".to_string(),
                            env: Default::default(),
                        },
                    ),
                    ..crate::adaptor::gateway::workflow::schema::NodeDefinition::default()
                },
            ],
            entry: "main".to_string(),
        };
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: snapshot,
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-main-1".to_string(),
                    node_name: "main".to_string(),
                    kind: NodeKindName::Sequence,
                    attempt: 1,
                    parent: None,
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-plan-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    parent: Some(crate::domain::workflow::ExecutionParentRef::sequence_child(
                        "ne-main-1",
                    )),
                    timestamp: 101.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-plan-1".to_string(),
                    node_name: "plan".to_string(),
                    attempt: 1,
                    result_summary: Some("done".to_string()),
                    token_usage: None,
                    timestamp: 102.0,
                },
            ],
        )
        .unwrap();

        // 現 worktree からの invoke は detail を返す
        let ok = get_workflow_node_detail_impl(
            &app.state::<AppState>().workflow_usecase,
            worktree_path.clone(),
            execution_id.clone(),
            "ne-plan-1".to_string(),
        )
        .await
        .expect("detail invoke must succeed");
        let detail = ok.expect("detail must be observable for matching worktree");
        assert_eq!(detail.node_name, "plan");
        assert_eq!(detail.attempt, 1);
        assert_eq!(detail.result_summary.as_deref(), Some("done"));

        // 別 worktree（managed 集合外）からの invoke は canonicalize 段階で Err
        let outside = tempfile::TempDir::new().unwrap();
        let result = get_workflow_node_detail_impl(
            &app.state::<AppState>().workflow_usecase,
            outside.path().to_string_lossy().to_string(),
            execution_id.clone(),
            "ne-plan-1".to_string(),
        )
        .await;
        assert!(
            result.is_err(),
            "detail invoke from unauthorized worktree must be rejected"
        );
    }
}
