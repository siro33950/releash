mod shared;
pub(crate) use shared::register_shared;

#[cfg(all(test, feature = "desktop"))]
use crate::adaptor::controller::state::AppState;
#[cfg(all(test, feature = "desktop"))]
use crate::adaptor::gateway::workflow::builtin;
#[cfg(all(test, feature = "desktop"))]
use crate::adaptor::gateway::workflow::facet::FacetKind;
#[cfg(all(test, feature = "desktop"))]
use crate::adaptor::gateway::workflow::schema::{
    FacetSummary as GatewayFacetSummary, WorkflowDefinitionYaml,
};
#[cfg(all(test, feature = "desktop"))]
use crate::adaptor::gateway::workflow::storage;
#[cfg(all(test, feature = "desktop"))]
use std::path::Path;
#[cfg(all(test, feature = "desktop"))]
use std::sync::Arc;
#[cfg(all(test, feature = "desktop"))]
use tauri::Manager;

pub(crate) mod definition;
pub(crate) mod diagnostics;
pub(crate) mod facet;
pub(crate) mod output;
pub(crate) mod runtime;
#[cfg(all(test, feature = "desktop"))]
pub(crate) mod session_errors;

#[cfg(all(test, feature = "desktop"))]
use self::session_errors::redacted_workflow_tab_error;

#[cfg(all(test, feature = "desktop"))]
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

#[cfg(all(test, feature = "desktop"))]
fn list_facets_inner(kind: &str, base_dir: &Path) -> Result<Vec<String>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "desktop"))]
fn get_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<String, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::load_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "desktop"))]
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

#[cfg(all(test, feature = "desktop"))]
fn delete_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは削除できません".to_string());
    }
    crate::adaptor::gateway::workflow::facet::delete_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "desktop"))]
fn list_facet_summaries_inner(
    kind: &str,
    base_dir: &Path,
) -> Result<Vec<GatewayFacetSummary>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facet_summaries(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "desktop"))]
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
#[cfg(all(test, feature = "desktop"))]
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

#[cfg(all(test, feature = "desktop"))]
fn validation_error_string(
    e: crate::domain::workflow::services::validation::ValidationError,
) -> String {
    format!("validation_error: {e}")
}

// ---- ワークフロー実行コマンド ----

#[cfg(all(test, feature = "desktop"))]
fn parse_execution_origin(
    value: Option<String>,
) -> Result<crate::domain::workflow::ExecutionOrigin, String> {
    value
        .as_deref()
        .map(crate::domain::workflow::ExecutionOrigin::from_public_value)
        .unwrap_or(Ok(crate::domain::workflow::ExecutionOrigin::DesktopUi))
        .map_err(|error| error.to_string())
}

/// `execution_id` の形式検証（path traversal / 不正文字対策）。
/// UUID（RFC 4122）形式のみ許容する。
fn validate_execution_id(
    execution_id: &str,
) -> Result<(), crate::adaptor::presenter::error::AppError> {
    uuid::Uuid::parse_str(execution_id)
        .map(|_| ())
        .map_err(|_| {
            crate::adaptor::presenter::error::AppError::new(
                "Invalid execution_id format (must be UUID)",
            )
            .with_failure_kind(crate::domain::failure::FailureKind::InvalidInput)
        })
}

// ---- [05] read-only execution 観測 API ----
//
// `execution_id` 主語で workflow execution を観測する read-only API。

// ---- 新規コマンド ----

#[cfg(all(test, feature = "desktop"))]
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

