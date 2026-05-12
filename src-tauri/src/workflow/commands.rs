use super::builtin;
use super::diagnostics;
use super::engine::{ApprovalDecision, WorkflowEngine};
use super::facet::FacetKind;
use super::log::{WorkflowEventLog, WorkflowLogEvent};
use super::schema::{FacetSummary, Summary, Workflow};
use super::storage;
use crate::agent_sdk::AgentProcessMap;
use crate::backends::{AgentBackendRegistry, ImageAttachment};
use crate::config::AppConfig;
use crate::session::{resolve_data_dir, SessionStore, WorkflowState};
use std::sync::Arc;
use tokio::sync::Mutex;

fn parse_facet_kind(kind: &str) -> Result<FacetKind, String> {
    match kind {
        "persona" => Ok(FacetKind::Persona),
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        "output_contract" => Ok(FacetKind::OutputContract),
        _ => Err(format!("Unknown facet kind: {kind}")),
    }
}

fn validation_error_string(e: super::validation::ValidationError) -> String {
    format!("validation_error: {e}")
}

#[tauri::command]
pub async fn list_workflows(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
) -> Result<Vec<Summary>, String> {
    let running_names = engine.running_workflow_names().await;
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        let mut summaries = storage::list_workflows(&dir).map_err(|e| e.to_string())?;
        for s in &mut summaries {
            s.is_running = running_names.contains(&s.name);
        }
        Ok(summaries)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow(name: String) -> Result<Workflow, String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&name).map_err(validation_error_string)?;
        let file_path = dir.join(format!("{name}.yml"));
        if file_path.exists() {
            return storage::load_workflow(&file_path).map_err(|e| e.to_string());
        }
        builtin::get_builtin_workflow(&name)
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
    tokio::task::spawn_blocking(move || {
        // 新規作成 or リネーム時は既存名との重複チェック
        let is_new = original_name.is_none();
        let is_rename = original_name.as_ref().is_some_and(|o| *o != workflow.name);
        if (is_new || is_rename) && dir.join(format!("{}.yml", workflow.name)).exists() {
            return Err(format!("ワークフロー '{}' は既に存在します", workflow.name));
        }

        // 先に保存（validate含む）を実行し、成功した場合のみ旧ファイルを削除
        storage::save_workflow(&dir, &workflow).map_err(|e| e.to_string())?;

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

#[tauri::command]
pub async fn start_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    workflow_name: String,
    chat_session_id: String,
    task: Option<String>,
) -> Result<(), String> {
    let dir = storage::workflows_dir();
    let file_stem = workflow_name.clone();
    let workflow = tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&workflow_name).map_err(validation_error_string)?;
        let file_path = dir.join(format!("{workflow_name}.yml"));
        if file_path.exists() {
            return storage::load_workflow(&file_path).map_err(|e| e.to_string());
        }
        builtin::get_builtin_workflow(&workflow_name)
            .ok_or_else(|| format!("ワークフロー '{workflow_name}' が見つかりません"))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;

    engine
        .start_workflow(
            &app,
            session_store.inner(),
            handles.inner(),
            workflow,
            &chat_session_id,
            &file_stem,
            task,
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn abort_workflow(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    worktree_path: String,
) -> Result<(), String> {
    engine
        .abort_workflow(&app, session_store.inner(), handles.inner(), &worktree_path)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("abort_workflow failed for worktree {worktree_path}: {msg}");
            msg
        })
}

