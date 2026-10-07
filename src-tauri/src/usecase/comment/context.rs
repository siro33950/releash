use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::comment::{ensure_session_can_review, ReviewActor, ReviewError};
use crate::usecase::workspace_tree::WorkspaceWorktreePathQuery;
use std::sync::Arc;

pub struct ReviewContextUsecase {
    sessions: Arc<dyn AgentSessionRepository>,
    worktrees: Arc<dyn WorkspaceWorktreePathQuery>,
}
impl ReviewContextUsecase {
    pub fn new(
        sessions: Arc<dyn AgentSessionRepository>,
        worktrees: Arc<dyn WorkspaceWorktreePathQuery>,
    ) -> Self {
        Self {
            sessions,
            worktrees,
        }
    }
    pub async fn session_for_read(
        &self,
        id: &str,
    ) -> Result<Option<(String, ReviewActor)>, ReviewError> {
        let Some(session) = self.session(id).await? else {
            return Ok(None);
        };
        Ok(Some(Self::session_context(session.session(), id)))
    }

    pub async fn session_for_write(&self, id: &str) -> Result<(String, ReviewActor), ReviewError> {
        let session = self
            .session(id)
            .await?
            .ok_or_else(|| ReviewError::NotFound(format!("Session not found: {id}")))?;
        ensure_session_can_review(session.session().lifecycle(), id)?;
        Ok(Self::session_context(session.session(), id))
    }

    async fn session(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::agent_session::repository::VersionedAgentSession>, ReviewError>
    {
        if id.trim().is_empty() {
            return Err(ReviewError::InvalidInput(
                "Session ID must not be empty".into(),
            ));
        }
        self.sessions.find(id).await.map_err(technical_error)
    }

    fn session_context(
        session: &crate::domain::agent_session::aggregates::AgentSession,
        id: &str,
    ) -> (String, ReviewActor) {
        let provider = match session.provider() {
            crate::domain::provider_lifecycle::ProviderKind::Claude => "claude",
            crate::domain::provider_lifecycle::ProviderKind::Codex => "codex",
        };
        (
            session.workspace().as_str().into(),
            ReviewActor::provider_agent(provider.into(), Some(id.into())),
        )
    }

    pub async fn worktree(&self, path: &str) -> Result<(String, ReviewActor), ReviewError> {
        let path =
            self.worktrees
                .workspace_worktree_path(path)
                .await
                .map_err(|error| match error {
                    crate::domain::workflow::WorkflowError::NotFound(reason) => {
                        ReviewError::NotFound(reason)
                    }
                    crate::domain::workflow::WorkflowError::Validation(reason) => {
                        ReviewError::InvalidInput(reason)
                    }
                    crate::domain::workflow::WorkflowError::Technical(failure) => {
                        ReviewError::Technical(failure)
                    }
                    error => technical_error(error),
                })?;
        Ok((path, ReviewActor::human()))
    }
}
fn technical_error(error: impl Into<crate::domain::failure::StorageFailure>) -> ReviewError {
    let failure = error.into();
    ReviewError::Technical(crate::domain::failure::TechnicalFailure {
        nature: failure.nature,
        message: failure.to_string(),
    })
}

#[cfg(test)]
#[path = "context_test.rs"]
mod context_tests;
