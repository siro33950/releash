use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn create_worktree_shared(
    state: &AppState,
    repo_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.create_worktree(&repo_path, &branch, create_branch, base_branch.as_deref())
            .map(|entry| entry.path)
    })
    .await
}

pub(crate) async fn remove_worktree_shared(
    state: &AppState,
    runtime: std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), AppError> {
    state
        .repository_usecase
        .remove_worktree(runtime.as_ref(), &repo_path, &worktree_path, force)
        .await
        .map_err(AppError::from)
}

pub(crate) fn parse_launch(
    launch: Option<crate::adaptor::presenter::client::create_worktrees_request::Launch>,
) -> Result<
    crate::usecase::create_worktrees::LaunchAfterCreation,
    crate::adaptor::presenter::client::CommandFailure,
> {
    use crate::adaptor::controller::client::invalid_request;
    use crate::adaptor::presenter::client::create_worktrees_request::Launch;
    use crate::usecase::create_worktrees::LaunchAfterCreation as L;
    Ok(match launch {
        None => L::None,
        Some(Launch::Session(session)) => L::Session {
            provider: match session.provider.as_str() {
                "claude" => crate::domain::provider_lifecycle::ProviderKind::Claude,
                "codex" => crate::domain::provider_lifecycle::ProviderKind::Codex,
                _ => return Err(invalid_request("Unknown provider")),
            },
            rows: u16::try_from(session.rows)
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| invalid_request("Invalid rows"))?,
            cols: u16::try_from(session.cols)
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| invalid_request("Invalid cols"))?,
            request_id: if session.request_id.trim().is_empty() {
                return Err(invalid_request("Missing request id"));
            } else {
                session.request_id
            },
        },
        Some(Launch::Workflow(workflow)) => L::Workflow {
            name: if workflow.name.trim().is_empty() {
                return Err(invalid_request("Missing workflow name"));
            } else {
                workflow.name
            },
            request: workflow.request,
        },
    })
}

#[cfg(test)]
#[path = "worktree_test.rs"]
mod worktree_tests;
