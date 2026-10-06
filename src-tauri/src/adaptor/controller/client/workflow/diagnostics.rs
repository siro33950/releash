use crate::adaptor::presenter::error::AppError;
use std::collections::HashMap;

use crate::adaptor::controller::state::AppState;

pub(crate) async fn render_facet_preview_shared(
    state: &AppState,
    content: String,
    sample_values: HashMap<String, String>,
) -> Result<String, AppError> {
    Ok(state
        .workflow_usecase
        .render_facet_preview(&content, &sample_values))
}

pub(crate) async fn diagnose_workflow_directory_shared(
    state: &AppState,
    dir: String,
) -> Result<crate::usecase::workflow::diagnostic_dto::DiagnosticReport, AppError> {
    let target =
        crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::from_optional_directory(Some(
            dir,
        ))
        .map_err(AppError::from_failure)?;
    let usecase = state.workflow_usecase.clone();
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(move || {
        usecase.diagnose_all(target).map_err(AppError::from_failure)
    })
    .await
    .map_err(|error| {
        AppError::from_failure(crate::domain::failure::TechnicalFailure::from(error))
    })?
}
