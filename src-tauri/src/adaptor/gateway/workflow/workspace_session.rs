use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::session::{
    workflow_step_context_mapper, SessionState, SessionStore, SessionSummary,
};
use crate::usecase::workflow::{
    WorkspaceSessionGateway, WorkspaceSessionInput, WorkspaceSessionState,
};

pub(crate) struct StoredWorkspaceSessionGateway {
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
}

impl StoredWorkspaceSessionGateway {
    pub(crate) fn new(session_store: Arc<SessionStore>, data_dir: PathBuf) -> Self {
        Self {
            session_store,
            data_dir,
        }
    }

    fn session_input(session: SessionSummary) -> WorkspaceSessionInput {
        WorkspaceSessionInput {
            id: session.id,
            worktree_path: session.worktree_path,
            state: workspace_session_state(session.state),
            updated_at: session.updated_at,
            first_message: session.first_message,
            workflow_step_session: session.workflow_step_session,
            workflow_step_context: session
                .workflow_step_context
                .map(workflow_step_context_mapper::to_domain),
        }
    }
}

impl WorkspaceSessionGateway for StoredWorkspaceSessionGateway {
    fn list_active_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        self.session_store
            .list_sessions(&self.data_dir, worktree_path)
            .map(|sessions| sessions.into_iter().map(Self::session_input).collect())
            .map_err(WorkflowError::external)
    }

    fn list_closed_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        self.session_store
            .list_closed_sessions(&self.data_dir, worktree_path)
            .map(|sessions| sessions.into_iter().map(Self::session_input).collect())
            .map_err(WorkflowError::external)
    }
}

fn workspace_session_state(state: SessionState) -> WorkspaceSessionState {
    match state {
        SessionState::Active => WorkspaceSessionState::Active,
        SessionState::Idle => WorkspaceSessionState::Idle,
        SessionState::Done => WorkspaceSessionState::Done,
        SessionState::Error => WorkspaceSessionState::Error,
        SessionState::Closed => WorkspaceSessionState::Closed,
        SessionState::Archived => WorkspaceSessionState::Archived,
    }
}