#[cfg(all(test, feature = "desktop"))]
pub(crate) mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, NodeKindName, SessionSpec,
    };
    use crate::domain::workflow::ExecutionOrigin;
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
            .invoke_handler(
                crate::adaptor::controller::command::application_lifecycle::invoke_handler(),
            )
            .manage(Arc::new(crate::infrastructure::push::PushSink::new()))
            .manage(crate::desktop_test_support::TestDataDir(data_dir))
            .manage(app_config)
            .manage(config_repository)
            .manage(config_secret_repository)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    const REQUIRED_WORKSPACE_EXECUTION_COMMANDS: &[&str] = &[
        "approve_workspace_node",
        "archive_workspace_workflow_execution",
        "restore_workspace_workflow_execution",
        "rename_workspace_session_node",
        "retry_workspace_node",
    ];

    const RETIRED_WORKFLOW_COMMANDS: &[&str] = &[
        "get_workflow_execution_state",
        "resolve_active_execution_by_worktree",
        "get_workspace_node_detail",
        "get_workspace_session_node_id",
        "get_workspace_tree_selection_reconciliation",
        "list_workflow_executions",
        "get_workflow_execution",
        "get_workflow_execution_log",
        "get_workflow_node_detail",
        "resolve_worktree_by_execution",
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
        let (app, _data_dir, _store) = make_read_only_app();
        app.manage(Arc::new(
            crate::infrastructure::file_watcher::FileWatcherManager::default(),
        ));
        let deps = crate::desktop_test_support::build_client_dependencies(app.handle());
        let mut dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(
            Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
        );
        register_shared(&mut dispatch, &deps);
        let handles_command = |command| dispatch.contains(command);
        for command in RETIRED_WORKFLOW_COMMANDS {
            assert!(
                !handles_command(command),
                "retired workflow command is still registered: {command}"
            );
        }

        let mut workspace_dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(
            Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
        );
        crate::adaptor::controller::client::workspace_tree::register_shared(
            &mut workspace_dispatch,
            &deps,
        );
        for command in REQUIRED_WORKSPACE_EXECUTION_COMMANDS {
            assert!(
                workspace_dispatch.contains(command),
                "missing workspace workflow command: {command}"
            );
        }
        for command in RETIRED_WORKFLOW_COMMANDS {
            assert!(
                !workspace_dispatch.contains(command),
                "retired workspace workflow command is still registered: {command}"
            );
        }

        assert!(handles_command("start_workflow"));
        assert!(!handles_command("stop_workflow"));
        assert!(!handles_command("resume_workflow"));
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
            crate::domain::workflow::ExecutionOrigin::DesktopUi
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
    fn test_template変数検証_0段と多段の参照を受理する() {
        let result = validate_template_variables("{{ goal }} and {{ goal.a.b }}");
        assert!(result.is_ok());
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

    pub(crate) fn make_read_only_app() -> (
        AdapterTestApp,
        std::path::PathBuf,
        Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    ) {
        make_read_only_app_with_terminal(Arc::new(
            crate::adaptor::controller::wiring::build_terminal_surface_application_for_tests(),
        ))
    }

    pub(crate) fn make_read_only_app_with_terminal(
        terminal_surface: Arc<
            crate::usecase::terminal_surface::application::TerminalSurfaceApplication,
        >,
    ) -> (
        AdapterTestApp,
        std::path::PathBuf,
        Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    ) {
        let app = make_adapter_app();
        let data_dir = crate::desktop_test_support::data_dir(app.handle()).unwrap();
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
            crate::usecase::repository_state::RepositoryStateService::new(crate::usecase::work_queue::shared().clone(),
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
                crate::usecase::work_queue::shared().clone(),
                data_dir.clone(),
                repository_usecase.clone(),
                config_repository,
                config_secret_repository,
                local_event_store.clone(),
                Arc::new(
                    crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::default(
                    ),
                ),
            );
        let workflow_usecase = Arc::new(workflow_usecase);
        let git_host_usecase =
            Arc::new(crate::adaptor::controller::wiring::build_git_host_usecase());
        let workspace_list = Arc::new(
            crate::adaptor::controller::wiring::build_workspace_list_usecase(
                repo_paths_usecase.clone(),
                repository_state.clone(),
                workflow_usecase.clone(),
                git_host_usecase.clone(),
            ),
        );
        app.manage(AppState {
            workspace_list,
            repository_usecase: repository_usecase.clone(),
            repository_state,
            repo_paths_usecase,
            code_usecase,
            review_usecase,
            notion_usecase,
            workflow_usecase,
            terminal_surface,
            git_host_usecase,
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

    /// Spec [05] Rule: 指定 execution の現在 state を観測する（event log からの純粋投影）。
    /// 観測結果の露出範囲境界: live runtime registry / OpenTabRegistry 由来の runtime_active /
    /// tab_open enrichment は含めない（戻り値の runtime_states は空）。
    #[tokio::test]
    async fn execution_projection_preserves_state_without_runtime_enrichment() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = read_only_test_uuid(5);
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    repository_root: None,
                    execution_id: execution_id.clone(),
                    workflow_name: "adapter-boundary".to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: make_test_workflow("adapter-boundary"),
                    timestamp: 500.0,
                },
                WorkflowEvent::NodeStarted {
                    worktree: None,
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
        .await
        .unwrap();

        let read = app.state::<AppState>().workflow_usecase.read_usecase();
        let view = read
            .get_execution_state(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view.id, execution_id);
        assert_eq!(view.node_executions.len(), 1);
        assert_eq!(view.node_executions[0].node_name, "main");
        assert!(view.node_executions[0].session_id.is_none());

        // 存在しない execution_id は Ok(None)。
        let missing = read
            .get_execution_state(&read_only_test_uuid(97))
            .await
            .unwrap();
        assert!(missing.is_none());
    }
}
