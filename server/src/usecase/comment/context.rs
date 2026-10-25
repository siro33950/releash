use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::comment::{ensure_session_can_review, Author, Error};
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
    pub async fn session_for_read(&self, id: &str) -> Result<Option<(String, Author)>, Error> {
        let Some(session) = self.session(id).await? else {
            return Ok(None);
        };
        Ok(Some(Self::session_context(session.session(), id)))
    }

    pub async fn session_for_write(&self, id: &str) -> Result<(String, Author), Error> {
        let session = self
            .session(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Session not found: {id}")))?;
        ensure_session_can_review(session.session().lifecycle(), id)?;
        Ok(Self::session_context(session.session(), id))
    }

    async fn session(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::agent_session::repository::VersionedAgentSession>, Error>
    {
        if id.trim().is_empty() {
            return Err(Error::InvalidInput("Session ID must not be empty".into()));
        }
        self.sessions.find(id).await.map_err(technical_error)
    }

    fn session_context(
        session: &crate::domain::agent_session::aggregates::AgentSession,
        id: &str,
    ) -> (String, Author) {
        let provider = match session.provider() {
            crate::domain::provider_lifecycle::ProviderKind::Claude => "claude",
            crate::domain::provider_lifecycle::ProviderKind::Codex => "codex",
        };
        (
            session.workspace().as_str().into(),
            Author::provider_agent(provider.into(), Some(id.into())),
        )
    }

    pub async fn worktree(&self, path: &str) -> Result<(String, Author), Error> {
        let path =
            self.worktrees
                .workspace_worktree_path(path)
                .await
                .map_err(|error| match error {
                    crate::domain::workflow::WorkflowError::NotFound(reason) => {
                        Error::NotFound(reason)
                    }
                    crate::domain::workflow::WorkflowError::Validation(reason) => {
                        Error::InvalidInput(reason)
                    }
                    crate::domain::workflow::WorkflowError::Technical(failure) => {
                        Error::Technical(failure)
                    }
                    error => technical_error(error),
                })?;
        Ok((path, Author::human()))
    }
}
fn technical_error(error: impl Into<crate::domain::failure::StorageFailure>) -> Error {
    let failure = error.into();
    Error::Technical(crate::domain::failure::TechnicalFailure {
        nature: failure.nature,
        message: failure.to_string(),
    })
}

#[cfg(test)]
#[path = "context_test.rs"]
mod context_tests;
