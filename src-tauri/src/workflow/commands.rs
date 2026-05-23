use super::builtin;
use super::command::{WorkflowCommand, WorkflowCommandResult};
use super::diagnostics;
use super::engine::WorkflowEngine;
use super::facet::{self, FacetKind};
use super::log::WorkflowEventLog;
use super::run::{RunListFilter, RunStatusFilter, WorkflowRunSummary};
use super::schema::{FacetSummary, Summary, Workflow};
use super::storage;
use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::agent_sdk::AgentProcessMap;
use crate::backends::{AgentBackendRegistry, ImageAttachment};
use crate::config::AppConfig;
use crate::permission::PermissionMode;
use crate::protocol::WorkflowStateView;
use crate::session::OpenTabRegistry;
use crate::session::{resolve_data_dir, SessionStore};
use crate::workflow::session_errors::redacted_workflow_tab_error;
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

fn parse_facet_kind(kind: &str) -> Result<FacetKind, String> {
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        "contract" => Ok(FacetKind::Contract),
        _ => Err(format!("Unknown facet kind: {kind}")),
    }
}

fn parse_workflow_approval_permission_mode(
    permission_mode: Option<String>,
) -> Result<PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_default();
    PermissionMode::parse(&permission_value).map_err(|e| e.to_string())
}

// ---- ファセットコマンドの内部実装（テスト可能な純粋関数として切り出し） ----
//
// Tauri コマンドはこれらの inner 関数に委譲する。インテグレーションを
// テンポラリディレクトリ上で再現することで、4 種それぞれの正常経路到達と、
// 廃止済み種別および未知種別での I/O 非発生を直接検証できるようにする。

fn list_facets_inner(kind: &str, base_dir: &Path) -> Result<Vec<String>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    super::facet::list_facets(facet_kind, base_dir).map_err(|e| e.to_string())
}

fn get_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<String, String> {
    let facet_kind = parse_facet_kind(kind)?;
    super::facet::load_facet(facet_kind, key, base_dir).map_err(|e| e.to_string())
}

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
        let existing =
            super::facet::list_facets(facet_kind, base_dir).map_err(|e| e.to_string())?;
        if existing.contains(&key.to_string()) {
            return Err(format!("ファセット '{key}' は既に存在します"));
        }
    }
    super::facet::save_facet(facet_kind, key, content, base_dir).map_err(|e| e.to_string())
}

fn delete_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは削除できません".to_string());
    }
    super::facet::delete_facet(facet_kind, key, base_dir).map_err(|e| e.to_string())
}

fn list_facet_summaries_inner(kind: &str, base_dir: &Path) -> Result<Vec<FacetSummary>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    super::facet::list_facet_summaries(facet_kind, base_dir).map_err(|e| e.to_string())
}

fn duplicate_facet_inner(
    kind: &str,
    source_key: &str,
    new_key: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    super::facet::validate_facet_key(new_key).map_err(|e| e.to_string())?;
    let existing = super::facet::list_facets(facet_kind, base_dir).map_err(|e| e.to_string())?;
    if existing.contains(&new_key.to_string()) {
        return Err(format!("ファセット '{new_key}' は既に存在します"));
    }
    let content =
        super::facet::load_facet(facet_kind, source_key, base_dir).map_err(|e| e.to_string())?;
    super::facet::save_facet(facet_kind, new_key, &content, base_dir).map_err(|e| e.to_string())
}

/// `open_facet_in_editor` の中核ロジック。エディタ起動はテストで差し替え可能にするため
/// `opener` を引数で受け取る（production では実エディタ起動を渡す）。
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
        super::facet::resolve_facet_path(facet_kind, key, base_dir).map_err(|e| e.to_string())?;
    let path_str = file_path.to_string_lossy().to_string();
    opener(&path_str)
}

fn validation_error_string(e: super::validation::ValidationError) -> String {
    format!("validation_error: {e}")
}

/// [05] `list_workflows` Tauri command が委譲する実 API inner 関数。
///
/// `running_names` は engine の in-memory active map 由来の workflow 名集合、
/// `workflows_dir` は workflow YAML を読む対象ディレクトリ。Tauri State / Arc に
/// 依存しない pure 関数として切り出すことで、API 経路と CLI 経路の意味的等価性を
/// 直接テストできるようにする（spec [05] API / CLI の意味的等価性境界）。
pub(crate) fn list_workflows_inner(
    running_names: &std::collections::HashSet<String>,
    workflows_dir: &std::path::Path,
) -> Result<Vec<Summary>, String> {
    let mut summaries = storage::list_workflows(workflows_dir).map_err(|e| e.to_string())?;
    for s in &mut summaries {
        s.is_running = running_names.contains(&s.name);
    }
    Ok(summaries)
}

#[tauri::command]
pub async fn list_workflows(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
) -> Result<Vec<Summary>, String> {
    let running_names = engine.running_workflow_names().await;
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || list_workflows_inner(&running_names, &dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow(name: String) -> Result<Workflow, String> {
    let dir = storage::workflows_dir();
    let facets_base = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&name).map_err(validation_error_string)?;
        let file_path = dir.join(format!("{name}.yml"));
        if file_path.exists() {
            return storage::load_workflow(&file_path, &facets_base).map_err(|e| e.to_string());
        }
        builtin::load_builtin_workflow_resolved(&name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("ワークフロー '{name}' が見つかりません"))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_workflow(
    workflow: Workflow,
    original_name: Option<String>,
) -> Result<(), String> {
    // workflow.name のバリデーション（パストラバーサル防止）
    super::validation::validate_name(&workflow.name).map_err(validation_error_string)?;
    // ビルトイン編集ガード: original_name がビルトインの場合は編集拒否
    if let Some(ref orig) = original_name {
        if builtin::is_builtin_workflow(orig) {
            return Err("ビルトインワークフローは編集できません".to_string());
        }
        // パストラバーサル防止: original_name のバリデーション
        super::validation::validate_name(orig).map_err(validation_error_string)?;
    }
    // ビルトイン名との重複チェック
    if builtin::is_builtin_workflow(&workflow.name) {
        return Err(format!(
            "ワークフロー名 '{}' はビルトイン名と重複するため使用できません",
            workflow.name
        ));
    }
    let dir = storage::workflows_dir();
    let facets_base = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        // 新規作成 or リネーム時は既存名との重複チェック
        let is_new = original_name.is_none();
        let is_rename = original_name.as_ref().is_some_and(|o| *o != workflow.name);
        if (is_new || is_rename) && dir.join(format!("{}.yml", workflow.name)).exists() {
            return Err(format!("ワークフロー '{}' は既に存在します", workflow.name));
        }

        // 先に保存（validate含む）を実行し、成功した場合のみ旧ファイルを削除
        storage::save_workflow(&dir, &facets_base, &workflow).map_err(|e| e.to_string())?;

        // リネーム時は旧ファイルを削除（保存成功後）
        if let Some(ref orig) = original_name {
            if *orig != workflow.name {
                let old_path = dir.join(format!("{orig}.yml"));
                if old_path.exists() {
                    std::fs::remove_file(&old_path)
                        .map_err(|e| format!("旧ファイル削除失敗: {e}"))?;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_workflow(name: String) -> Result<(), String> {
    if builtin::is_builtin_workflow(&name) {
        return Err("ビルトインワークフローは削除できません".to_string());
    }
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        storage::delete_workflow(&dir, &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_workflow_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    name: String,
) -> Result<(), String> {
    if builtin::is_builtin_workflow(&name) {
        return Err("ビルトインワークフローは外部エディタで開けません".to_string());
    }
    let dir = storage::workflows_dir();
    let file_path = storage::resolve_workflow_path(&dir, &name).map_err(|e| e.to_string())?;

    let path_str = file_path.to_string_lossy().to_string();
    let config = state.get_config()?;
    crate::external_editor::open_path_with_opener(
        &app,
        &path_str,
        &config.app.external_editor,
        "ワークフロー",
    )
}

// ---- ワークフロー実行コマンド ----

fn parse_trigger_source(value: Option<String>) -> Result<super::run::TriggerSource, String> {
    match value.as_deref() {
        Some("cli") => Ok(super::run::TriggerSource::Cli),
        Some("remote") => Ok(super::run::TriggerSource::Remote),
        Some("agent") => Ok(super::run::TriggerSource::Agent),
        Some("desktop_ui") | Some("desktop-ui") | None => Ok(super::run::TriggerSource::DesktopUi),
        Some(other) => Err(format!("unknown trigger_source: {other}")),
    }
}

fn parse_workflow_start_permission_mode(
    permission_mode: Option<String>,
) -> Result<PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_else(|| PermissionMode::Readonly.to_string());
    PermissionMode::parse(&permission_value).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn start_workflow_adapter<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    engine: &Arc<WorkflowEngine>,
    workflow_name: String,
    worktree_path: String,
    task: Option<String>,
    trigger_source: Option<String>,
    permission_mode: Option<String>,
) -> Result<String, String> {
    super::validation::validate_name(&workflow_name).map_err(validation_error_string)?;
    let trigger = parse_trigger_source(trigger_source)?;
    let permission_mode = parse_workflow_start_permission_mode(permission_mode)?;
    // [04] managed worktree 検証は dispatch 経路（= 全 command 入口の合流地点）で行う。
    // Tauri adapter は文字列引数のまま command を組み立て、engine 入口で正規化される境界に揃える。
    // [05] adapter-facing 拒否境界: 外部 adapter は internal-only variant を組み立てないため
    // dispatch_external を経由する。
    engine
        .dispatch_external(
            app,
            session_store,
            handles,
            WorkflowCommand::StartRun {
                workflow_file_stem: workflow_name,
                worktree_path,
                task,
                trigger_source: trigger,
                permission_mode,
            },
        )
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| match result {
            WorkflowCommandResult::RunStarted { run_id } => Ok(run_id),
            // dispatch routing が壊れていない限り `StartRun` → `RunStarted` のみが
            // 返るはずで、ここに到達する場合は engine 内部不整合（spec [04] 責務配置：
            // sentinel 禁止）として明示的に Err にする。空文字列 run_id を成功扱い
            // することはしない。
            WorkflowCommandResult::Accepted => Err(
                "start_workflow received non-RunStarted dispatch result; internal inconsistency"
                    .to_string(),
            ),
        })
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    workflow_name: String,
    worktree_path: String,
    task: Option<String>,
    trigger_source: Option<String>,
    permission_mode: Option<String>,
) -> Result<String, String> {
    start_workflow_adapter(
        &app,
        handles.inner(),
        session_store.inner(),
        engine.inner(),
        workflow_name,
        worktree_path,
        task,
        trigger_source,
        permission_mode,
    )
    .await
}

async fn abort_workflow_adapter<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    engine: &Arc<WorkflowEngine>,
    run_id: String,
) -> Result<(), String> {
    validate_run_id(&run_id)?;
    engine
        .dispatch_external(
            app,
            session_store,
            handles,
            WorkflowCommand::AbortRun {
                run_id,
                expected_node_name: None,
            },
        )
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("abort_workflow failed: code=ABORT_WORKFLOW_FAILED");
            msg
        })
        .and_then(|result| match result {
            WorkflowCommandResult::Accepted => Ok(()),
            // dispatch routing が壊れていない限り `AbortRun` → `Accepted` のみが返るはず。
            // ここに到達する場合は engine 内部不整合（spec [04] sentinel 禁止）として
            // 明示的に Err にする。
            WorkflowCommandResult::RunStarted { .. } => Err(
                "abort_workflow received non-Accepted dispatch result; internal inconsistency"
                    .to_string(),
            ),
        })
}

#[tauri::command]
pub async fn abort_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    run_id: String,
) -> Result<(), String> {
    abort_workflow_adapter(
        &app,
        handles.inner(),
        session_store.inner(),
        engine.inner(),
        run_id,
    )
    .await
}

#[tauri::command]
pub async fn get_workflow_state(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    open_tabs: tauri::State<'_, Arc<OpenTabRegistry>>,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    validate_run_id(&run_id)?;
    match engine.get_state_by_run_id(&run_id).await {
        Some(state) => Ok(Some(
            crate::workflow_state_events::build_workflow_state_view(
                state,
                handles.inner(),
                open_tabs.inner(),
            )
            .await,
        )),
        None => Ok(None),
    }
}

/// approval UI / Tauri command 境界からの判断入力 DTO。
///
/// [04] Command / Event Boundary: engine 内部の `ApprovalDecision` には依存させず、
/// command 境界専用の DTO として `WorkflowCommand` への変換責務だけを担う。
/// wire 形式: `{"approve":{"comment":...}}` / `{"reject":{"reason":...}}` / `"abort"`。
/// 旧 unit variant `"approve"` は受理しない。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionInput {
    Approve {
        #[serde(default)]
        comment: Option<String>,
    },
    Reject {
        reason: String,
    },
    Abort,
}

