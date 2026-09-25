use crate::other::AppError;
use std::collections::HashMap;
use std::sync::Arc;

use crate::adaptor::controller::state::AppState;

pub(crate) async fn diagnose_all_cmd_shared(
    state: &AppState,
    dir: Option<String>,
) -> Result<crate::usecase::workflow::diagnostic_dto::DiagnosticReport, AppError> {
    diagnose_all_impl(&state.workflow_usecase, dir).await
}

/// 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
pub(crate) async fn diagnose_all_impl(
    usecase: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    dir: Option<String>,
) -> Result<crate::usecase::workflow::diagnostic_dto::DiagnosticReport, AppError> {
    let target =
        crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::from_optional_directory(dir)
            .map_err(AppError::from_failure)?;
    let usecase = usecase.clone();
    crate::other::operation_context::spawn_blocking(move || {
        usecase.diagnose_all(target).map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn render_facet_preview_shared(
    state: &AppState,
    content: String,
    sample_values: HashMap<String, String>,
) -> Result<String, AppError> {
    Ok(state
        .workflow_usecase
        .render_facet_preview(&content, &sample_values))
}

pub(crate) fn get_automation_config_dir_shared(state: &AppState) -> Result<String, AppError> {
    state
        .workflow_usecase
        .automation_config_dir()
        .map_err(AppError::from_failure)
}
