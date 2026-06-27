use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::{
    WorkspaceTreeNodeDto, WorkspaceWorkflowHistoryItemDto, WorkspaceWorkflowStepNodeDto,
};

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_workspace_worktree_nodes",
    "list_workspace_workflow_history",
    "get_workspace_workflow_step_detail",
    "archive_workspace_workflow_run",
    "restore_workspace_workflow_run",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        list_workspace_worktree_nodes,
        list_workspace_workflow_history,
        get_workspace_workflow_step_detail,
        archive_workspace_workflow_run,
        restore_workspace_workflow_run,
    ]
}

#[tauri::command]
pub async fn list_workspace_worktree_nodes(
    app_state: State<'_, AppState>,
    worktree_path: String,
) -> Result<Vec<WorkspaceTreeNodeDto>, String> {
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
pub async fn get_workspace_workflow_step_detail(
    app_state: State<'_, AppState>,
    worktree_path: String,
    run_id: String,
    step_id: String,
) -> Result<Option<WorkspaceWorkflowStepNodeDto>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let sessions = workflow_usecase
            .collect_workspace_session_inputs(&worktree_path)
            .map_err(|e| e.to_string())?;
        workflow_usecase
            .get_workspace_workflow_step_detail(&worktree_path, &run_id, &step_id, sessions)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn archive_workspace_workflow_run(
    app_state: State<'_, AppState>,
    worktree_path: String,
    run_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .archive_workspace_workflow_run(&worktree_path, &run_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn restore_workspace_workflow_run(
    app_state: State<'_, AppState>,
    worktree_path: String,
    run_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .restore_workspace_workflow_run(&worktree_path, &run_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::workflow::{
        WorkspaceSessionGateway, WorkspaceSessionInput, WorkspaceSessionState,
    };

    struct FakeWorkspaceSessionGateway {
        active: Vec<WorkspaceSessionInput>,
        closed: Vec<WorkspaceSessionInput>,
    }

    impl WorkspaceSessionGateway for FakeWorkspaceSessionGateway {
        fn list_active_sessions(
            &self,
            _worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            Ok(self.active.clone())
        }

        fn list_closed_sessions(
            &self,
            _worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            Ok(self.closed.clone())
        }
    }

    fn session(
        id: &str,
        state: WorkspaceSessionState,
        workflow_step_session: bool,
    ) -> WorkspaceSessionInput {
        WorkspaceSessionInput {
            id: id.to_string(),
            worktree_path: "/repo/wt".to_string(),
            state,
            updated_at: 2.0,
            first_message: id.to_string(),
            workflow_step_session,
            workflow_step_context: None,
        }
    }

    #[test]
    fn workspace_session_collection_includes_closed_workflow_steps_only() {
        let gateway = FakeWorkspaceSessionGateway {
            active: vec![session("active", WorkspaceSessionState::Active, false)],
            closed: vec![
                session("closed-regular", WorkspaceSessionState::Closed, false),
                session("closed-step", WorkspaceSessionState::Closed, true),
            ],
        };
        let workflow_usecase =
            crate::adaptor::controller::wiring::build_workflow_usecase_with_workspace_sessions(
                "/tmp/releash-test",
                std::sync::Arc::new(gateway),
            );

        let sessions = workflow_usecase
            .collect_workspace_session_inputs("/repo/wt")
            .unwrap();

        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["active", "closed-step"]
        );
    }
}
