use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::comment::{ensure_session_can_review, ReviewActor, ReviewError};
use crate::usecase::workspace_tree::WorkspaceWorktreePathQuery;
use std::sync::Arc;

pub enum ReviewContextTarget<'a> {
    Session(&'a str),
    Worktree(&'a str),
}
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
    pub async fn resolve(
        &self,
        target: ReviewContextTarget<'_>,
        mutation: bool,
    ) -> Result<Option<(String, ReviewActor)>, ReviewError> {
        match target {
            ReviewContextTarget::Session(id) => {
                if id.trim().is_empty() {
                    return Err(ReviewError::InvalidInput(
                        "Session ID must not be empty".into(),
                    ));
                }
                let Some(session) = self
                    .sessions
                    .find(id)
                    .await
                    .map_err(|error| ReviewError::Store(error.into()))?
                else {
                    return Ok(None);
                };
                let session = session.session();
                if mutation {
                    ensure_session_can_review(session.lifecycle(), id)?;
                }
                let provider = match session.provider() {
                    crate::domain::provider_lifecycle::ProviderKind::Claude => "claude",
                    crate::domain::provider_lifecycle::ProviderKind::Codex => "codex",
                };
                Ok(Some((
                    session.workspace().as_str().into(),
                    ReviewActor::provider_agent(provider.into(), Some(id.into())),
                )))
            }
            ReviewContextTarget::Worktree(path) => {
                let path = self
                    .worktrees
                    .workspace_worktree_path(path)
                    .await
                    .map_err(|error| ReviewError::Store(error.into()))?;
                Ok(Some((path, ReviewActor::human())))
            }
        }
    }
}
#[cfg(test)]
#[path = "context_test.rs"]
mod context_tests;
