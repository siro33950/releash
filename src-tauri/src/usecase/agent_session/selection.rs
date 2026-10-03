use super::{AgentSessionUsecase, AgentSessionUsecaseError};
use crate::domain::workflow::WorkflowError;
use crate::usecase::workspace_tree::{SessionNodeSelectionDto, WorkspaceQueryService};

pub(super) enum SelectionError {
    Session(AgentSessionUsecaseError),
    Query(WorkflowError),
    Missing,
}

pub(super) async fn required_selection(
    sessions: &AgentSessionUsecase,
    query: &dyn WorkspaceQueryService,
    id: &str,
) -> Result<SessionNodeSelectionDto, SelectionError> {
    let session = sessions
        .find(id)
        .await
        .map_err(SelectionError::Session)?
        .ok_or(SelectionError::Missing)?;
    query
        .session_selection(session.session().workspace(), id)
        .await
        .map_err(SelectionError::Query)?
        .ok_or(SelectionError::Missing)
}