impl ApprovalDecisionInput {
    /// approval UI 判断を `WorkflowCommand` に変換する。
    ///
    /// `Approve` / `Reject` は `ApproveNode` / `RejectNode` に対応し、`Abort` は
    /// 「現在の承認待ち node を対象にした AbortRun」として中断系 command に揃える。
    /// `step_name` は approval UI 上の対象 node 名であり、`Abort` 経路では
    /// engine 側の stale target 検証用に `expected_node_name` として伝播する。
    fn into_command(self, run_id: String, step_name: String) -> WorkflowCommand {
        match self {
            Self::Approve { comment } => WorkflowCommand::ApproveNode {
                run_id,
                node_name: Some(step_name),
                comment,
            },
            Self::Reject { reason } => WorkflowCommand::RejectNode {
                run_id,
                node_name: Some(step_name),
                reason,
            },
            Self::Abort => WorkflowCommand::AbortRun {
                run_id,
                expected_node_name: Some(step_name),
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn approve_workflow_step_adapter<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    engine: &Arc<WorkflowEngine>,
    run_id: String,
    decision: ApprovalDecisionInput,
    step_name: String,
) -> Result<(), String> {
    validate_run_id(&run_id)?;
    // [04] approval UI からの decision は command 境界専用 DTO で受け取り、
    // `WorkflowCommand` に変換して engine へ受け渡す。engine 内部 domain 型
    // `ApprovalDecision` を Tauri 境界に露出させない（同一意図は呼び出し経路に依らず
    // engine から等価に扱われる）。
    let command = decision.into_command(run_id, step_name);
    engine
        .dispatch_external(app, session_store, handles, command)
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| match result {
            WorkflowCommandResult::Accepted => Ok(()),
            // dispatch routing が壊れていない限り approval 系 command → `Accepted` のみが
            // 返るはず。ここに到達する場合は engine 内部不整合（spec [04] sentinel 禁止）と
            // して明示的に Err にする。
            WorkflowCommandResult::RunStarted { .. } => Err(
                "approve_workflow_step received non-Accepted dispatch result; internal inconsistency"
                    .to_string(),
            ),
        })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn approve_workflow_step(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    run_id: String,
    decision: ApprovalDecisionInput,
    step_name: String,
) -> Result<(), String> {
    approve_workflow_step_adapter(
        &app,
        handles.inner(),
        session_store.inner(),
        engine.inner(),
        run_id,
        decision,
        step_name,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_workflow_approval_chat_message(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    run_id: String,
    content: String,
    permission_mode: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::file_mention::MentionReference>>,
) -> Result<crate::agent_sdk::SendMessageResponse, String> {
    // Spec issues-1011 line 121: 起動以外の workflow 操作 API は run_id を主語に取る。
    // chat_session_id / worktree_path は run_id から engine が解決する。
    validate_run_id(&run_id)?;
    let permission_mode = parse_workflow_approval_permission_mode(permission_mode)?;

    let (chat_session_id, worktree_path) = engine
        .resolve_chat_session_for_approval(&run_id)
        .await
        .map_err(|e| e.to_string())?;

    engine
        .validate_approval_chat_instruction(&chat_session_id, &content)
        .await
        .map_err(|e| e.to_string())?;

    let response = dispatch_agent_message(
        AgentMessageDispatchContext {
            app: &app,
            session_store: session_store.inner(),
            registry: registry.inner(),
            handles: handles.inner(),
        },
        AgentMessageDispatchRequest {
            chat_session_id: Some(chat_session_id),
            worktree_path,
            content,
            permission_mode,
            backend_id: None,
            images,
            mentions,
        },
    )
    .await?;
    crate::workflow_state_events::emit_after_workflow_step_message(
        &app,
        engine.inner(),
        &response.session,
        handles.inner(),
        app.state::<Arc<OpenTabRegistry>>().inner(),
    )
    .await;
    Ok(response)
}

#[tauri::command]
pub async fn open_workflow_step_tab(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    open_tabs: tauri::State<'_, Arc<OpenTabRegistry>>,
    chat_session_id: String,
) -> Result<(), String> {
    let lifecycle = crate::workflow_step_lifecycle_adapters::TauriWorkflowStepLifecycle::new(
        &app,
        session_store.inner().as_ref(),
        handles.inner(),
        open_tabs.inner().as_ref(),
    );
    let target = lifecycle
        .open_tab(&chat_session_id)
        .await
        .map_err(|_| redacted_workflow_tab_error("workflow_step_session_rejected"))?;
    crate::workflow_state_events::emit_workflow_step_target_state(
        &app,
        engine.inner(),
        &target,
        handles.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(())
}

/// `run_id` の形式検証（path traversal / 不正文字対策）。
/// UUID（RFC 4122）形式のみ許容する。Run Store 内部でも canonicalize 後の
/// `workflow_runs/` 配下チェックを行うが、command 入口でも形式不正を弾く。
fn validate_run_id(run_id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(run_id)
        .map(|_| ())
        .map_err(|_| "Invalid run_id format (must be UUID)".to_string())
}

// ---- [05] read-only Run 観測 API ----
//
// `run_id` 主語で workflow run を観測する read-only API。spec [05] により
// 旧 `list_active_workflow_runs` / `list_completed_workflow_runs` /
// `list_workflow_runs_for_worktree` / `get_workflow_execution_log` /
// `get_workflow_execution_state` を破壊的に置換する。

/// [05] worktree-scoped 認可境界: run metadata の `worktree_path` が caller の
/// 認可済み (= 既存 managed worktree) のいずれかに合致するかを既存 helper
/// `canonicalize_managed_worktree_path` で検証する。合致しない場合は `Ok(None)`
/// として返し（not-found と同表現で存在情報を漏らさない）、observation 経路を
/// 当該 caller から閉じる（spec [05] L104-108 観測経路の認可境界 / L182
/// worktree-scoped Tauri 認証依拠）。
async fn authorize_run_for_caller(
    engine: &Arc<WorkflowEngine>,
    config: &Arc<AppConfig>,
    run_id: &str,
) -> Result<Option<WorkflowRunSummary>, String> {
    let Some(summary) = engine.get_run(run_id).await else {
        return Ok(None);
    };
    match super::worktree::canonicalize_managed_worktree_path(
        config.clone(),
        summary.worktree_path.clone(),
    )
    .await
    {
        Ok(_) => Ok(Some(summary)),
        Err(_) => Ok(None),
    }
}

/// [05] read-only API: 過去 / 進行中の workflow run 一覧を返す。
/// `worktree_path` は必須。caller の認可済み worktree のみを対象にすることで
/// 別 worktree の run を観測できる経路を閉じる（spec [05] L104-108 観測経路の
/// 認可境界）。`status` は optional な filter。
#[tauri::command]
pub async fn list_workflow_runs(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    status: Option<String>,
    worktree_path: String,
) -> Result<Vec<WorkflowRunSummary>, String> {
    let status = match status.as_deref() {
        None | Some("") => None,
        Some("active") => Some(RunStatusFilter::Active),
        Some("terminal") => Some(RunStatusFilter::Terminal),
        Some(other) => return Err(format!("Invalid status filter: {other}")),
    };
    let worktree_path =
        super::worktree::canonicalize_managed_worktree_path(config.inner().clone(), worktree_path)
            .await?;
    Ok(engine
        .list_runs(RunListFilter {
            status,
            worktree_path: Some(worktree_path),
        })
        .await)
}

/// [05] read-only API: 単一 run の summary metadata を返す。
/// active / terminal のいずれであっても返す。該当 run なし、または run の
/// worktree_path が caller の認可済み worktree に合致しない場合は `Ok(None)`
/// （spec [05] L104-108 / L182）。
#[tauri::command]
pub async fn get_workflow_run(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    run_id: String,
) -> Result<Option<WorkflowRunSummary>, String> {
    validate_run_id(&run_id)?;
    authorize_run_for_caller(engine.inner(), config.inner(), &run_id).await
}

/// spec issues-1023: 永続化 event の timestamp は engine 内 `current_timestamp()`
/// 由来の秒単位 f64 だが、frontend 表示用に projection 境界で ms 単位の
/// `WorkflowEventView` に変換する。view 型分離で「秒 / ms」の二重意味を排除する。
pub use super::event_projection::WorkflowEventView;

/// [05] read-only API: 指定 run の event log を返す。
/// `workflow_logs/{run_id}.ndjson` を engine 一次 owner の log source として読む。
/// 該当 run なし、または認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + run metadata の
/// `worktree_path` 一致を二重に検証してから event log を返す。
#[tauri::command]
pub async fn get_workflow_run_log(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    get_workflow_run_log_inner(&app, engine.inner(), config.inner(), worktree_path, run_id).await
}

/// [05] generic 入口: テストで MockRuntime AppHandle を渡せるように runtime を generic 化する
/// 内部経路。`#[tauri::command]` 側は Wry に固定して本関数に委譲する。
async fn get_workflow_run_log_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &Arc<WorkflowEngine>,
    config: &Arc<AppConfig>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<Vec<WorkflowEventView>>, String> {
    validate_run_id(&run_id)?;
    let canonical =
        super::worktree::canonicalize_managed_worktree_path(config.clone(), worktree_path).await?;
    let summary = match authorize_run_for_caller(engine, config, &run_id).await? {
        Some(s) => s,
        None => return Ok(None),
    };
    if summary.worktree_path != canonical {
        return Ok(None);
    }
    let data_dir = resolve_data_dir(app)?;
    let event_log = WorkflowEventLog::new(&data_dir);
    let events = tokio::task::spawn_blocking(move || event_log.read_log(&run_id))
        .await
        .map_err(|e| format!("task join error: {e}"))??;
    Ok(Some(super::event_projection::events_with_ms_timestamps(
        events,
    )))
}

/// [05] read-only API: 指定 run の現在 state を返す。
/// NDJSON event log から `reconstruct_state_from_events` で投影する。
///
/// 観測結果の露出範囲境界（spec [05]）に従い、戻り値は engine が一次 owner として
/// 保持している event log / state の純粋投影のみを含む。`AgentProcessMap` /
/// `OpenTabRegistry` 由来の runtime_active / tab_open enrichment は read-only API
/// 経路では含めない（同種情報は live emission 経路の `WorkflowStateChanged` event を
/// 通じて UI に届ける）。
///
/// 認可不一致は `None`（spec [05] L104-108 / L182）。
///
/// spec issues-1023 L132/L150: 観測 invoke は caller の現 worktree path を必須引数
/// として受け取り、`canonicalize_managed_worktree_path` + run metadata の
/// `worktree_path` 一致を二重に検証してから state を返す。
#[tauri::command]
pub async fn get_workflow_run_state(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    get_workflow_run_state_inner(&app, engine.inner(), config.inner(), worktree_path, run_id).await
}

/// [05] generic 入口: テストで MockRuntime AppHandle を渡せるように runtime を generic 化する
/// 内部経路。`#[tauri::command]` 側は Wry に固定して本関数に委譲する。
async fn get_workflow_run_state_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &Arc<WorkflowEngine>,
    config: &Arc<AppConfig>,
    worktree_path: String,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    validate_run_id(&run_id)?;
    let canonical =
        super::worktree::canonicalize_managed_worktree_path(config.clone(), worktree_path).await?;
    let summary = match authorize_run_for_caller(engine, config, &run_id).await? {
        Some(s) => s,
        None => return Ok(None),
    };
    if summary.worktree_path != canonical {
        return Ok(None);
    }
    let data_dir = resolve_data_dir(app)?;
    let state = tokio::task::spawn_blocking(move || {
        let event_log = WorkflowEventLog::new(&data_dir);
        let events = event_log.read_log(&run_id)?;
        // [04] schema 境界: 復元は `RunStarted.workflow_definition` snapshot 経由のみ。
        super::event_projection::reconstruct_state_from_events(&run_id, &events)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(state.map(|state| {
        WorkflowStateView::from_parts(
            crate::workflow_state_presenter::workflow_state_to_view(state),
            std::collections::HashMap::new(),
        )
    }))
}

/// spec issues-1023: 選択 step の入出力・遷移結果・所要時間を 1 つの View で返す
/// 観測用 API。frontend で `WorkflowState` を再走査する代わりに、`worktree_path` /
/// `run_id` / `node_name` / `run_index` を渡してこの View を受け取る境界。
pub use super::event_projection::WorkflowStepDetailView;

#[tauri::command]
pub async fn get_workflow_step_detail(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    get_workflow_step_detail_inner(
        &app,
        engine.inner(),
        config.inner(),
        worktree_path,
        run_id,
        node_name,
        run_index,
    )
    .await
}

async fn get_workflow_step_detail_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &Arc<WorkflowEngine>,
    config: &Arc<AppConfig>,
    worktree_path: String,
    run_id: String,
    node_name: String,
    run_index: Option<u32>,
) -> Result<Option<WorkflowStepDetailView>, String> {
    validate_run_id(&run_id)?;
    let canonical =
        super::worktree::canonicalize_managed_worktree_path(config.clone(), worktree_path).await?;
    let summary = match authorize_run_for_caller(engine, config, &run_id).await? {
        Some(s) => s,
        None => return Ok(None),
    };
    if summary.worktree_path != canonical {
        return Ok(None);
    }
    let data_dir = resolve_data_dir(app)?;
    let detail = tokio::task::spawn_blocking(move || {
        let event_log = WorkflowEventLog::new(&data_dir);
        let events = event_log.read_log(&run_id)?;
        let Some(state) = super::event_projection::reconstruct_state_from_events(&run_id, &events)?
        else {
            return Ok::<Option<WorkflowStepDetailView>, String>(None);
        };
        Ok(super::event_projection::compute_step_detail(
            &state, &events, &node_name, run_index,
        ))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(detail)
}

// ---- ファセットCRUDコマンド ----

#[tauri::command]
pub async fn list_facets(kind: String) -> Result<Vec<String>, String> {
    let base_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || list_facets_inner(&kind, &base_dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_facet(kind: String, key: String) -> Result<String, String> {
    let base_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || get_facet_inner(&kind, &key, &base_dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_facet(
    kind: String,
    key: String,
    content: String,
    is_new: Option<bool>,
) -> Result<(), String> {
    let base_dir = facet::facets_base_dir();
    let is_new = is_new.unwrap_or(false);
    tokio::task::spawn_blocking(move || save_facet_inner(&kind, &key, &content, is_new, &base_dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_facet(kind: String, key: String) -> Result<(), String> {
    let base_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || delete_facet_inner(&kind, &key, &base_dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

// ---- 新規コマンド ----

#[tauri::command]
pub async fn diagnose_all_cmd() -> Result<diagnostics::DiagnosticReport, String> {
    let wf_dir = storage::workflows_dir();
    let facets_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || Ok(diagnostics::diagnose_all(&wf_dir, &facets_dir)))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn list_facet_summaries(kind: String) -> Result<Vec<FacetSummary>, String> {
    let base_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || list_facet_summaries_inner(&kind, &base_dir))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn duplicate_workflow(source_name: String, new_name: String) -> Result<(), String> {
    super::validation::validate_name(&source_name).map_err(validation_error_string)?;
    super::validation::validate_name(&new_name).map_err(validation_error_string)?;
    let dir = storage::workflows_dir();
    let facets_base = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        // 重複チェック
        if dir.join(format!("{new_name}.yml")).exists() {
            return Err(format!("ワークフロー '{new_name}' は既に存在します"));
        }
        if builtin::is_builtin_workflow(&new_name) {
            return Err(format!(
                "ワークフロー名 '{new_name}' はビルトインと重複します"
            ));
        }

        // ソースの読み込み
        let mut wf = {
            let file_path = dir.join(format!("{source_name}.yml"));
            if file_path.exists() {
                storage::load_workflow(&file_path, &facets_base).map_err(|e| e.to_string())?
            } else {
                builtin::load_builtin_workflow_resolved(&source_name)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("ソースワークフロー '{source_name}' が見つかりません"))?
            }
        };

        wf.name = new_name;
        wf.builtin = false;
        storage::save_workflow(&dir, &facets_base, &wf).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn duplicate_facet(
    kind: String,
    source_key: String,
    new_key: String,
) -> Result<(), String> {
    let base_dir = facet::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        duplicate_facet_inner(&kind, &source_key, &new_key, &base_dir)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_facet_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    kind: String,
    key: String,
) -> Result<(), String> {
    parse_facet_kind(&kind)?;
    let base_dir = facet::facets_base_dir();
    let config = state.get_config()?;
    open_facet_in_editor_inner(&kind, &key, &base_dir, |path_str| {
        crate::external_editor::open_path_with_opener(
            &app,
            path_str,
            &config.app.external_editor,
            "ファセット",
        )
    })
}

#[tauri::command]
pub async fn render_facet_preview(
    content: String,
    sample_values: std::collections::HashMap<String, String>,
) -> Result<String, String> {
    Ok(super::facet::render_template_variables(
        &content,
        &sample_values,
    ))
}

// ---- Run Store / WorkflowRun コマンド ----

/// worktree_path から active な run_id を解決する（双方向 lookup の一方向）。
#[tauri::command]
pub async fn resolve_active_run_by_worktree(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    config: tauri::State<'_, Arc<AppConfig>>,
    worktree_path: String,
) -> Result<Option<String>, String> {
    let worktree_path =
        super::worktree::canonicalize_managed_worktree_path(config.inner().clone(), worktree_path)
            .await?;
    Ok(engine.run_id_for_worktree(&worktree_path).await)
}

/// run_id から worktree_path を解決する（双方向 lookup のもう一方向）。
/// active / 終了済みの両方について metadata 経由で解決する。
/// path traversal 対策として command 入口で UUID 形式を検証する。
#[tauri::command]
pub async fn resolve_worktree_by_run(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    run_id: String,
) -> Result<Option<String>, String> {
    validate_run_id(&run_id)?;
    Ok(engine.resolve_worktree_by_run(&run_id).await)
}

#[tauri::command]
pub fn get_automation_config_dir() -> Result<String, String> {
    let dir = storage::workflows_dir();
    Ok(dir.to_string_lossy().to_string())
}

fn validate_template_variables(content: &str) -> Result<(), String> {
    let errors = super::facet::find_undefined_template_variables(content);
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
    use super::*;
    use crate::workflow::event::WorkflowEvent;
    use crate::workflow::log::WorkflowEventLog;
    use crate::workflow::resolver::{
        ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
        WorkflowDefinitionResolverError,
    };
    use crate::workflow::run::{RunStatus, TriggerSource};
    use crate::workflow::schema::{NodeDefinition, NodeType};
    use crate::workflow::state::WorkflowExecutionState;
    use std::path::Path;
    use tempfile::TempDir;

    struct StaticWorkflowResolver;

    #[async_trait::async_trait]
    impl WorkflowDefinitionResolver for StaticWorkflowResolver {
        async fn resolve(
            &self,
            _file_stem: &str,
        ) -> Result<Workflow, WorkflowDefinitionResolverError> {
            Ok(approval_only_workflow())
        }
    }

    struct TestWorktreeResolver;

    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for TestWorktreeResolver {
        async fn resolve(
            &self,
            worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Ok(worktree_path)
        }
    }

    type AdapterTestApp = tauri::App<tauri::test::MockRuntime>;

    fn approval_only_workflow() -> Workflow {
        Workflow {
            name: "adapter-boundary".to_string(),
            description: "adapter command test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Approval,
                instruction: Some("review".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    fn rejectable_adapter_workflow() -> Workflow {
        Workflow {
            name: "adapter-boundary".to_string(),
            description: "adapter command test".to_string(),
            builtin: false,
            nodes: vec![
                NodeDefinition {
                    name: "review".to_string(),
                    node_type: NodeType::Approval,
                    instruction: Some("review".to_string()),
                    transition_rules: vec![crate::workflow::schema::TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    }],
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    node_type: NodeType::Agent,
                    instruction: Some("fix".to_string()),
                    ..NodeDefinition::default()
                },
            ],
        }
    }

    fn make_adapter_app() -> AdapterTestApp {
        let mut config = crate::config::ReleashConfig::default();
        config.agents.codex.models = vec!["default".to_string(), "gpt-5.5".to_string()];
        config.agents.default = Some("codex".to_string());
        let app_config = Arc::new(crate::config::AppConfig::new(
            config,
            TempDir::new().unwrap().path().join("config.toml"),
        ));
        let registry = Arc::new(crate::backends::build_registry(Arc::clone(&app_config)));
        let data_dir =
            std::env::temp_dir().join(format!("releash-command-adapter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        tauri::test::mock_builder()
            .manage(crate::session::TestDataDir(data_dir))
            .manage(app_config)
            .manage(registry)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    fn make_adapter_engine() -> Arc<WorkflowEngine> {
        Arc::new(WorkflowEngine::new(
            Arc::new(StaticWorkflowResolver),
            Arc::new(TestWorktreeResolver),
        ))
    }

    fn make_adapter_deps() -> (Arc<SessionStore>, Arc<Mutex<AgentProcessMap>>) {
        (
            Arc::new(SessionStore::default()),
            Arc::new(Mutex::new(AgentProcessMap::new())),
        )
    }

    async fn configure_run_store(
        app: &AdapterTestApp,
        engine: &Arc<WorkflowEngine>,
    ) -> std::path::PathBuf {
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine
            .set_run_store_data_dir(data_dir.join("workflow_runs"))
            .await;
        data_dir
    }

    fn create_adapter_parent_session(
        app: &AdapterTestApp,
        session_store: &SessionStore,
        worktree_path: &str,
    ) -> crate::session::ChatSession {
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        crate::session::create_session_internal_with_permission(
            session_store,
            &data_dir,
            worktree_path,
            None,
            PermissionMode::Edit,
        )
        .unwrap()
    }

    fn read_adapter_events(data_dir: &Path, run_id: &str) -> Vec<WorkflowEvent> {
        WorkflowEventLog::new(data_dir).read_log(run_id).unwrap()
    }

    async fn start_adapter_run(
        app: &AdapterTestApp,
        engine: &Arc<WorkflowEngine>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
    ) -> String {
        start_workflow_adapter(
            app.handle(),
            handles,
            session_store,
            engine,
            "adapter-boundary".to_string(),
            worktree_path.to_string(),
            Some("task".to_string()),
            Some("desktop_ui".to_string()),
            Some("edit".to_string()),
        )
        .await
        .expect("adapter start_workflow must succeed")
    }

    async fn start_direct_run(
        app: &AdapterTestApp,
        engine: &Arc<WorkflowEngine>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
    ) -> String {
        let result = engine
            .dispatch(
                app.handle(),
                session_store,
                handles,
                WorkflowCommand::StartRun {
                    workflow_file_stem: "adapter-boundary".to_string(),
                    worktree_path: worktree_path.to_string(),
                    task: Some("task".to_string()),
                    trigger_source: TriggerSource::DesktopUi,
                    permission_mode: PermissionMode::Edit,
                },
            )
            .await
            .expect("direct StartRun dispatch must succeed");
        match result {
            WorkflowCommandResult::RunStarted { run_id } => run_id,
            other => panic!("StartRun must return RunStarted, got {other:?}"),
        }
    }

    fn event_kinds(events: &[WorkflowEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                WorkflowEvent::RunStarted { .. } => "RunStarted",
                WorkflowEvent::NodeStarted { .. } => "NodeStarted",
                WorkflowEvent::NodeCompleted { .. } => "NodeCompleted",
                WorkflowEvent::NodeFailed { .. } => "NodeFailed",
                WorkflowEvent::ApprovalRequested { .. } => "ApprovalRequested",
                WorkflowEvent::ApprovalResolved { .. } => "ApprovalResolved",
                WorkflowEvent::RunCompleted { .. } => "RunCompleted",
                WorkflowEvent::RunFailed { .. } => "RunFailed",
                WorkflowEvent::RunAborted { .. } => "RunAborted",
                WorkflowEvent::OutputCollected { .. } => "OutputCollected",
                WorkflowEvent::ParallelStarted { .. } => "ParallelStarted",
                WorkflowEvent::ParallelChildStarted { .. } => "ParallelChildStarted",
                WorkflowEvent::ParallelChildCompleted { .. } => "ParallelChildCompleted",
                WorkflowEvent::ParallelCompleted { .. } => "ParallelCompleted",
                WorkflowEvent::ContractRepairRequested { .. } => "ContractRepairRequested",
                WorkflowEvent::CliMutationRequested { .. } => "CliMutationRequested",
            })
            .collect()
    }

    /// `event_kinds` から `CliMutationRequested` 発生（CLI 経路のみ追加される
    /// 観測 event）を除外する。CLI / UI 経路の engine 出力等価性を比較する
    /// 際に使用する（review R4-01）。
    fn event_kinds_excluding_cli_mutation(events: &[WorkflowEvent]) -> Vec<&'static str> {
        event_kinds(events)
            .into_iter()
            .filter(|kind| *kind != "CliMutationRequested")
            .collect()
    }

    async fn adapter_run_status(engine: &WorkflowEngine, run_id: &str) -> RunStatus {
        if let Some(run) = engine
            .list_active_runs()
            .await
            .into_iter()
            .find(|run| run.run_id == run_id)
        {
            return run.status;
        }
        engine
            .list_completed_runs()
            .await
            .into_iter()
            .find(|run| run.run_id == run_id)
            .map(|run| run.status)
            .expect("run must exist in active or completed store")
    }

    #[test]
    fn parse_facet_kind_valid_kinds() {
        assert_eq!(parse_facet_kind("policy").unwrap(), FacetKind::Policy);
        assert_eq!(parse_facet_kind("knowledge").unwrap(), FacetKind::Knowledge);
        assert_eq!(
            parse_facet_kind("instruction").unwrap(),
            FacetKind::Instruction
        );
        assert_eq!(parse_facet_kind("contract").unwrap(), FacetKind::Contract);
    }

    /// Spec [04] Rule「同一意図 command は呼び出し経路に依らず等価」:
    /// Tauri adapter の start_workflow は typed `WorkflowCommand::StartRun` を組み立て、
    /// direct dispatch と同じ state / Run Store / event vocabulary に到達する。
    #[tokio::test]
    async fn start_workflow_adapter_matches_direct_start_run_dispatch() {
        let adapter_app = make_adapter_app();
        let adapter_engine = make_adapter_engine();
        let adapter_data_dir = configure_run_store(&adapter_app, &adapter_engine).await;
        let (adapter_store, adapter_handles) = make_adapter_deps();
        let adapter_run_id = start_adapter_run(
            &adapter_app,
            &adapter_engine,
            &adapter_store,
            &adapter_handles,
            "/wt/adapter-start",
        )
        .await;

        let direct_app = make_adapter_app();
        let direct_engine = make_adapter_engine();
        let direct_data_dir = configure_run_store(&direct_app, &direct_engine).await;
        let (direct_store, direct_handles) = make_adapter_deps();
        let direct_run_id = start_direct_run(
            &direct_app,
            &direct_engine,
            &direct_store,
            &direct_handles,
            "/wt/direct-start",
        )
        .await;

        let adapter_state = adapter_engine
            .get_state_by_run_id(&adapter_run_id)
            .await
            .expect("adapter start must create state");
        let direct_state = direct_engine
            .get_state_by_run_id(&direct_run_id)
            .await
            .expect("direct start must create state");
        assert_eq!(adapter_state.state, direct_state.state);
        assert_eq!(
            adapter_state.current_step_name,
            direct_state.current_step_name
        );
        assert_eq!(adapter_state.workflow_name, direct_state.workflow_name);
        assert_eq!(
            event_kinds(&read_adapter_events(&adapter_data_dir, &adapter_run_id)),
            event_kinds(&read_adapter_events(&direct_data_dir, &direct_run_id))
        );
    }

    /// Tauri adapter の abort_workflow は `AbortRun { expected_node_name: None }` に射影され、
    /// direct dispatch と同じ terminal state / Run Store / event log を返す。
    #[tokio::test]
    async fn abort_workflow_adapter_matches_direct_abort_run_dispatch() {
        let adapter_app = make_adapter_app();
        let adapter_engine = make_adapter_engine();
        let adapter_data_dir = configure_run_store(&adapter_app, &adapter_engine).await;
        let (adapter_store, adapter_handles) = make_adapter_deps();
        let adapter_run_id = uuid::Uuid::new_v4().to_string();
        let adapter_parent =
            create_adapter_parent_session(&adapter_app, &adapter_store, "/wt/adapter-abort");
        adapter_engine
            .seed_active_execution_for_test(
                adapter_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::Running,
                "/wt/adapter-abort".to_string(),
                adapter_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let direct_app = make_adapter_app();
        let direct_engine = make_adapter_engine();
        let direct_data_dir = configure_run_store(&direct_app, &direct_engine).await;
        let (direct_store, direct_handles) = make_adapter_deps();
        let direct_run_id = uuid::Uuid::new_v4().to_string();
        let direct_parent =
            create_adapter_parent_session(&direct_app, &direct_store, "/wt/direct-abort");
        direct_engine
            .seed_active_execution_for_test(
                direct_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::Running,
                "/wt/direct-abort".to_string(),
                direct_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        abort_workflow_adapter(
            adapter_app.handle(),
            &adapter_handles,
            &adapter_store,
            &adapter_engine,
            adapter_run_id.clone(),
        )
        .await
        .expect("adapter abort must succeed");
        let direct_result = direct_engine
            .dispatch(
                direct_app.handle(),
                &direct_store,
                &direct_handles,
                WorkflowCommand::AbortRun {
                    run_id: direct_run_id.clone(),
                    expected_node_name: None,
                },
            )
            .await
            .expect("direct abort must succeed");
        assert_eq!(direct_result, WorkflowCommandResult::Accepted);

        assert_eq!(
            adapter_engine
                .get_state_by_run_id(&adapter_run_id)
                .await
                .unwrap()
                .state,
            WorkflowExecutionState::Aborted
        );
        assert_eq!(
            direct_engine
                .get_state_by_run_id(&direct_run_id)
                .await
                .unwrap()
                .state,
            WorkflowExecutionState::Aborted
        );
        assert_eq!(
            adapter_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            direct_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            event_kinds(&read_adapter_events(&adapter_data_dir, &adapter_run_id)),
            event_kinds(&read_adapter_events(&direct_data_dir, &direct_run_id))
        );
    }

    /// Tauri adapter の approve_workflow_step は approval DTO を `ApproveNode` に変換し、
    /// direct dispatch と同じ state / Run Store / typed event を返す。
    #[tokio::test]
    async fn approve_workflow_step_adapter_matches_direct_approve_node_dispatch() {
        let adapter_app = make_adapter_app();
        let adapter_engine = make_adapter_engine();
        let adapter_data_dir = configure_run_store(&adapter_app, &adapter_engine).await;
        let (adapter_store, adapter_handles) = make_adapter_deps();
        let adapter_run_id = uuid::Uuid::new_v4().to_string();
        let adapter_parent =
            create_adapter_parent_session(&adapter_app, &adapter_store, "/wt/adapter-approve");
        adapter_engine
            .seed_active_execution_for_test(
                adapter_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/adapter-approve".to_string(),
                adapter_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let direct_app = make_adapter_app();
        let direct_engine = make_adapter_engine();
        let direct_data_dir = configure_run_store(&direct_app, &direct_engine).await;
        let (direct_store, direct_handles) = make_adapter_deps();
        let direct_run_id = uuid::Uuid::new_v4().to_string();
        let direct_parent =
            create_adapter_parent_session(&direct_app, &direct_store, "/wt/direct-approve");
        direct_engine
            .seed_active_execution_for_test(
                direct_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/direct-approve".to_string(),
                direct_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        approve_workflow_step_adapter(
            adapter_app.handle(),
            &adapter_handles,
            &adapter_store,
            &adapter_engine,
            adapter_run_id.clone(),
            ApprovalDecisionInput::Approve {
                comment: Some("lgtm".to_string()),
            },
            "review".to_string(),
        )
        .await
        .expect("adapter approval must succeed");
        let direct_result = direct_engine
            .dispatch(
                direct_app.handle(),
                &direct_store,
                &direct_handles,
                WorkflowCommand::ApproveNode {
                    run_id: direct_run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: Some("lgtm".to_string()),
                },
            )
            .await
            .expect("direct approval must succeed");
        assert_eq!(direct_result, WorkflowCommandResult::Accepted);

        let adapter_state = adapter_engine
            .get_state_by_run_id(&adapter_run_id)
            .await
            .unwrap();
        let direct_state = direct_engine
            .get_state_by_run_id(&direct_run_id)
            .await
            .unwrap();
        assert_eq!(adapter_state.state, WorkflowExecutionState::Completed);
        assert_eq!(direct_state.state, WorkflowExecutionState::Completed);
        assert_eq!(
            adapter_state.step_history.len(),
            direct_state.step_history.len()
        );
        assert_eq!(
            adapter_engine.list_completed_runs().await[0].status,
            direct_engine.list_completed_runs().await[0].status
        );
        assert_eq!(
            event_kinds(&read_adapter_events(&adapter_data_dir, &adapter_run_id)),
            event_kinds(&read_adapter_events(&direct_data_dir, &direct_run_id))
        );
    }

    /// Tauri adapter の Reject decision は `RejectNode` に射影され、
    /// direct dispatch と同じ state / Run Store status / typed event sequence に到達する。
    #[tokio::test]
    async fn approve_workflow_step_adapter_matches_direct_reject_node_dispatch() {
        let adapter_app = make_adapter_app();
        let adapter_engine = make_adapter_engine();
        let adapter_data_dir = configure_run_store(&adapter_app, &adapter_engine).await;
        let (adapter_store, adapter_handles) = make_adapter_deps();
        let adapter_run_id = uuid::Uuid::new_v4().to_string();
        let adapter_parent =
            create_adapter_parent_session(&adapter_app, &adapter_store, "/wt/adapter-reject");
        adapter_engine
            .seed_active_execution_for_test(
                adapter_run_id.clone(),
                rejectable_adapter_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/adapter-reject".to_string(),
                adapter_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let direct_app = make_adapter_app();
        let direct_engine = make_adapter_engine();
        let direct_data_dir = configure_run_store(&direct_app, &direct_engine).await;
        let (direct_store, direct_handles) = make_adapter_deps();
        let direct_run_id = uuid::Uuid::new_v4().to_string();
        let direct_parent =
            create_adapter_parent_session(&direct_app, &direct_store, "/wt/direct-reject");
        direct_engine
            .seed_active_execution_for_test(
                direct_run_id.clone(),
                rejectable_adapter_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/direct-reject".to_string(),
                direct_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        approve_workflow_step_adapter(
            adapter_app.handle(),
            &adapter_handles,
            &adapter_store,
            &adapter_engine,
            adapter_run_id.clone(),
            ApprovalDecisionInput::Reject {
                reason: "needs changes".to_string(),
            },
            "review".to_string(),
        )
        .await
        .expect("adapter reject must succeed");
        let direct_result = direct_engine
            .dispatch(
                direct_app.handle(),
                &direct_store,
                &direct_handles,
                WorkflowCommand::RejectNode {
                    run_id: direct_run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
            )
            .await
            .expect("direct reject must succeed");
        assert_eq!(direct_result, WorkflowCommandResult::Accepted);

        let adapter_state = adapter_engine
            .get_state_by_run_id(&adapter_run_id)
            .await
            .unwrap();
        let direct_state = direct_engine
            .get_state_by_run_id(&direct_run_id)
            .await
            .unwrap();
        assert_eq!(adapter_state.state, direct_state.state);
        assert_eq!(
            adapter_run_status(&adapter_engine, &adapter_run_id).await,
            adapter_run_status(&direct_engine, &direct_run_id).await
        );
        assert_eq!(
            event_kinds(&read_adapter_events(&adapter_data_dir, &adapter_run_id)),
            event_kinds(&read_adapter_events(&direct_data_dir, &direct_run_id))
        );
    }

    /// approval UI 由来の Abort decision は `AbortRun { expected_node_name: Some(_) }`
    /// に射影され、direct dispatch と同じ terminal state / Run Store status / event sequence に到達する。
    #[tokio::test]
    async fn approve_workflow_step_adapter_matches_direct_approval_abort_dispatch() {
        let adapter_app = make_adapter_app();
        let adapter_engine = make_adapter_engine();
        let adapter_data_dir = configure_run_store(&adapter_app, &adapter_engine).await;
        let (adapter_store, adapter_handles) = make_adapter_deps();
        let adapter_run_id = uuid::Uuid::new_v4().to_string();
        let adapter_parent = create_adapter_parent_session(
            &adapter_app,
            &adapter_store,
            "/wt/adapter-approval-abort",
        );
        adapter_engine
            .seed_active_execution_for_test(
                adapter_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/adapter-approval-abort".to_string(),
                adapter_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let direct_app = make_adapter_app();
        let direct_engine = make_adapter_engine();
        let direct_data_dir = configure_run_store(&direct_app, &direct_engine).await;
        let (direct_store, direct_handles) = make_adapter_deps();
        let direct_run_id = uuid::Uuid::new_v4().to_string();
        let direct_parent =
            create_adapter_parent_session(&direct_app, &direct_store, "/wt/direct-approval-abort");
        direct_engine
            .seed_active_execution_for_test(
                direct_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/direct-approval-abort".to_string(),
                direct_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        approve_workflow_step_adapter(
            adapter_app.handle(),
            &adapter_handles,
            &adapter_store,
            &adapter_engine,
            adapter_run_id.clone(),
            ApprovalDecisionInput::Abort,
            "review".to_string(),
        )
        .await
        .expect("adapter approval abort must succeed");
        let direct_result = direct_engine
            .dispatch(
                direct_app.handle(),
                &direct_store,
                &direct_handles,
                WorkflowCommand::AbortRun {
                    run_id: direct_run_id.clone(),
                    expected_node_name: Some("review".to_string()),
                },
            )
            .await
            .expect("direct approval abort must succeed");
        assert_eq!(direct_result, WorkflowCommandResult::Accepted);

        assert_eq!(
            adapter_engine
                .get_state_by_run_id(&adapter_run_id)
                .await
                .unwrap()
                .state,
            direct_engine
                .get_state_by_run_id(&direct_run_id)
                .await
                .unwrap()
                .state
        );
        assert_eq!(
            adapter_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            direct_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            event_kinds(&read_adapter_events(&adapter_data_dir, &adapter_run_id)),
            event_kinds(&read_adapter_events(&direct_data_dir, &direct_run_id))
        );
    }

    // ---- CLI / UI 経路の engine 等価性（spec [06] L99-102 Rule, review R4-01） ----
    //
    // 同一意図の state 変化要求は呼び出し経路に依らず engine から見て等価に
    // 扱われる、という Rule を直接検証する。各テストは UI adapter
    // （`approve_workflow_step_adapter` / `abort_workflow_adapter`）と CLI
    // pending dispatcher（`dispatch_pending_command`）を同一初期 state に
    // 流し、`CliMutationRequested`（CLI 経路のみ追加される観測 event）を
    // 除いた event 列が一致することを確認する。

    /// CLI pending Approve は UI approve_workflow_step と engine 視点で等価。
    #[tokio::test]
    async fn cli_pending_approve_and_ui_approve_yield_equivalent_engine_outcome() {
        let ui_app = make_adapter_app();
        let ui_engine = make_adapter_engine();
        let ui_data_dir = configure_run_store(&ui_app, &ui_engine).await;
        let (ui_store, ui_handles) = make_adapter_deps();
        let ui_run_id = uuid::Uuid::new_v4().to_string();
        let ui_parent = create_adapter_parent_session(&ui_app, &ui_store, "/wt/ui-approve-parity");
        ui_engine
            .seed_active_execution_for_test(
                ui_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/ui-approve-parity".to_string(),
                ui_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let cli_app = make_adapter_app();
        let cli_engine = make_adapter_engine();
        let cli_data_dir = configure_run_store(&cli_app, &cli_engine).await;
        let (cli_store, cli_handles) = make_adapter_deps();
        let cli_run_id = uuid::Uuid::new_v4().to_string();
        let cli_parent =
            create_adapter_parent_session(&cli_app, &cli_store, "/wt/cli-approve-parity");
        cli_engine
            .seed_active_execution_for_test(
                cli_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/cli-approve-parity".to_string(),
                cli_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        approve_workflow_step_adapter(
            ui_app.handle(),
            &ui_handles,
            &ui_store,
            &ui_engine,
            ui_run_id.clone(),
            ApprovalDecisionInput::Approve {
                comment: Some("parity-lgtm".to_string()),
            },
            "review".to_string(),
        )
        .await
        .expect("UI approval must succeed");

        let cli_outcome = crate::workflow::pending_command_dispatcher::dispatch_pending_command(
            cli_app.handle(),
            &cli_engine,
            &cli_store,
            &cli_handles,
            crate::workflow::pending_command::PendingCommand::new(
                cli_run_id.clone(),
                crate::workflow::pending_command::PendingCommandPayload::Approve {
                    node_name: Some("review".to_string()),
                    comment: Some("parity-lgtm".to_string()),
                },
                100.0,
            ),
        )
        .await;
        assert_eq!(
            cli_outcome,
            crate::workflow::pending_command_dispatcher::PendingCommandDispatchOutcome::Accepted
        );

        let ui_state = ui_engine.get_state_by_run_id(&ui_run_id).await.unwrap();
        let cli_state = cli_engine.get_state_by_run_id(&cli_run_id).await.unwrap();
        assert_eq!(ui_state.state, cli_state.state);
        assert_eq!(
            adapter_run_status(&ui_engine, &ui_run_id).await,
            adapter_run_status(&cli_engine, &cli_run_id).await
        );
        assert_eq!(
            event_kinds_excluding_cli_mutation(&read_adapter_events(&ui_data_dir, &ui_run_id)),
            event_kinds_excluding_cli_mutation(&read_adapter_events(&cli_data_dir, &cli_run_id))
        );
    }

    /// CLI pending Reject は UI approve_workflow_step Reject と engine 視点で等価。
    #[tokio::test]
    async fn cli_pending_reject_and_ui_reject_yield_equivalent_engine_outcome() {
        let ui_app = make_adapter_app();
        let ui_engine = make_adapter_engine();
        let ui_data_dir = configure_run_store(&ui_app, &ui_engine).await;
        let (ui_store, ui_handles) = make_adapter_deps();
        let ui_run_id = uuid::Uuid::new_v4().to_string();
        let ui_parent = create_adapter_parent_session(&ui_app, &ui_store, "/wt/ui-reject-parity");
        ui_engine
            .seed_active_execution_for_test(
                ui_run_id.clone(),
                rejectable_adapter_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/ui-reject-parity".to_string(),
                ui_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let cli_app = make_adapter_app();
        let cli_engine = make_adapter_engine();
        let cli_data_dir = configure_run_store(&cli_app, &cli_engine).await;
        let (cli_store, cli_handles) = make_adapter_deps();
        let cli_run_id = uuid::Uuid::new_v4().to_string();
        let cli_parent =
            create_adapter_parent_session(&cli_app, &cli_store, "/wt/cli-reject-parity");
        cli_engine
            .seed_active_execution_for_test(
                cli_run_id.clone(),
                rejectable_adapter_workflow(),
                WorkflowExecutionState::WaitingApproval,
                "/wt/cli-reject-parity".to_string(),
                cli_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        approve_workflow_step_adapter(
            ui_app.handle(),
            &ui_handles,
            &ui_store,
            &ui_engine,
            ui_run_id.clone(),
            ApprovalDecisionInput::Reject {
                reason: "needs changes".to_string(),
            },
            "review".to_string(),
        )
        .await
        .expect("UI reject must succeed");

        let cli_outcome = crate::workflow::pending_command_dispatcher::dispatch_pending_command(
            cli_app.handle(),
            &cli_engine,
            &cli_store,
            &cli_handles,
            crate::workflow::pending_command::PendingCommand::new(
                cli_run_id.clone(),
                crate::workflow::pending_command::PendingCommandPayload::Reject {
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
                200.0,
            ),
        )
        .await;
        assert_eq!(
            cli_outcome,
            crate::workflow::pending_command_dispatcher::PendingCommandDispatchOutcome::Accepted
        );

        let ui_state = ui_engine.get_state_by_run_id(&ui_run_id).await.unwrap();
        let cli_state = cli_engine.get_state_by_run_id(&cli_run_id).await.unwrap();
        assert_eq!(ui_state.state, cli_state.state);
        assert_eq!(
            adapter_run_status(&ui_engine, &ui_run_id).await,
            adapter_run_status(&cli_engine, &cli_run_id).await
        );
        assert_eq!(
            event_kinds_excluding_cli_mutation(&read_adapter_events(&ui_data_dir, &ui_run_id)),
            event_kinds_excluding_cli_mutation(&read_adapter_events(&cli_data_dir, &cli_run_id))
        );
    }

    /// CLI pending Abort（run 全体）は UI abort_workflow と engine 視点で等価。
    #[tokio::test]
    async fn cli_pending_abort_and_ui_abort_yield_equivalent_engine_outcome() {
        let ui_app = make_adapter_app();
        let ui_engine = make_adapter_engine();
        let ui_data_dir = configure_run_store(&ui_app, &ui_engine).await;
        let (ui_store, ui_handles) = make_adapter_deps();
        let ui_run_id = uuid::Uuid::new_v4().to_string();
        let ui_parent = create_adapter_parent_session(&ui_app, &ui_store, "/wt/ui-abort-parity");
        ui_engine
            .seed_active_execution_for_test(
                ui_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::Running,
                "/wt/ui-abort-parity".to_string(),
                ui_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        let cli_app = make_adapter_app();
        let cli_engine = make_adapter_engine();
        let cli_data_dir = configure_run_store(&cli_app, &cli_engine).await;
        let (cli_store, cli_handles) = make_adapter_deps();
        let cli_run_id = uuid::Uuid::new_v4().to_string();
        let cli_parent =
            create_adapter_parent_session(&cli_app, &cli_store, "/wt/cli-abort-parity");
        cli_engine
            .seed_active_execution_for_test(
                cli_run_id.clone(),
                approval_only_workflow(),
                WorkflowExecutionState::Running,
                "/wt/cli-abort-parity".to_string(),
                cli_parent.id,
                TriggerSource::DesktopUi,
            )
            .await;

        abort_workflow_adapter(
            ui_app.handle(),
            &ui_handles,
            &ui_store,
            &ui_engine,
            ui_run_id.clone(),
        )
        .await
        .expect("UI abort must succeed");

        let cli_outcome = crate::workflow::pending_command_dispatcher::dispatch_pending_command(
            cli_app.handle(),
            &cli_engine,
            &cli_store,
            &cli_handles,
            crate::workflow::pending_command::PendingCommand::new(
                cli_run_id.clone(),
                crate::workflow::pending_command::PendingCommandPayload::Abort { node_name: None },
                300.0,
            ),
        )
        .await;
        assert_eq!(
            cli_outcome,
            crate::workflow::pending_command_dispatcher::PendingCommandDispatchOutcome::Accepted
        );

        let ui_state = ui_engine.get_state_by_run_id(&ui_run_id).await.unwrap();
        let cli_state = cli_engine.get_state_by_run_id(&cli_run_id).await.unwrap();
        assert_eq!(ui_state.state, WorkflowExecutionState::Aborted);
        assert_eq!(cli_state.state, WorkflowExecutionState::Aborted);
        assert_eq!(
            ui_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            cli_engine.list_completed_runs().await[0].status,
            RunStatus::Aborted
        );
        assert_eq!(
            event_kinds_excluding_cli_mutation(&read_adapter_events(&ui_data_dir, &ui_run_id)),
            event_kinds_excluding_cli_mutation(&read_adapter_events(&cli_data_dir, &cli_run_id))
        );
    }

    // ---- ApprovalDecisionInput DTO（[04] command 境界の wire 形式 + WorkflowCommand 変換） ----

    /// Spec [04] / issues-1013: `ApprovalDecisionInput::Approve` は任意 comment を内包し、
    /// `{"approve":{}}` / `{"approve":{"comment":"..."}}` の双方を受理する。
    #[test]
    fn approval_decision_input_deserialize_approve_optional_comment() {
        let no_comment: ApprovalDecisionInput = serde_json::from_str(r#"{"approve":{}}"#).unwrap();
        assert_eq!(no_comment, ApprovalDecisionInput::Approve { comment: None });
        let with_comment: ApprovalDecisionInput =
            serde_json::from_str(r#"{"approve":{"comment":"lgtm"}}"#).unwrap();
        assert_eq!(
            with_comment,
            ApprovalDecisionInput::Approve {
                comment: Some("lgtm".to_string())
            }
        );
    }

    /// Spec [04] / issues-1013: `ApprovalDecisionInput::Reject` は `reason` 必須。
    #[test]
    fn approval_decision_input_deserialize_reject_with_reason() {
        let decision: ApprovalDecisionInput =
            serde_json::from_str(r#"{"reject":{"reason":"Please fix"}}"#).unwrap();
        assert_eq!(
            decision,
            ApprovalDecisionInput::Reject {
                reason: "Please fix".to_string()
            }
        );
    }

    /// Spec [04] / issues-1013: `ApprovalDecisionInput::Abort` は unit variant の wire 形式。
    #[test]
    fn approval_decision_input_deserialize_abort_unit_variant() {
        let decision: ApprovalDecisionInput = serde_json::from_str(r#""abort""#).unwrap();
        assert_eq!(decision, ApprovalDecisionInput::Abort);
    }

    /// Spec [04] / issues-1013: 旧 unit variant `"approve"` は受理しない
    /// （後方互換 wrapper を持たない command 境界の不変条件）。
    #[test]
    fn approval_decision_input_rejects_legacy_unit_approve() {
        assert!(serde_json::from_str::<ApprovalDecisionInput>(r#""approve""#).is_err());
    }

    /// Spec [04] / issues-1013: `into_command` は approval 入力を typed `WorkflowCommand`
    /// にマップする。Approve → ApproveNode、Reject → RejectNode（reason が伝播）、
    /// Abort → AbortRun（current node が `expected_node_name` に固定される）。
    #[test]
    fn approval_decision_input_into_command_routes_each_variant() {
        let run_id = "00000000-0000-0000-0000-000000000099".to_string();
        let step = "review".to_string();

        let approve_input = ApprovalDecisionInput::Approve {
            comment: Some("lgtm".to_string()),
        };
        let approve_cmd = approve_input.into_command(run_id.clone(), step.clone());
        match approve_cmd {
            WorkflowCommand::ApproveNode {
                run_id: rid,
                node_name,
                comment,
            } => {
                assert_eq!(rid, run_id);
                assert_eq!(node_name, Some(step.clone()));
                assert_eq!(comment.as_deref(), Some("lgtm"));
            }
            other => panic!("Approve must map to ApproveNode, got: {other:?}"),
        }

        let reject_input = ApprovalDecisionInput::Reject {
            reason: "needs fix".to_string(),
        };
        let reject_cmd = reject_input.into_command(run_id.clone(), step.clone());
        match reject_cmd {
            WorkflowCommand::RejectNode {
                run_id: rid,
                node_name,
                reason,
            } => {
                assert_eq!(rid, run_id);
                assert_eq!(node_name, Some(step.clone()));
                assert_eq!(reason, "needs fix");
            }
            other => panic!("Reject must map to RejectNode, got: {other:?}"),
        }

        let abort_cmd = ApprovalDecisionInput::Abort.into_command(run_id.clone(), step.clone());
        match abort_cmd {
            WorkflowCommand::AbortRun {
                run_id: rid,
                expected_node_name,
            } => {
                assert_eq!(rid, run_id);
                assert_eq!(expected_node_name.as_deref(), Some("review"));
            }
            other => panic!("Abort must map to AbortRun, got: {other:?}"),
        }
    }

    #[test]
    fn workflow_tab_error_is_redacted() {
        let err = redacted_workflow_tab_error("workflow_step_session_rejected");
        assert_eq!(
            err,
            "workflow_step_session_rejected: workflow step tab operation failed"
        );
        assert!(!err.contains("/repo"));
        assert!(!err.contains("agent-session"));
        assert!(!err.contains("message body"));
    }

    /// Spec issues-1011 finding 12: command 入口の `validate_run_id` は path traversal や
    /// 形式不正な run_id を拒否し、後段の Run Store / engine に到達させない。
    /// abort_workflow / get_workflow_state / approve_workflow_step /
    /// get_workflow_run / get_workflow_run_log / get_workflow_run_state /
    /// resolve_worktree_by_run の全 command で共通に使われるため、入力種別ごとに
    /// 受理/拒否を一括で担保する。
    #[test]
    fn validate_run_id_table_accepts_uuid_and_rejects_invalid_inputs() {
        // 受理: 正規 UUID（生成値）と既知サンプル
        let generated = uuid::Uuid::new_v4().to_string();
        let accepted = [
            generated.as_str(),
            "550e8400-e29b-41d4-a716-446655440000",
            "00000000-0000-0000-0000-000000000000",
        ];
        for input in accepted {
            assert!(
                validate_run_id(input).is_ok(),
                "valid UUID must be accepted: {input}"
            );
        }

        // 拒否: 空文字 / 非 UUID / path traversal / 不正文字 / 余分なスペース / 長さ違い
        let rejected = [
            "",
            "not-a-uuid",
            "../etc/passwd",
            "../../workflow_runs/secret",
            "run-1",
            "550e8400-e29b-41d4-a716-44665544000", // 1 文字不足
            "550e8400-e29b-41d4-a716-4466554400000", // 1 文字過剰
            "550e8400-e29b-41d4-a716-44665544000g", // 非 hex
            "550e8400-e29b-41d4-a716-446655440000\n",
            " 550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440000 ",
        ];
        for input in rejected {
            assert!(
                validate_run_id(input).is_err(),
                "invalid run_id must be rejected: {input:?}"
            );
        }
    }

    #[test]
    fn workflow_approval_chat_permission_mode_rejects_invalid_values_before_dispatch() {
        for invalid in [
            None,
            Some(""),
            Some("acceptEdits"),
            Some("default"),
            Some("unknown"),
        ] {
            let err = parse_workflow_approval_permission_mode(invalid.map(str::to_string))
                .expect_err("invalid permission_mode must be rejected");
            assert!(
                err.contains("readonly, edit, full"),
                "error must include allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn workflow_approval_chat_permission_mode_accepts_abstract_values() {
        for (value, expected) in [
            ("readonly", PermissionMode::Readonly),
            ("edit", PermissionMode::Edit),
            ("full", PermissionMode::Full),
        ] {
            let parsed = parse_workflow_approval_permission_mode(Some(value.to_string())).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn parse_trigger_source_rejects_unknown_values() {
        assert!(matches!(
            parse_trigger_source(None).unwrap(),
            crate::workflow::run::TriggerSource::DesktopUi
        ));
        assert!(matches!(
            parse_trigger_source(Some("remote".to_string())).unwrap(),
            crate::workflow::run::TriggerSource::Remote
        ));
        let err = parse_trigger_source(Some("unknown".to_string())).unwrap_err();
        assert!(err.contains("unknown trigger_source"));
    }

    #[test]
    fn workflow_start_permission_mode_defaults_readonly_and_rejects_invalid_values() {
        assert_eq!(
            parse_workflow_start_permission_mode(None).unwrap(),
            PermissionMode::Readonly
        );
        assert_eq!(
            parse_workflow_start_permission_mode(Some("edit".to_string())).unwrap(),
            PermissionMode::Edit
        );
        let err = parse_workflow_start_permission_mode(Some("acceptEdits".to_string()))
            .expect_err("provider-specific permission flags must not be accepted");
        assert!(err.contains("readonly, edit, full"));
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

    /// Gherkin: parse_facet_kind を経由する Tauri コマンドは 4種それぞれの種別指定で
    /// 正常経路に到達する（種別解決層）
    #[test]
    fn parse_facet_kind_resolves_all_four_kinds_for_command_routing() {
        for kind in ["policy", "knowledge", "instruction", "contract"] {
            assert!(
                parse_facet_kind(kind).is_ok(),
                "kind '{kind}' should be accepted"
            );
        }
    }

    // ---- ファセットコマンド × 4 種カバレッジ + persona / 未知種別拒否 ----
    //
    // Spec L107-127 の「列挙した各 Tauri コマンドは 4種それぞれの種別指定で正常経路に到達する」
    // および「persona または未知種別を指定した Tauri コマンドは拒否される」を、
    // テンポラリディレクトリ上で各コマンド × 4種の組合せで実行することで検証する。

    const FOUR_KINDS: [(&str, &str); 4] = [
        ("policy", "policies"),
        ("knowledge", "knowledge"),
        ("instruction", "instructions"),
        ("contract", "contracts"),
    ];

    /// 4 種それぞれのディレクトリを作成し、各種に既存の非ビルトインキー（"sample-{kind}"）を配置する。
    fn setup_tmp_facets_base() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        for (_kind, dir_name) in FOUR_KINDS {
            let dir = tmp.path().join(dir_name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("sample-{dir_name}.md")), "SAMPLE_BODY").unwrap();
        }
        tmp
    }

    fn key_for(kind: &str) -> String {
        let (_, dir) = FOUR_KINDS.iter().find(|(k, _)| *k == kind).unwrap();
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
        for (kind, _) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
            let result = list_facets_inner(bad, tmp.path());
            assert!(result.is_err(), "list_facets({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn get_facet_inner_reaches_load_path_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in FOUR_KINDS {
            let body = get_facet_inner(kind, &key_for(kind), tmp.path()).unwrap();
            assert_eq!(body, "SAMPLE_BODY", "get_facet({kind}) body mismatch");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn get_facet_inner_rejects_persona_and_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["persona", "unknown"] {
            let result = get_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "get_facet({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn save_facet_inner_writes_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
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
        for (kind, dir_name) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
            let result = delete_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "delete_facet({bad}) must be rejected");
        }
        // 4種のサンプルは温存されている
        for (_, dir_name) in FOUR_KINDS {
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
        for (kind, _) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
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
        for (kind, dir_name) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
            let result = duplicate_facet_inner(bad, "src", "dst", tmp.path());
            assert!(result.is_err(), "duplicate_facet({bad}) must be rejected");
        }
        assert_no_persona_files(tmp.path());
    }

    #[test]
    fn open_facet_in_editor_inner_invokes_opener_for_each_kind() {
        // open_facet_in_editor のエディタ呼び出し点はテストダブル（クロージャ）で差し替えて、
        // 実プロセスを起動せずに 4 種すべての正常経路到達と引数（対象パス）を検証する。
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in FOUR_KINDS {
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
        for bad in ["persona", "unknown"] {
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
    /// ファイルが残り、4種の一覧結果に legacy が含まれないことを直接 assert する。
    #[test]
    fn legacy_persona_file_remains_on_disk_and_is_not_listed_for_any_kind() {
        let tmp = setup_tmp_facets_base();
        let base = tmp.path();

        // 既存ユーザーが残した persona ファイル相当を事前配置
        let personas_dir = base.join("personas");
        std::fs::create_dir_all(&personas_dir).unwrap();
        let legacy_path = personas_dir.join("legacy.md");
        std::fs::write(&legacy_path, "LEGACY_PERSONA_BODY").unwrap();

        // ファセット一覧系経路を 4 種それぞれで実行
        for (kind, _dir_name) in FOUR_KINDS {
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
    fn validate_template_variables_system_vars_ok() {
        assert!(validate_template_variables("Use {{project_name}} and {{task}}").is_ok());
    }

    #[test]
    fn validate_template_variables_unknown_var_fails() {
        let result = validate_template_variables("Use {{unknown}}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown"));
    }

    #[test]
    fn validate_template_variables_no_vars_ok() {
        assert!(validate_template_variables("No variables here").is_ok());
    }

    #[test]
    fn validate_template_variables_mixed() {
        let result = validate_template_variables("{{task}} and {{bad_var}}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad_var"));
    }

    // ---- duplicate logic tests ----
    // These test the core duplicate logic using storage/facet/builtin functions directly,
    // mirroring what the Tauri commands do inside spawn_blocking.

    fn make_test_workflow(name: &str) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "step1".to_string(),
                node_type: NodeType::Agent,
                inline_prompt: Some("Do something".to_string()),
                permission: Some("edit".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    #[test]
    fn duplicate_workflow_normal_case() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let wf = make_test_workflow("source-wf");
        storage::save_workflow(dir, dir, &wf).unwrap();

        // Simulate duplicate logic
        let new_name = "copied-wf";
        super::super::validation::validate_name(new_name).unwrap();
        assert!(!dir.join(format!("{new_name}.yml")).exists());
        assert!(!builtin::is_builtin_workflow(new_name));

        let mut copied = storage::load_workflow(&dir.join("source-wf.yml"), dir).unwrap();
        copied.name = new_name.to_string();
        copied.builtin = false;
        storage::save_workflow(dir, dir, &copied).unwrap();

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
        storage::save_workflow(dir, dir, &wf).unwrap();

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
        let result = super::super::validation::validate_name("bad name!");
        assert!(result.is_err());
    }

    #[test]
    fn validation_errors_return_stable_kind_prefix_for_commands() {
        let err = super::super::validation::validate_name("bad name!")
            .map_err(super::validation_error_string)
            .unwrap_err();
        assert!(err.starts_with("validation_error:"));
    }

    #[test]
    fn duplicate_workflow_source_from_builtin() {
        let builtin_names: Vec<String> = builtin::list_builtin_workflows()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        if let Some(name) = builtin_names.first() {
            let wf = builtin::load_builtin_workflow_resolved(name)
                .expect("builtin load must succeed")
                .expect("builtin must exist for known name");
            assert!(wf.builtin);
        }
    }

    #[test]
    fn duplicate_facet_normal_case() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        super::super::facet::save_facet(
            kind,
            "source-facet",
            "# Source Policy\nContent here",
            base_dir,
        )
        .unwrap();

        let new_key = "copied-facet";
        super::super::facet::validate_facet_key(new_key).unwrap();

        let existing = super::super::facet::list_facets(kind, base_dir).unwrap();
        assert!(!existing.contains(&new_key.to_string()));

        let content = super::super::facet::load_facet(kind, "source-facet", base_dir).unwrap();
        super::super::facet::save_facet(kind, new_key, &content, base_dir).unwrap();

        let loaded = super::super::facet::load_facet(kind, new_key, base_dir).unwrap();
        assert_eq!(loaded, "# Source Policy\nContent here");
    }

    #[test]
    fn duplicate_facet_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        super::super::facet::save_facet(kind, "my-facet", "content", base_dir).unwrap();

        let existing = super::super::facet::list_facets(kind, base_dir).unwrap();

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
        let result = super::super::facet::validate_facet_key("../evil");
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
        storage::save_workflow(dir, dir, &wf_a).unwrap();
        storage::save_workflow(dir, dir, &wf_b).unwrap();

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
        super::super::facet::save_facet(kind, "existing-facet", "content", base_dir).unwrap();

        // Simulate is_new=true with duplicate key
        let existing = super::super::facet::list_facets(kind, base_dir).unwrap();

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
            let existing = super::super::facet::list_facets(FacetKind::Policy, base_dir).unwrap();
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
        workflow: &Workflow,
        original_name: Option<&str>,
    ) -> Result<(), String> {
        let is_new = original_name.is_none();
        let is_rename = original_name.is_some_and(|o| o != workflow.name);
        if (is_new || is_rename) && dir.join(format!("{}.yml", workflow.name)).exists() {
            return Err(format!("ワークフロー '{}' は既に存在します", workflow.name));
        }
        storage::save_workflow(dir, dir, workflow).map_err(|e| e.to_string())?;
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

        let wf = make_test_workflow("my-wf");
        storage::save_workflow(dir, dir, &wf).unwrap();

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
        storage::save_workflow(dir, dir, &wf).unwrap();

        let result = simulate_save_workflow(dir, &wf, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    fn save_workflow_rename_succeeds_and_removes_old_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("old-name");
        storage::save_workflow(dir, dir, &wf).unwrap();

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

        storage::save_workflow(dir, dir, &make_test_workflow("wf-a")).unwrap();
        storage::save_workflow(dir, dir, &make_test_workflow("wf-b")).unwrap();

        let renamed = make_test_workflow("wf-b");
        let result = simulate_save_workflow(dir, &renamed, Some("wf-a"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- [05] read-only Run 観測 API: Tauri command 境界の直接テスト ----

    fn read_only_test_uuid(seed: u8) -> String {
        uuid::Uuid::from_bytes([seed; 16]).to_string()
    }

    fn make_read_only_run(
        run_id: &str,
        workflow_name: &str,
        worktree: &str,
        status: RunStatus,
        started_at: f64,
    ) -> crate::workflow::run::WorkflowRun {
        crate::workflow::run::WorkflowRun {
            run_id: run_id.to_string(),
            workflow_name: workflow_name.to_string(),
            task: None,
            status,
            worktree_path: worktree.to_string(),
            chat_session_id: None,
            current_node_name: None,
            trigger_source: TriggerSource::DesktopUi,
            started_at,
            updated_at: started_at,
            completed_at: if status.is_terminal() {
                Some(started_at + 1.0)
            } else {
                None
            },
            error_reason: None,
        }
    }

    fn write_read_only_run(data_dir: &Path, run: &crate::workflow::run::WorkflowRun) {
        let runs_dir = data_dir.join("workflow_runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        let path = runs_dir.join(format!("{}.json", run.run_id));
        let json = serde_json::to_string_pretty(run).unwrap();
        std::fs::write(path, json).unwrap();
    }

    fn make_read_only_app() -> (AdapterTestApp, Arc<WorkflowEngine>, std::path::PathBuf) {
        let app = make_adapter_app();
        let engine = make_adapter_engine();
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        app.manage(engine.clone());
        (app, engine, data_dir)
    }

    /// [05] worktree-scoped 認可境界のテスト用 fixture: 実 git repo + worktree を作り、
    /// `AppConfig.last_repo_paths` に親 repo を登録する。戻り値は test app / engine /
    /// data_dir / canonical worktree path / TempDir guards（lifetime 保持用）。
    fn make_read_only_app_with_managed_worktree() -> (
        AdapterTestApp,
        Arc<WorkflowEngine>,
        std::path::PathBuf,
        String,
        TempDir,
        TempDir,
    ) {
        let (app, engine, data_dir) = make_read_only_app();
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
        app.state::<Arc<AppConfig>>()
            .with_config_mut(|config| {
                config.app.last_repo_paths = vec![repo_path.to_string_lossy().to_string()];
                Ok(())
            })
            .unwrap();
        (
            app,
            engine,
            data_dir,
            canonical_str,
            repo_parent,
            worktree_parent,
        )
    }

    /// Spec [05] Rule: 外部 caller は run_id を主語として workflow run 一覧を観測できる。
    /// `list_workflow_runs` Tauri command 経路が active を先頭・terminal を後続として
    /// 並び替え、status filter が機能することを直接検証する。
    ///
    /// 観測経路の認可境界（spec [05] L104-108 / L182）として `worktree_path` は必須で、
    /// caller の認可済み managed worktree のみを対象にする。
    ///
    /// active run は in-memory map + metadata file の両方に存在する境界状態であるため、
    /// `RunStore::register_active` 経由で投入する。terminal run はメタデータファイル
    /// のみに投入し、`list_completed` が拾い上げることを検証する。
    #[tokio::test]
    async fn list_workflow_runs_command_returns_active_first_filtered_by_status() {
        let (app, engine, data_dir, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let active_id = read_only_test_uuid(1);
        let done_id = read_only_test_uuid(2);
        engine
            .run_store()
            .register_active(make_read_only_run(
                &active_id,
                "wf-active",
                &worktree_path,
                RunStatus::Running,
                200.0,
            ))
            .await
            .expect("register_active must succeed");
        write_read_only_run(
            &data_dir,
            &make_read_only_run(
                &done_id,
                "wf-done",
                &worktree_path,
                RunStatus::Completed,
                100.0,
            ),
        );

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let all = list_workflow_runs(engine_state, config_state, None, worktree_path.clone())
            .await
            .expect("list_workflow_runs must succeed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, RunStatus::Running);
        assert_eq!(all[1].status, RunStatus::Completed);

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let terminal = list_workflow_runs(
            engine_state,
            config_state,
            Some("terminal".to_string()),
            worktree_path.clone(),
        )
        .await
        .expect("terminal filter must succeed");
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].run_id, done_id);

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let bad = list_workflow_runs(
            engine_state,
            config_state,
            Some("garbage".to_string()),
            worktree_path.clone(),
        )
        .await;
        assert!(bad.is_err(), "invalid status filter must be rejected");
    }

    /// Spec [05] Rule: 観測経路は API と CLI で等価な手段を提供する。
    /// `list_workflow_runs` の `worktree_path` 入力は `canonicalize_managed_worktree_path`
    /// で正規化されたうえで filter に渡されるため、末尾 `/` 付きや `.` を含む形で
    /// 同一 managed worktree を指定しても、canonical 表現で永続化された run と一致する。
    /// CLI 経路と同じ `normalize_worktree_filter_path` を経由する境界を直接検証する。
    #[tokio::test]
    async fn list_workflow_runs_canonicalizes_worktree_path_filter() {
        let (app, engine, data_dir, canonical_str, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;

        // canonical な worktree_path で run を 1 件、別 worktree で run を 1 件登録する。
        let target_id = read_only_test_uuid(40);
        let other_id = read_only_test_uuid(41);
        engine
            .run_store()
            .register_active(make_read_only_run(
                &target_id,
                "wf-target",
                &canonical_str,
                RunStatus::Running,
                200.0,
            ))
            .await
            .expect("register_active for target must succeed");
        write_read_only_run(
            &data_dir,
            &make_read_only_run(
                &other_id,
                "wf-other",
                "/wt/other",
                RunStatus::Completed,
                100.0,
            ),
        );

        // 末尾 `/` を付けて非 canonical な表現で filter を渡す。
        let trailing = format!("{canonical_str}/");
        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let runs = list_workflow_runs(engine_state, config_state, None, trailing)
            .await
            .expect("list_workflow_runs with trailing slash must canonicalize and match");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, target_id);
        assert_eq!(runs[0].worktree_path, canonical_str);

        // managed worktree でない path は Err。
        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let outside = TempDir::new().unwrap();
        let bad = list_workflow_runs(
            engine_state,
            config_state,
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
    /// 既存 `list_workflow_runs_command_canonicalizes_managed_worktree_filter` と相補的に
    /// 観測経路ごとの unauthorized 経路を明示する境界テスト。
    #[tokio::test]
    async fn list_workflow_runs_rejects_unauthorized_worktree_observation() {
        let (app, engine, data_dir) = make_read_only_app();
        engine.set_run_store_data_dir(data_dir.clone()).await;

        // run は存在する。ただし caller が指定する worktree_path は
        // configured repo に紐づかない（= 観測権限を持たない）ので Err として弾かれる。
        let run_id = read_only_test_uuid(60);
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", "/wt/inside", RunStatus::Running, 100.0),
        );

        let outside = TempDir::new().unwrap();
        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let result = list_workflow_runs(
            engine_state,
            config_state,
            None,
            outside.path().to_string_lossy().to_string(),
        )
        .await;
        assert!(
            result.is_err(),
            "unauthorized worktree_path must be rejected as Err (no observation leak)"
        );
    }

    /// Spec [05] Rule: 指定 run の summary metadata を観測する。
    /// Spec [05] Rule: 存在しない run_id は明示的に「該当 run なし」として扱われる。
    /// Spec [05] L104-108 / L182: caller の認可済み worktree に紐づかない run は Ok(None)。
    #[tokio::test]
    async fn get_workflow_run_command_returns_summary_or_none() {
        let (app, engine, data_dir, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = read_only_test_uuid(3);
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", &worktree_path, RunStatus::Running, 300.0),
        );

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let found = get_workflow_run(engine_state, config_state, run_id.clone())
            .await
            .expect("get_workflow_run must succeed");
        let found = found.expect("run must be found");
        assert_eq!(found.run_id, run_id);
        assert_eq!(found.worktree_path, worktree_path);
        assert_eq!(found.status, RunStatus::Running);

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let missing = get_workflow_run(engine_state, config_state, read_only_test_uuid(99))
            .await
            .expect("get_workflow_run for unknown run_id must Ok(None)");
        assert!(missing.is_none());

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let invalid = get_workflow_run(engine_state, config_state, "not-a-uuid".to_string()).await;
        assert!(invalid.is_err(), "non-UUID run_id must be rejected");
    }

    /// Spec [05] L104-108 / L182 negative test: 認可境界として、run metadata の
    /// worktree_path が caller の認可済み managed worktree に合致しない場合、
    /// `get_workflow_run` / `get_workflow_run_log` / `get_workflow_run_state` は
    /// いずれも `Ok(None)` を返し、存在する run の summary / log / state を
    /// 未認可 caller に伝えない。
    #[tokio::test]
    async fn single_run_read_only_apis_reject_unauthorized_worktree_observation() {
        let (app, engine, data_dir, _managed, _r, _w) = make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;

        // run は別 worktree に紐づく metadata で書かれている（未認可）。
        let run_id = read_only_test_uuid(70);
        let unauthorized_wt = "/wt/unauthorized";
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", unauthorized_wt, RunStatus::Running, 100.0),
        );
        let event_log = WorkflowEventLog::new(&data_dir);
        event_log
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: unauthorized_wt.to_string(),
                workflow_definition: Workflow {
                    name: "wf".to_string(),
                    description: "test".to_string(),
                    builtin: false,
                    nodes: vec![],
                },
                timestamp: 100.0,
            })
            .unwrap();

        let engine_state = app.state::<Arc<WorkflowEngine>>();
        let config_state = app.state::<Arc<AppConfig>>();
        let summary = get_workflow_run(engine_state, config_state, run_id.clone())
            .await
            .expect("get_workflow_run must succeed");
        assert!(
            summary.is_none(),
            "unauthorized run summary must not be observable"
        );

        // unauthorized worktree_path は canonicalize 段階で Err として弾かれる。
        let config_state = app.state::<Arc<AppConfig>>();
        let log = get_workflow_run_log_inner(
            app.handle(),
            &engine,
            &config_state,
            unauthorized_wt.to_string(),
            run_id.clone(),
        )
        .await;
        assert!(
            log.is_err(),
            "unauthorized worktree_path must be rejected by event log invoke"
        );

        let config_state = app.state::<Arc<AppConfig>>();
        let state = get_workflow_run_state_inner(
            app.handle(),
            &engine,
            &config_state,
            unauthorized_wt.to_string(),
            run_id.clone(),
        )
        .await;
        assert!(
            state.is_err(),
            "unauthorized worktree_path must be rejected by state invoke"
        );
    }

    /// Spec [05] Rule: 指定 run の event log を観測する。
    /// `get_workflow_run_log` Tauri command が NDJSON から読み込んだ event 列を返す。
    #[tokio::test]
    async fn get_workflow_run_log_command_reads_persisted_ndjson() {
        let (app, engine, data_dir, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = read_only_test_uuid(4);
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", &worktree_path, RunStatus::Completed, 400.0),
        );
        let event_log = WorkflowEventLog::new(&data_dir);
        event_log
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: worktree_path.clone(),
                workflow_definition: Workflow {
                    name: "wf".to_string(),
                    description: "test".to_string(),
                    builtin: false,
                    nodes: vec![],
                },
                timestamp: 400.0,
            })
            .unwrap();

        let config_state = app.state::<Arc<AppConfig>>();
        let events = get_workflow_run_log_inner(
            app.handle(),
            &engine,
            &config_state,
            worktree_path.clone(),
            run_id.clone(),
        )
        .await
        .expect("get_workflow_run_log must succeed")
        .expect("run must be found");
        assert_eq!(events.len(), 1);
        match &events[0] {
            super::WorkflowEventView::RunStarted { timestamp_ms, .. } => {
                // spec issues-1023: 永続化された秒単位 timestamp は API 境界で ms 単位の
                // view 型へ変換されて返る（`timestamp_ms` フィールドで単位を明示）。
                assert_eq!(*timestamp_ms, 400_000.0);
            }
            _ => panic!("first event must be RunStarted"),
        }

        let config_state = app.state::<Arc<AppConfig>>();
        let missing = get_workflow_run_log_inner(
            app.handle(),
            &engine,
            &config_state,
            worktree_path.clone(),
            read_only_test_uuid(98),
        )
        .await
        .expect("get_workflow_run_log for unknown run_id must Ok(None)");
        assert!(missing.is_none());
    }

    /// Spec [05] Rule: 指定 run の現在 state を観測する（event log からの純粋投影）。
    /// 観測結果の露出範囲境界: AgentProcessMap / OpenTabRegistry 由来の runtime_active /
    /// tab_open enrichment は含めない（戻り値の runtime_states は空）。
    #[tokio::test]
    async fn get_workflow_run_state_command_projects_state_without_runtime_enrichment() {
        let (app, engine, data_dir, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = read_only_test_uuid(5);
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", &worktree_path, RunStatus::Running, 500.0),
        );
        let event_log = WorkflowEventLog::new(&data_dir);
        event_log
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: "adapter-boundary".to_string(),
                workflow_file_stem: "adapter-boundary".to_string(),
                worktree_path: worktree_path.clone(),
                workflow_definition: approval_only_workflow(),
                timestamp: 500.0,
            })
            .unwrap();

        let config_state = app.state::<Arc<AppConfig>>();
        let view = get_workflow_run_state_inner(
            app.handle(),
            &engine,
            &config_state,
            worktree_path.clone(),
            run_id.clone(),
        )
        .await
        .expect("get_workflow_run_state must succeed")
        .expect("state must be available");
        assert_eq!(view.state.execution_id, run_id);
        assert!(
            view.runtime_states.is_empty(),
            "read-only state view must not include runtime enrichment"
        );

        // 存在しない run_id は Ok(None)。
        let config_state = app.state::<Arc<AppConfig>>();
        let missing = get_workflow_run_state_inner(
            app.handle(),
            &engine,
            &config_state,
            worktree_path.clone(),
            read_only_test_uuid(97),
        )
        .await
        .expect("unknown run must Ok(None)");
        assert!(missing.is_none());
    }

    /// spec issues-1023 L132/L150: `get_workflow_step_detail_inner` の worktree
    /// 認可境界。現 worktree と一致する run の detail は Some、別 worktree（managed
    /// 集合外）からの invoke は canonicalize 段階で Err として弾かれる。
    #[tokio::test]
    async fn get_workflow_step_detail_enforces_current_worktree_authorization() {
        let (app, engine, data_dir, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        engine.set_run_store_data_dir(data_dir.clone()).await;

        let run_id = read_only_test_uuid(82);
        write_read_only_run(
            &data_dir,
            &make_read_only_run(&run_id, "wf", &worktree_path, RunStatus::Completed, 100.0),
        );
        let event_log = WorkflowEventLog::new(&data_dir);
        let snapshot = Workflow {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![crate::workflow::schema::NodeDefinition {
                name: "plan".to_string(),
                node_type: crate::workflow::schema::NodeType::Agent,
                instruction: Some("plan it".to_string()),
                ..crate::workflow::schema::NodeDefinition::default()
            }],
        };
        event_log
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: worktree_path.clone(),
                workflow_definition: snapshot,
                timestamp: 100.0,
            })
            .unwrap();
        event_log
            .append(&WorkflowEvent::NodeStarted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 101.0,
            })
            .unwrap();
        event_log
            .append(&WorkflowEvent::NodeCompleted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                node_name: "plan".to_string(),
                result: Some("done".to_string()),
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: Some(1),
                timestamp: 102.0,
            })
            .unwrap();

        let config_state = app.state::<Arc<AppConfig>>().inner().clone();

        // 現 worktree からの invoke は detail を返す
        let ok = get_workflow_step_detail_inner(
            app.handle(),
            &engine,
            &config_state,
            worktree_path.clone(),
            run_id.clone(),
            "plan".to_string(),
            Some(1),
        )
        .await
        .expect("detail invoke must succeed");
        let detail = ok.expect("detail must be observable for matching worktree");
        assert_eq!(detail.step_name, "plan");
        assert_eq!(detail.run_index, 1);

        // 別 worktree（managed 集合外）からの invoke は canonicalize 段階で Err
        let outside = tempfile::TempDir::new().unwrap();
        let result = get_workflow_step_detail_inner(
            app.handle(),
            &engine,
            &config_state,
            outside.path().to_string_lossy().to_string(),
            run_id.clone(),
            "plan".to_string(),
            Some(1),
        )
        .await;
        assert!(
            result.is_err(),
            "detail invoke from unauthorized worktree must be rejected"
        );
    }
}
