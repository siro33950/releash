use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::app_data_dir::resolve_data_dir;
use crate::domain::workflow::{
    RunId, RunStatus, WorkflowRunSummary, WorkflowStateSnapshot, STEP_STATE_RUNNING,
};
use crate::usecase::agent_session::session::{
    now_timestamp, SessionState, SessionStore, SessionSummary,
};

const DEFAULT_SESSION_TITLE: &str = "NewSession";
const WORKFLOW_ARCHIVES_FILE: &str = "workflow_run_archives.json";
const ARCHIVE_REASON_AUTO_NO_SESSIONS: &str = "auto_no_sessions";
const ARCHIVE_REASON_MANUAL: &str = "manual";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceTreeNodeDto {
    Session(WorkspaceSessionNodeDto),
    Workflow(WorkspaceWorkflowNodeDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionNodeDto {
    pub id: String,
    pub worktree_path: String,
    pub title: String,
    pub state: SessionState,
    pub updated_at: f64,
    pub workflow_step_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowNodeDto {
    pub run_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub children: Vec<WorkspaceSessionNodeDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowHistoryItemDto {
    pub run_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub archived_at: f64,
    pub archive_reason: String,
    pub children: Vec<WorkspaceSessionNodeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StepSessionRef {
    session_id: String,
    step_name: String,
    run_index: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunArchiveRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restored_at: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunArchiveIndex {
    #[serde(default)]
    runs: BTreeMap<String, WorkflowRunArchiveRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowArchiveIndexSavePolicy {
    Always,
    IfChanged,
}

#[tauri::command]
pub async fn list_workspace_worktree_nodes(
    app_state: State<'_, AppState>,
    session_store: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<WorkspaceTreeNodeDto>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let sessions = session_store.list_sessions(&data_dir, &worktree_path)?;
    let workflow_usecase = app_state.workflow_usecase.clone();
    let archive_index_lock = app_state.workflow_archive_index_lock.clone();
    let nodes = tokio::task::spawn_blocking(move || {
        with_workflow_archive_index(
            &archive_index_lock,
            &data_dir,
            WorkflowArchiveIndexSavePolicy::IfChanged,
            |archives| {
                let runs = workflow_usecase
                    .list_runs_for_worktree(None, &worktree_path)
                    .map_err(|e| e.to_string())?;
                let mut states = HashMap::new();
                for run in &runs {
                    let state = workflow_usecase
                        .get_run_state(&run.run_id)
                        .map_err(|e| e.to_string())?;
                    states.insert(run.run_id.clone(), state);
                }
                Ok(project_workspace_tree_nodes(
                    sessions,
                    runs,
                    states,
                    archives,
                    now_timestamp(),
                ))
            },
        )
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(nodes)
}

#[tauri::command]
pub async fn list_workspace_workflow_history(
    app_state: State<'_, AppState>,
    session_store: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let sessions = session_store.list_sessions(&data_dir, &worktree_path)?;
    let workflow_usecase = app_state.workflow_usecase.clone();
    let archive_index_lock = app_state.workflow_archive_index_lock.clone();
    let history = tokio::task::spawn_blocking(move || {
        with_workflow_archive_index(
            &archive_index_lock,
            &data_dir,
            WorkflowArchiveIndexSavePolicy::IfChanged,
            |archives| {
                let runs = workflow_usecase
                    .list_runs_for_worktree(None, &worktree_path)
                    .map_err(|e| e.to_string())?;
                let mut states = HashMap::new();
                for run in &runs {
                    let state = workflow_usecase
                        .get_run_state(&run.run_id)
                        .map_err(|e| e.to_string())?;
                    states.insert(run.run_id.clone(), state);
                }
                Ok(project_workspace_workflow_history(
                    sessions,
                    runs,
                    states,
                    archives,
                    now_timestamp(),
                ))
            },
        )
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(history)
}

#[tauri::command]
pub async fn archive_workspace_workflow_run(
    app_state: State<'_, AppState>,
    app: tauri::AppHandle,
    worktree_path: String,
    run_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let run_id = RunId::new(run_id).map_err(|e| e.to_string())?;
    let workflow_usecase = app_state.workflow_usecase.clone();
    let run_id_string = run_id.to_string();
    let authorized = tokio::task::spawn_blocking(move || {
        workflow_usecase
            .authorize_run_summary_for_worktree(&run_id_string, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    if authorized.is_none() {
        return Err(format!("Workflow run not found: {run_id}"));
    }

    let archive_index_lock = app_state.workflow_archive_index_lock.clone();
    let run_id = run_id.to_string();
    tokio::task::spawn_blocking(move || {
        with_workflow_archive_index(
            &archive_index_lock,
            &data_dir,
            WorkflowArchiveIndexSavePolicy::Always,
            |archives| {
                let record = archives.runs.entry(run_id).or_default();
                record.archived_at = Some(now_timestamp());
                record.archive_reason = Some(ARCHIVE_REASON_MANUAL.to_string());
                Ok(())
            },
        )
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn restore_workspace_workflow_run(
    app_state: State<'_, AppState>,
    app: tauri::AppHandle,
    worktree_path: String,
    run_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let run_id = RunId::new(run_id).map_err(|e| e.to_string())?;
    let workflow_usecase = app_state.workflow_usecase.clone();
    let run_id_string = run_id.to_string();
    let authorized = tokio::task::spawn_blocking(move || {
        workflow_usecase
            .authorize_run_summary_for_worktree(&run_id_string, &worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    if authorized.is_none() {
        return Err(format!("Workflow run not found: {run_id}"));
    }

    let archive_index_lock = app_state.workflow_archive_index_lock.clone();
    let run_id = run_id.to_string();
    tokio::task::spawn_blocking(move || {
        with_workflow_archive_index(
            &archive_index_lock,
            &data_dir,
            WorkflowArchiveIndexSavePolicy::Always,
            |archives| {
                let record = archives.runs.entry(run_id).or_default();
                record.archived_at = None;
                record.archive_reason = None;
                record.restored_at = Some(now_timestamp());
                Ok(())
            },
        )
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

fn project_workspace_tree_nodes(
    sessions: Vec<SessionSummary>,
    runs: Vec<WorkflowRunSummary>,
    states: HashMap<String, Option<WorkflowStateSnapshot>>,
    archives: &mut WorkflowRunArchiveIndex,
    now: f64,
) -> Vec<WorkspaceTreeNodeDto> {
    let mut direct_sessions = Vec::new();
    let mut workflow_sessions: HashMap<String, SessionSummary> = HashMap::new();

    for session in sessions {
        if session.workflow_step_session {
            workflow_sessions.insert(session.id.clone(), session);
        } else {
            direct_sessions.push(session_node(session, None, None));
        }
    }

    direct_sessions.sort_by(compare_session_nodes);

    let mut workflow_nodes: Vec<WorkspaceWorkflowNodeDto> = runs
        .into_iter()
        .filter_map(|run| {
            let step_refs = states
                .get(&run.run_id)
                .and_then(|state| state.as_ref())
                .map(collect_step_session_refs)
                .unwrap_or_default();
            let node = workflow_node(run.clone(), &workflow_sessions, step_refs);
            apply_auto_archive_if_needed(&run, &node, archives, now);
            if is_workflow_archived(&run.run_id, archives) {
                None
            } else {
                Some(node)
            }
        })
        .collect();

    workflow_nodes.sort_by(|a, b| compare_titles(&a.title, &b.title));

    direct_sessions
        .into_iter()
        .map(WorkspaceTreeNodeDto::Session)
        .chain(
            workflow_nodes
                .into_iter()
                .map(WorkspaceTreeNodeDto::Workflow),
        )
        .collect()
}

fn workflow_node(
    run: WorkflowRunSummary,
    workflow_sessions: &HashMap<String, SessionSummary>,
    step_refs: Vec<StepSessionRef>,
) -> WorkspaceWorkflowNodeDto {
    let mut children = step_refs
        .into_iter()
        .filter_map(|step_ref| {
            workflow_sessions
                .get(&step_ref.session_id)
                .cloned()
                .map(|session| session_node(session, Some(step_ref.step_name), step_ref.run_index))
        })
        .collect::<Vec<_>>();
    children.sort_by(compare_session_nodes);

    WorkspaceWorkflowNodeDto {
        run_id: run.run_id.clone(),
        worktree_path: run.worktree_path.clone(),
        title: workflow_title(&run),
        status: run_status_label(run.status).to_string(),
        updated_at: run.updated_at,
        children,
    }
}

fn project_workspace_workflow_history(
    sessions: Vec<SessionSummary>,
    runs: Vec<WorkflowRunSummary>,
    states: HashMap<String, Option<WorkflowStateSnapshot>>,
    archives: &mut WorkflowRunArchiveIndex,
    now: f64,
) -> Vec<WorkspaceWorkflowHistoryItemDto> {
    let workflow_sessions = sessions
        .into_iter()
        .filter(|session| session.workflow_step_session)
        .map(|session| (session.id.clone(), session))
        .collect::<HashMap<_, _>>();

    let mut history = runs
        .into_iter()
        .filter_map(|run| {
            let step_refs = states
                .get(&run.run_id)
                .and_then(|state| state.as_ref())
                .map(collect_step_session_refs)
                .unwrap_or_default();
            let node = workflow_node(run.clone(), &workflow_sessions, step_refs);
            apply_auto_archive_if_needed(&run, &node, archives, now);
            let record = archives.runs.get(&run.run_id)?;
            let archived_at = record.archived_at?;
            Some(WorkspaceWorkflowHistoryItemDto {
                run_id: node.run_id,
                worktree_path: node.worktree_path,
                title: node.title,
                status: node.status,
                updated_at: node.updated_at,
                archived_at,
                archive_reason: record
                    .archive_reason
                    .clone()
                    .unwrap_or_else(|| ARCHIVE_REASON_MANUAL.to_string()),
                children: node.children,
            })
        })
        .collect::<Vec<_>>();

    history.sort_by(|a, b| {
        b.archived_at
            .partial_cmp(&a.archived_at)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| compare_titles(&a.title, &b.title))
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    history
}

fn apply_auto_archive_if_needed(
    run: &WorkflowRunSummary,
    node: &WorkspaceWorkflowNodeDto,
    archives: &mut WorkflowRunArchiveIndex,
    now: f64,
) {
    if !run.status.is_terminal() || !node.children.is_empty() {
        return;
    }
    let record = archives.runs.entry(run.run_id.clone()).or_default();
    if record.restored_at.is_some() || record.archived_at.is_some() {
        return;
    }
    record.archived_at = Some(now);
    record.archive_reason = Some(ARCHIVE_REASON_AUTO_NO_SESSIONS.to_string());
}

fn is_workflow_archived(run_id: &str, archives: &WorkflowRunArchiveIndex) -> bool {
    archives
        .runs
        .get(run_id)
        .and_then(|record| record.archived_at)
        .is_some()
}

fn session_node(
    session: SessionSummary,
    step_name: Option<String>,
    run_index: Option<u32>,
) -> WorkspaceSessionNodeDto {
    let title = step_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let first_message = session.first_message.trim();
            (!first_message.is_empty()).then(|| first_message.to_string())
        })
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());

    WorkspaceSessionNodeDto {
        id: session.id,
        worktree_path: session.worktree_path,
        title,
        state: session.state,
        updated_at: session.updated_at,
        workflow_step_session: session.workflow_step_session,
        step_name,
        run_index,
    }
}

fn workflow_title(run: &WorkflowRunSummary) -> String {
    run.task
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let workflow_name = run.workflow_name.trim();
            if workflow_name.is_empty() {
                run.run_id.clone()
            } else {
                workflow_name.to_string()
            }
        })
}

fn collect_step_session_refs(state: &WorkflowStateSnapshot) -> Vec<StepSessionRef> {
    let mut refs = Vec::new();
    if let Some(session_id) = state.current_session_id.as_ref() {
        refs.push(StepSessionRef {
            session_id: session_id.clone(),
            step_name: state.current_step_name.clone(),
            run_index: state
                .step_execution_counts
                .get(&state.current_step_name)
                .copied()
                .or(Some(1)),
        });
    }
    for entry in &state.step_history {
        if let Some(session_id) = entry.session_id.as_ref() {
            refs.push(StepSessionRef {
                session_id: session_id.clone(),
                step_name: entry.step_name.clone(),
                run_index: Some(entry.run_index),
            });
        }
        if let Some(children) = entry.child_outputs.as_ref() {
            refs.extend(children.iter().filter_map(|child| {
                child.session_id.as_ref().map(|session_id| StepSessionRef {
                    session_id: session_id.clone(),
                    step_name: child.step_name.clone(),
                    run_index: Some(child.run_index),
                })
            }));
        }
    }
    refs.extend(state.active_parallel_steps.iter().filter_map(|step| {
        step.session_id.as_ref().map(|session_id| StepSessionRef {
            session_id: session_id.clone(),
            step_name: step.step_name.clone(),
            run_index: Some(step.run_index),
        })
    }));
    refs.sort_by(|a, b| {
        compare_titles(&a.step_name, &b.step_name)
            .then_with(|| a.run_index.cmp(&b.run_index))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    refs.dedup_by(|a, b| a.session_id == b.session_id);
    refs
}

fn compare_session_nodes(
    a: &WorkspaceSessionNodeDto,
    b: &WorkspaceSessionNodeDto,
) -> std::cmp::Ordering {
    compare_titles(&a.title, &b.title).then_with(|| a.id.cmp(&b.id))
}

fn compare_titles(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => STEP_STATE_RUNNING,
        RunStatus::WaitingApproval => "waiting_approval",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Aborted => "aborted",
    }
}

fn workflow_archive_index_path(data_dir: &Path) -> PathBuf {
    data_dir.join(WORKFLOW_ARCHIVES_FILE)
}

fn load_workflow_archive_index(data_dir: &Path) -> Result<WorkflowRunArchiveIndex, String> {
    let path = workflow_archive_index_path(data_dir);
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse workflow run archives: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(WorkflowRunArchiveIndex::default())
        }
        Err(e) => Err(format!("Failed to read workflow run archives: {e}")),
    }
}

fn with_workflow_archive_index<R>(
    lock: &tokio::sync::Mutex<()>,
    data_dir: &Path,
    save_policy: WorkflowArchiveIndexSavePolicy,
    update: impl FnOnce(&mut WorkflowRunArchiveIndex) -> Result<R, String>,
) -> Result<R, String> {
    let _guard = lock.blocking_lock();
    let mut archives = load_workflow_archive_index(data_dir)?;
    let original = if save_policy == WorkflowArchiveIndexSavePolicy::IfChanged {
        Some(archives.clone())
    } else {
        None
    };
    let result = update(&mut archives)?;
    match original {
        Some(original) => {
            save_workflow_archive_index_if_changed(data_dir, &original, &archives)?;
        }
        None => save_workflow_archive_index(data_dir, &archives)?,
    }
    Ok(result)
}

fn save_workflow_archive_index_if_changed(
    data_dir: &Path,
    original: &WorkflowRunArchiveIndex,
    updated: &WorkflowRunArchiveIndex,
) -> Result<bool, String> {
    if original == updated {
        return Ok(false);
    }
    save_workflow_archive_index(data_dir, updated)?;
    Ok(true)
}

fn save_workflow_archive_index(
    data_dir: &Path,
    index: &WorkflowRunArchiveIndex,
) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|e| format!("Failed to create app data directory: {e}"))?;
    let path = workflow_archive_index_path(data_dir);
    let json = serde_json::to_string_pretty(index)
        .map_err(|e| format!("Failed to serialize workflow run archives: {e}"))?;
    atomic_write(&path, &json).map_err(|e| format!("Failed to write workflow run archives: {e}"))
}

fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
        })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp, content)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ChildOutputSnapshot, ParallelStepState, TriggerSource, WorkflowDefinition,
        WorkflowExecutionState, STEP_STATE_COMPLETED,
    };
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    fn session(id: &str, title: &str, workflow_step_session: bool) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            worktree_path: "/repo/wt".to_string(),
            state: SessionState::Active,
            created_at: 1.0,
            updated_at: 2.0,
            first_message: title.to_string(),
            message_count: 1,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            backend_id: None,
            workflow_step_session,
        }
    }

    fn run(run_id: &str, task: &str) -> WorkflowRunSummary {
        run_with_status(run_id, task, RunStatus::Running)
    }

    fn run_with_status(run_id: &str, task: &str, status: RunStatus) -> WorkflowRunSummary {
        WorkflowRunSummary {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: Some(task.to_string()),
            status,
            worktree_path: "/repo/wt".to_string(),
            current_node_name: Some("build".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 3.0,
            completed_at: None,
            error_reason: None,
        }
    }

    fn state(run_id: &str) -> WorkflowStateSnapshot {
        WorkflowStateSnapshot {
            execution_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "build".to_string(),
            current_session_id: Some("step-build".to_string()),
            total_steps: 2,
            step_history: vec![crate::domain::workflow::StepHistoryEntry {
                step_name: "plan".to_string(),
                completed_at: 1.0,
                result: None,
                session_id: Some("step-plan".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: Some(vec![ChildOutputSnapshot {
                    step_name: "child-review".to_string(),
                    session_id: Some("step-child".to_string()),
                    result: None,
                    run_index: 2,
                    completed_at: 2.0,
                    structured_output: None,
                    output_contract: None,
                    state: STEP_STATE_COMPLETED.to_string(),
                }]),
                state: STEP_STATE_COMPLETED.to_string(),
            }],
            step_execution_counts: HashMap::from([("build".to_string(), 3)]),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::new(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![ParallelStepState {
                step_name: "parallel-lint".to_string(),
                state: STEP_STATE_RUNNING.to_string(),
                session_id: Some("step-parallel".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            }],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn projects_direct_sessions_before_workflows_and_sorts_by_title() {
        let mut archives = WorkflowRunArchiveIndex::default();
        let nodes = project_workspace_tree_nodes(
            vec![
                session("b", "Zulu", false),
                session("a", "Alpha", false),
                session("step-build", "ignored", true),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &mut archives,
            10.0,
        );

        assert!(matches!(&nodes[0], WorkspaceTreeNodeDto::Session(node) if node.title == "Alpha"));
        assert!(matches!(&nodes[1], WorkspaceTreeNodeDto::Session(node) if node.title == "Zulu"));
        assert!(
            matches!(&nodes[2], WorkspaceTreeNodeDto::Workflow(node) if node.title == "Implement")
        );
    }

    #[test]
    fn empty_direct_session_uses_default_new_session_title() {
        let mut archives = WorkflowRunArchiveIndex::default();
        let nodes = project_workspace_tree_nodes(
            vec![session("empty", "", false)],
            vec![],
            HashMap::new(),
            &mut archives,
            10.0,
        );

        assert!(
            matches!(&nodes[0], WorkspaceTreeNodeDto::Session(node) if node.title == DEFAULT_SESSION_TITLE)
        );
    }

    #[test]
    fn maps_workflow_step_sessions_to_their_parent_run_with_step_titles() {
        let mut archives = WorkflowRunArchiveIndex::default();
        let nodes = project_workspace_tree_nodes(
            vec![
                session("step-plan", "old plan title", true),
                session("step-build", "old build title", true),
                session("step-child", "old child title", true),
                session("step-parallel", "old parallel title", true),
                session("unmatched-step", "orphan", true),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &mut archives,
            10.0,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        let titles = workflow
            .children
            .iter()
            .map(|child| child.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            vec!["build", "child-review", "parallel-lint", "plan"]
        );
        assert!(workflow
            .children
            .iter()
            .any(|child| child.id == "step-build" && child.run_index == Some(3)));
        assert!(!workflow
            .children
            .iter()
            .any(|child| child.id == "unmatched-step"));
    }

    #[test]
    fn auto_archives_terminal_workflow_without_sessions() {
        let mut archives = WorkflowRunArchiveIndex::default();
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run_with_status(
                "run-1",
                "Done without sessions",
                RunStatus::Completed,
            )],
            HashMap::new(),
            &mut archives,
            20.0,
        );

        assert!(nodes.is_empty());
        let record = archives.runs.get("run-1").expect("archive record");
        assert_eq!(record.archived_at, Some(20.0));
        assert_eq!(
            record.archive_reason.as_deref(),
            Some(ARCHIVE_REASON_AUTO_NO_SESSIONS)
        );
    }

    #[test]
    fn restored_terminal_workflow_without_sessions_stays_visible() {
        let mut archives = WorkflowRunArchiveIndex {
            runs: BTreeMap::from([(
                "run-1".to_string(),
                WorkflowRunArchiveRecord {
                    restored_at: Some(15.0),
                    ..Default::default()
                },
            )]),
        };
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run_with_status("run-1", "Restored", RunStatus::Completed)],
            HashMap::new(),
            &mut archives,
            20.0,
        );

        assert!(
            matches!(&nodes[0], WorkspaceTreeNodeDto::Workflow(node) if node.run_id == "run-1")
        );
        assert_eq!(archives.runs["run-1"].archived_at, None);
    }

    #[test]
    fn manual_archive_hides_workflow_until_restored() {
        let mut archives = WorkflowRunArchiveIndex {
            runs: BTreeMap::from([(
                "run-1".to_string(),
                WorkflowRunArchiveRecord {
                    archived_at: Some(15.0),
                    archive_reason: Some(ARCHIVE_REASON_MANUAL.to_string()),
                    restored_at: Some(10.0),
                },
            )]),
        };
        let nodes = project_workspace_tree_nodes(
            vec![session("step-build", "old build title", true)],
            vec![run("run-1", "Manual")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &mut archives,
            20.0,
        );

        assert!(nodes.is_empty());
        let history = project_workspace_workflow_history(
            vec![session("step-build", "old build title", true)],
            vec![run("run-1", "Manual")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &mut archives,
            20.0,
        );
        assert_eq!(history[0].run_id, "run-1");
        assert_eq!(history[0].archive_reason, ARCHIVE_REASON_MANUAL);
    }

    #[test]
    fn unchanged_archive_index_is_not_saved() {
        let temp = tempfile::tempdir().unwrap();
        let original = WorkflowRunArchiveIndex::default();

        let saved =
            save_workflow_archive_index_if_changed(temp.path(), &original, &original).unwrap();

        assert!(!saved);
        assert!(!workflow_archive_index_path(temp.path()).exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archive_index_lock_preserves_concurrent_list_refresh_and_manual_archive_updates() {
        let temp = tempfile::tempdir().unwrap();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let (loaded_tx, loaded_rx) = mpsc::channel();

        let list_lock = lock.clone();
        let list_data_dir = temp.path().to_path_buf();
        let list_refresh = tokio::task::spawn_blocking(move || {
            with_workflow_archive_index(
                &list_lock,
                &list_data_dir,
                WorkflowArchiveIndexSavePolicy::IfChanged,
                |archives| {
                    loaded_tx.send(()).expect("list load signal");
                    std::thread::sleep(Duration::from_millis(50));
                    let record = archives.runs.entry("auto-run".to_string()).or_default();
                    record.archived_at = Some(20.0);
                    record.archive_reason = Some(ARCHIVE_REASON_AUTO_NO_SESSIONS.to_string());
                    Ok(())
                },
            )
        });

        loaded_rx.recv().expect("list refresh loaded index");

        let manual_lock = lock.clone();
        let manual_data_dir = temp.path().to_path_buf();
        let manual_archive = tokio::task::spawn_blocking(move || {
            with_workflow_archive_index(
                &manual_lock,
                &manual_data_dir,
                WorkflowArchiveIndexSavePolicy::Always,
                |archives| {
                    let record = archives.runs.entry("manual-run".to_string()).or_default();
                    record.archived_at = Some(30.0);
                    record.archive_reason = Some(ARCHIVE_REASON_MANUAL.to_string());
                    Ok(())
                },
            )
        });

        list_refresh.await.unwrap().unwrap();
        manual_archive.await.unwrap().unwrap();

        let archives = load_workflow_archive_index(temp.path()).unwrap();
        assert_eq!(
            archives.runs["auto-run"].archive_reason.as_deref(),
            Some(ARCHIVE_REASON_AUTO_NO_SESSIONS)
        );
        assert_eq!(
            archives.runs["manual-run"].archive_reason.as_deref(),
            Some(ARCHIVE_REASON_MANUAL)
        );
    }
}
