use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::client::SaveWorkflowSourceResultDto;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn save_workflow_source_shared(
    state: &AppState,
    source: String,
    original_name: Option<String>,
) -> Result<SaveWorkflowSourceResultDto, AppError> {
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        crate::adaptor::presenter::workflow::save_workflow_source_result(
            usecase.save_workflow_source_with_diagnostics(&source, original_name.as_deref()),
        )
    })
    .await
    .map_err(|e| AppError::from_failure(crate::domain::failure::TechnicalFailure::from(e)))?
}

pub(crate) async fn delete_workflow_shared(state: &AppState, name: String) -> Result<(), AppError> {
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase
            .delete_workflow(&name)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::from_failure(crate::domain::failure::TechnicalFailure::from(e)))?
}

pub(crate) fn open_workflow_in_editor_shared(
    state: &AppState,
    name: String,
) -> Result<(), AppError> {
    state
        .workflow_usecase
        .open_workflow_in_editor(&name)
        .map_err(AppError::from_failure)
}

pub(crate) async fn duplicate_workflow_shared(
    state: &AppState,
    source_name: String,
    new_name: String,
) -> Result<(), AppError> {
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase
            .duplicate_workflow(&source_name, &new_name)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::from_failure(crate::domain::failure::TechnicalFailure::from(e)))?
}