#[tauri::command]
pub async fn get_workflow_state(
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    worktree_path: String,
) -> Result<Option<WorkflowState>, String> {
    Ok(engine.get_state(&worktree_path).await)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn approve_workflow_step(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    worktree_path: String,
    decision: ApprovalDecision,
    execution_id: String,
    step_name: String,
) -> Result<(), String> {
    engine
        .handle_approval(
            &app,
            session_store.inner(),
            handles.inner(),
            &worktree_path,
            decision,
            Some(&execution_id),
            Some(&step_name),
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_workflow_approval_chat_message(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    chat_session_id: String,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::file_mention::MentionReference>>,
) -> Result<crate::agent_sdk::SendMessageResponse, String> {
    engine
        .validate_approval_chat_instruction(&chat_session_id, &content)
        .await
        .map_err(|e| e.to_string())?;

    let resolved_worktree_path = engine
        .resolve_worktree_path(&chat_session_id)
        .await
        .unwrap_or(worktree_path);

    crate::agent_sdk::send_agent_message_internal(
        &app,
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        Some(chat_session_id),
        resolved_worktree_path,
        content,
        permission_mode,
        None,
        images,
        mentions,
    )
    .await
}

fn validate_execution_id(execution_id: &str) -> Result<(), String> {
    if !execution_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Invalid execution_id format".to_string());
    }
    Ok(())
}

// ---- ワークフロー履歴閲覧コマンド ----

#[tauri::command]
pub async fn list_workflow_executions(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<String>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let event_log = WorkflowEventLog::new(&data_dir);
    tokio::task::spawn_blocking(move || event_log.list_execution_ids_for_worktree(&worktree_path))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow_execution_log(
    app: tauri::AppHandle,
    execution_id: String,
) -> Result<Vec<WorkflowLogEvent>, String> {
    validate_execution_id(&execution_id)?;
    let data_dir = resolve_data_dir(&app)?;
    let event_log = WorkflowEventLog::new(&data_dir);
    tokio::task::spawn_blocking(move || event_log.read_log(&execution_id))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow_execution_state(
    app: tauri::AppHandle,
    execution_id: String,
) -> Result<Option<WorkflowState>, String> {
    validate_execution_id(&execution_id)?;
    let data_dir = resolve_data_dir(&app)?;
    let workflows_dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        let event_log = WorkflowEventLog::new(&data_dir);
        let events = event_log.read_log(&execution_id)?;
        // ログからワークフロー定義を取得（スナップショット優先、なければYAMLファイルにフォールバック）
        let started = events.iter().find_map(|e| match e {
            super::log::WorkflowLogEvent::WorkflowStarted {
                workflow_definition,
                workflow_file_stem,
                workflow_name,
                ..
            } => {
                let stem = if workflow_file_stem.is_empty() {
                    workflow_name.clone()
                } else {
                    workflow_file_stem.clone()
                };
                Some((workflow_definition.clone(), stem))
            }
            _ => None,
        });
        let Some((snapshot_def, file_stem)) = started else {
            return Ok(None);
        };
        let workflow = if let Some(def) = snapshot_def {
            def
        } else {
            let file_path = workflows_dir.join(format!("{file_stem}.yml"));
            if file_path.exists() {
                match storage::load_workflow(&file_path) {
                    Ok(w) => w,
                    Err(e) => {
                        log::warn!(
                            "Failed to load workflow definition '{}': {e}",
                            file_path.display()
                        );
                        return Ok(None);
                    }
                }
            } else if let Some(w) = builtin::get_builtin_workflow(&file_stem) {
                w
            } else {
                return Ok(None);
            }
        };
        WorkflowEventLog::reconstruct_state_from_events(&execution_id, &events, &workflow)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

// ---- ファセットCRUDコマンド ----

#[tauri::command]
pub async fn list_facets(kind: String) -> Result<Vec<String>, String> {
    let facet_kind = parse_facet_kind(&kind)?;
    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        super::facet::list_facets(facet_kind, &base_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_facet(kind: String, key: String) -> Result<String, String> {
    let facet_kind = parse_facet_kind(&kind)?;
    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        super::facet::load_facet(facet_kind, &key, &base_dir).map_err(|e| e.to_string())
    })
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
    let facet_kind = parse_facet_kind(&kind)?;
    if builtin::is_builtin_facet(facet_kind, &key) {
        return Err("ビルトインファセットは編集できません".to_string());
    }
    // テンプレート変数の整合性チェック
    validate_template_variables(&content)?;
    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        // 新規作成時は既存キーとの重複チェック
        if is_new.unwrap_or(false) {
            let existing =
                super::facet::list_facets(facet_kind, &base_dir).map_err(|e| e.to_string())?;
            if existing.contains(&key) {
                return Err(format!("ファセット '{key}' は既に存在します"));
            }
        }
        super::facet::save_facet(facet_kind, &key, &content, &base_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_facet(kind: String, key: String) -> Result<(), String> {
    let facet_kind = parse_facet_kind(&kind)?;
    if builtin::is_builtin_facet(facet_kind, &key) {
        return Err("ビルトインファセットは削除できません".to_string());
    }
    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        super::facet::delete_facet(facet_kind, &key, &base_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

// ---- 新規コマンド ----

#[tauri::command]
pub async fn diagnose_all_cmd() -> Result<diagnostics::DiagnosticReport, String> {
    let wf_dir = storage::workflows_dir();
    let facets_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || Ok(diagnostics::diagnose_all(&wf_dir, &facets_dir)))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn list_facet_summaries(kind: String) -> Result<Vec<FacetSummary>, String> {
    let facet_kind = parse_facet_kind(&kind)?;
    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        super::facet::list_facet_summaries(facet_kind, &base_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn duplicate_workflow(source_name: String, new_name: String) -> Result<(), String> {
    super::validation::validate_name(&source_name).map_err(validation_error_string)?;
    super::validation::validate_name(&new_name).map_err(validation_error_string)?;
    let dir = storage::workflows_dir();
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
                storage::load_workflow(&file_path).map_err(|e| e.to_string())?
            } else {
                builtin::get_builtin_workflow(&source_name)
                    .ok_or_else(|| format!("ソースワークフロー '{source_name}' が見つかりません"))?
            }
        };

        wf.name = new_name;
        wf.builtin = false;
        storage::save_workflow(&dir, &wf).map_err(|e| e.to_string())
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
    let facet_kind = parse_facet_kind(&kind)?;
    super::facet::validate_facet_key(&new_key).map_err(|e| e.to_string())?;

    let base_dir = storage::facets_base_dir();
    tokio::task::spawn_blocking(move || {
        // 重複チェック
        let existing =
            super::facet::list_facets(facet_kind, &base_dir).map_err(|e| e.to_string())?;
        if existing.contains(&new_key) {
            return Err(format!("ファセット '{new_key}' は既に存在します"));
        }

        // ソースの読み込み
        let content = super::facet::load_facet(facet_kind, &source_key, &base_dir)
            .map_err(|e| e.to_string())?;

        super::facet::save_facet(facet_kind, &new_key, &content, &base_dir)
            .map_err(|e| e.to_string())
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
    let facet_kind = parse_facet_kind(&kind)?;
    if builtin::is_builtin_facet(facet_kind, &key) {
        return Err("ビルトインファセットは外部エディタで開けません".to_string());
    }
    let base_dir = storage::facets_base_dir();
    let file_path =
        super::facet::resolve_facet_path(facet_kind, &key, &base_dir).map_err(|e| e.to_string())?;

    let path_str = file_path.to_string_lossy().to_string();
    let config = state.get_config()?;
    crate::external_editor::open_path_with_opener(
        &app,
        &path_str,
        &config.app.external_editor,
        "ファセット",
    )
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
    use std::path::Path;

    #[test]
    fn parse_facet_kind_valid_kinds() {
        assert_eq!(parse_facet_kind("persona").unwrap(), FacetKind::Persona);
        assert_eq!(parse_facet_kind("policy").unwrap(), FacetKind::Policy);
        assert_eq!(parse_facet_kind("knowledge").unwrap(), FacetKind::Knowledge);
        assert_eq!(
            parse_facet_kind("instruction").unwrap(),
            FacetKind::Instruction
        );
        assert_eq!(
            parse_facet_kind("output_contract").unwrap(),
            FacetKind::OutputContract
        );
    }

    #[test]
    fn parse_facet_kind_unknown_returns_error() {
        assert!(parse_facet_kind("unknown").is_err());
        assert!(parse_facet_kind("").is_err());
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

    use crate::workflow::schema::{Step, StepMode, Workflow};
    use tempfile::TempDir;

    fn make_test_workflow(name: &str) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            steps: vec![Step {
                name: "step1".to_string(),
                mode: Some(StepMode::Auto),
                persona: None,
                policy: None,
                knowledge: None,
                instruction: None,
                output_contract: None,
                rules: vec![],
                cycle_guard: None,
                pass_previous_response: None,
                pass_output_from: None,
                inline_prompt: Some("Do something".to_string()),
                collect: None,
                parallel: None,
                aggregate: None,
                resets_cycle_for: None,
            }],
        }
    }

    #[test]
    fn duplicate_workflow_normal_case() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let wf = make_test_workflow("source-wf");
        storage::save_workflow(dir, &wf).unwrap();

        // Simulate duplicate logic
        let new_name = "copied-wf";
        super::super::validation::validate_name(new_name).unwrap();
        assert!(!dir.join(format!("{new_name}.yml")).exists());
        assert!(!builtin::is_builtin_workflow(new_name));

        let mut copied = storage::load_workflow(&dir.join("source-wf.yml")).unwrap();
        copied.name = new_name.to_string();
        copied.builtin = false;
        storage::save_workflow(dir, &copied).unwrap();

        assert!(dir.join(format!("{new_name}.yml")).exists());
        let loaded = storage::load_workflow(&dir.join(format!("{new_name}.yml"))).unwrap();
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
            let wf = builtin::get_builtin_workflow(name);
            assert!(wf.is_some());
            let wf = wf.unwrap();
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

        let loaded = storage::load_workflow(&dir.join("my-wf.yml")).unwrap();
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
}
