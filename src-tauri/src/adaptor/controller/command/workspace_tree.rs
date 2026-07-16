use std::sync::Arc;

use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::{
    CloseWorkspaceNodeCommand, WorkflowRuntimeUsecase, WorkspaceNodeCommandUsecase,
    WorkspaceNodeDetailDto, WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto,
};

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_workspace_worktree_nodes",
    "list_workspace_workflow_history",
    "get_workspace_node_detail",
    "get_workspace_session_node_id",
    "close_workspace_node",
    "approve_workspace_node",
    "archive_workspace_workflow_execution",
    "restore_workspace_workflow_execution",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        list_workspace_worktree_nodes,
        list_workspace_workflow_history,
        get_workspace_node_detail,
        get_workspace_session_node_id,
        close_workspace_node,
        approve_workspace_node,
        archive_workspace_workflow_execution,
        restore_workspace_workflow_execution,
    ]
}

#[tauri::command]
pub async fn list_workspace_worktree_nodes(
    app_state: State<'_, AppState>,
    worktree_path: String,
) -> Result<WorkspaceTreeSnapshotDto, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    let nodes = tokio::task::spawn_blocking(move || {
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&worktree_path)
            .map_err(|e| e.to_string())?;
        workflow_usecase
            .list_workspace_tree_nodes(&worktree_path, sessions)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(nodes)
}

#[tauri::command]
pub async fn list_workspace_workflow_history(
    app_state: State<'_, AppState>,
    worktree_path: String,
) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    let history = tokio::task::spawn_blocking(move || {
        workflow_usecase
            .list_workspace_workflow_history(&worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(history)
}

#[tauri::command]
pub async fn get_workspace_node_detail(
    app_state: State<'_, AppState>,
    worktree_path: String,
    node_id: String,
) -> Result<Option<WorkspaceNodeDetailDto>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&worktree_path)
            .map_err(|e| e.to_string())?;
        workflow_usecase
            .get_workspace_node_detail(&worktree_path, &node_id, sessions)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workspace_session_node_id(
    app_state: State<'_, AppState>,
    worktree_path: String,
    session_id: String,
) -> Result<Option<String>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&worktree_path)
            .map_err(|e| e.to_string())?;
        workflow_usecase
            .get_workspace_session_node_id(&worktree_path, &session_id, sessions)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn close_workspace_node(
    usecase: State<'_, Arc<WorkspaceNodeCommandUsecase>>,
    worktree_path: String,
    node_id: String,
) -> Result<(), String> {
    usecase
        .close_workspace_node(CloseWorkspaceNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn approve_workspace_node(
    app_state: State<'_, AppState>,
    runtime: State<'_, Arc<WorkflowRuntimeUsecase>>,
    worktree_path: String,
    node_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    let command = tokio::task::spawn_blocking(move || {
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&worktree_path)
            .map_err(|e| e.to_string())?;
        workflow_usecase
            .resolve_workspace_node_approval(&worktree_path, &node_id, sessions)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    runtime
        .resolve_approval(command)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn archive_workspace_workflow_execution(
    app_state: State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .archive_workspace_workflow_execution(&worktree_path, &execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn restore_workspace_workflow_execution(
    app_state: State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .restore_workspace_workflow_execution(&worktree_path, &execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::domain::workflow::WorkflowError;
    use crate::usecase::workflow::{
        WorkspaceNodeActionResolver, WorkspaceSessionGateway, WorkspaceSessionInput,
        WorkspaceSessionState,
    };

    struct FakeWorkspaceSessionGateway {
        active: Vec<WorkspaceSessionInput>,
        closed: Vec<WorkspaceSessionInput>,
        requested_worktree_paths: Arc<Mutex<Vec<String>>>,
    }

    impl WorkspaceSessionGateway for FakeWorkspaceSessionGateway {
        fn list_active_sessions(
            &self,
            worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            self.requested_worktree_paths
                .lock()
                .unwrap()
                .push(worktree_path.to_string());
            Ok(self.active.clone())
        }

        fn list_closed_sessions(
            &self,
            worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            self.requested_worktree_paths
                .lock()
                .unwrap()
                .push(worktree_path.to_string());
            Ok(self.closed.clone())
        }
    }

    fn session(
        id: &str,
        worktree_path: &str,
        state: WorkspaceSessionState,
        workflow_node_session: bool,
    ) -> WorkspaceSessionInput {
        WorkspaceSessionInput {
            id: id.to_string(),
            worktree_path: worktree_path.to_string(),
            state,
            updated_at: 2.0,
            first_message: id.to_string(),
            workflow_node_session,
            workflow_execution_id: None,
        }
    }

    #[test]
    fn workspace_session_collection_includes_closed_workflow_nodes_only() {
        let worktree = tempfile::tempdir().unwrap();
        let canonical_worktree = worktree.path().canonicalize().unwrap();
        let canonical_worktree = canonical_worktree.to_string_lossy().to_string();
        let requested_worktree_paths = Arc::new(Mutex::new(Vec::new()));
        let gateway = FakeWorkspaceSessionGateway {
            active: vec![session(
                "active",
                &canonical_worktree,
                WorkspaceSessionState::Active,
                false,
            )],
            closed: vec![
                session(
                    "closed-regular",
                    &canonical_worktree,
                    WorkspaceSessionState::Closed,
                    false,
                ),
                session(
                    "closed-node",
                    &canonical_worktree,
                    WorkspaceSessionState::Closed,
                    true,
                ),
            ],
            requested_worktree_paths: requested_worktree_paths.clone(),
        };
        let workflow_usecase =
            crate::adaptor::controller::wiring::build_workflow_usecase_with_workspace_sessions(
                worktree.path(),
                std::sync::Arc::new(gateway),
            );

        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&format!("{canonical_worktree}/"))
            .unwrap();

        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["active", "closed-node"]
        );
        assert_eq!(
            *requested_worktree_paths.lock().unwrap(),
            vec![canonical_worktree.clone(), canonical_worktree],
            "Session collection must use the same canonical worktree identity for active and closed inputs"
        );
    }

    #[test]
    fn workspace_close_resolver_maps_opaque_node_id_to_direct_session() {
        let worktree = tempfile::tempdir().unwrap();
        let canonical_worktree = worktree.path().canonicalize().unwrap();
        let canonical_worktree = canonical_worktree.to_string_lossy().to_string();
        let gateway = FakeWorkspaceSessionGateway {
            active: vec![session(
                "direct-session",
                &canonical_worktree,
                WorkspaceSessionState::Idle,
                false,
            )],
            closed: Vec::new(),
            requested_worktree_paths: Arc::new(Mutex::new(Vec::new())),
        };
        let workflow_usecase =
            crate::adaptor::controller::wiring::build_workflow_usecase_with_workspace_sessions(
                worktree.path(),
                Arc::new(gateway),
            );
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&canonical_worktree)
            .unwrap();
        let snapshot = workflow_usecase
            .list_workspace_tree_nodes(&canonical_worktree, sessions)
            .unwrap();
        let node_id = snapshot.preferred_node_id.unwrap();

        let target = workflow_usecase
            .resolve_close_target(&canonical_worktree, &node_id)
            .unwrap();

        assert_eq!(target.session_id, "direct-session");
    }
}
