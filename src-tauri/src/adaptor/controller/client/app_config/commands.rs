use crate::adaptor::presenter::error::AppError;
use std::sync::Arc;

use crate::usecase::app_config::{AppConfigUsecase, WorkflowConfigInput};

fn map_join_error(error: tokio::task::JoinError) -> AppError {
    AppError::new(format!("task join error: {error}"))
}

pub(crate) async fn update_performance_telemetry_shared(
    usecase: &Arc<AppConfigUsecase>,
    enabled: bool,
) -> Result<(), AppError> {
    let usecase = usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        crate::usecase::telemetry::TelemetryUsecase::new(
            &crate::adaptor::gateway::telemetry::TelemetryGateway,
        )
        .update_performance_telemetry(&usecase, enabled)
    })
    .await
    .map_err(map_join_error)?
    .map_err(AppError::from_failure)?;
    Ok(())
}

pub(crate) async fn update_app_settings_shared(
    usecase: &Arc<AppConfigUsecase>,
    close_to_tray: bool,
    start_minimized: bool,
) -> Result<(), AppError> {
    let usecase = usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase.update_app_settings(close_to_tray, start_minimized)
    })
    .await
    .map_err(map_join_error)?
    .map_err(AppError::from_failure)
}

pub(crate) async fn update_login_item_preference_shared(
    usecase: &Arc<AppConfigUsecase>,
    requested: bool,
) -> Result<(), AppError> {
    let usecase = usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase.update_login_item_preference(requested)
    })
    .await
    .map_err(map_join_error)?
    .map_err(AppError::from_failure)
}

pub(crate) async fn update_workflow_config_shared(
    usecase: &Arc<AppConfigUsecase>,
    workflow: WorkflowConfigInput,
) -> Result<(), AppError> {
    let usecase = usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase.update_workflow_config(workflow)
    })
    .await
    .map_err(map_join_error)?
    .map_err(AppError::from_failure)
}

pub(crate) async fn update_crash_reporting_shared(
    usecase: &Arc<AppConfigUsecase>,
    enabled: bool,
) -> Result<(), AppError> {
    let usecase = usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        crate::usecase::telemetry::TelemetryUsecase::new(
            &crate::adaptor::gateway::telemetry::TelemetryGateway,
        )
        .update_crash_reporting(&usecase, enabled)
    })
    .await
    .map_err(map_join_error)?
    .map_err(AppError::from_failure)?;
    Ok(())
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod commands_tests;
