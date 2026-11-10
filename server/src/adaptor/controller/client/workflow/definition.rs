use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) fn open_workflow_in_editor_shared(
    state: &AppState,
    name: String,
) -> Result<(), AppError> {
    state
        .workflow_usecase
        .open_workflow_in_editor(&name)
        .map_err(AppError::from_failure)
}
