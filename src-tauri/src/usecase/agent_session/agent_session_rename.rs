use std::sync::Arc;

use crate::domain::agent_session::aggregates::AgentSessionMutationOutcome;
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError,
};

use super::AgentSessionChangeNotifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRenameError {
    NotFound,
    InvalidOperation,
    Conflict,
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait AgentSessionRenameExecutor: Send + Sync {
    async fn rename(
        &self,
        agent_session_id: &str,
        name: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionRenameError>;
}

pub(crate) struct AgentSessionRenameUsecase {
    repository: Arc<dyn AgentSessionRepository>,
    change_notifier: Arc<dyn AgentSessionChangeNotifier>,
}

impl AgentSessionRenameUsecase {
    pub(crate) fn new(
        repository: Arc<dyn AgentSessionRepository>,
        change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    ) -> Self {
        Self {
            repository,
            change_notifier,
        }
    }
}

#[async_trait::async_trait]
impl AgentSessionRenameExecutor for AgentSessionRenameUsecase {
    async fn rename(
        &self,
        agent_session_id: &str,
        name: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionRenameError> {
        let mut session = self
            .repository
            .find(agent_session_id)
            .await
            .map_err(map_repository_error)?
            .ok_or(AgentSessionRenameError::NotFound)?;
        let outcome = session
            .session_mut()
            .rename(name)
            .map_err(|_| AgentSessionRenameError::InvalidOperation)?;
        if outcome == AgentSessionMutationOutcome::AlreadyApplied {
            return Ok(outcome);
        }
        debug_assert!(session.session().manual_name().is_some());
        let worktree_path = session.session().worktree_path().to_string();
        self.repository
            .save(
                session,
                &format!(
                    "workspace-session-node-rename.{agent_session_id}.{}",
                    uuid::Uuid::new_v4()
                ),
            )
            .await
            .map_err(map_repository_error)?;
        self.change_notifier.agent_session_changed(&worktree_path);
        Ok(outcome)
    }
}

fn map_repository_error(error: AgentSessionRepositoryError) -> AgentSessionRenameError {
    match error {
        AgentSessionRepositoryError::Conflict
        | AgentSessionRepositoryError::ProviderSessionAlreadyOwned { .. } => {
            AgentSessionRenameError::Conflict
        }
        AgentSessionRepositoryError::InvalidRequest => AgentSessionRenameError::InvalidOperation,
        AgentSessionRepositoryError::Corrupt => AgentSessionRenameError::Corrupt,
        AgentSessionRepositoryError::Unavailable => AgentSessionRenameError::Unavailable,
    }
}
