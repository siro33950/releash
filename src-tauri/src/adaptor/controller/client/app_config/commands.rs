use crate::other::AppError;
use std::sync::Arc;

use crate::adaptor::gateway::app_config::{
    app_to_model, workflow_to_domain, workflow_to_model, AppSection, WorkflowSection,
};
use crate::domain::app_config::ConfigRepository;
use crate::usecase::app_config::AppConfigUsecase;

fn build_usecase(app_config: Arc<dyn ConfigRepository>) -> AppConfigUsecase {
    AppConfigUsecase::new(app_config)
}

fn map_join_error(error: tokio::task::JoinError) -> AppError {
    AppError::new(format!("task join error: {error}"))
}

pub(crate) async fn update_performance_telemetry_shared(
    state: &Arc<dyn ConfigRepository>,
    enabled: bool,
) -> Result<(), AppError> {
    let usecase = build_usecase(state.clone());
    tokio::task::spawn_blocking(move || {
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

pub(crate) fn get_app_settings_shared(
    state: &Arc<dyn ConfigRepository>,
) -> Result<AppSection, AppError> {
    let usecase = build_usecase(state.clone());
    let app = usecase.get_app_settings().map_err(AppError::from_failure)?;
    Ok(app_to_model(app))
}

pub(crate) async fn update_app_settings_shared(
    state: &Arc<dyn ConfigRepository>,
    close_to_tray: bool,
    start_minimized: bool,
) -> Result<(), AppError> {
    let usecase = build_usecase(state.clone());
    tokio::task::spawn_blocking(move || usecase.update_app_settings(close_to_tray, start_minimized))
        .await
        .map_err(map_join_error)?
        .map_err(AppError::from_failure)
}

pub(crate) async fn update_login_item_preference_shared(
    state: &Arc<dyn ConfigRepository>,
    requested: bool,
) -> Result<(), AppError> {
    let usecase = build_usecase(state.clone());
    tokio::task::spawn_blocking(move || usecase.update_login_item_preference(requested))
        .await
        .map_err(map_join_error)?
        .map_err(AppError::from_failure)
}

pub(crate) fn get_workflow_config_shared(
    state: &Arc<dyn ConfigRepository>,
) -> Result<WorkflowSection, AppError> {
    let usecase = build_usecase(state.clone());
    let workflow = usecase
        .get_workflow_config()
        .map_err(AppError::from_failure)?;
    Ok(workflow_to_model(workflow))
}

pub(crate) async fn update_workflow_config_shared(
    state: &Arc<dyn ConfigRepository>,
    workflow: WorkflowSection,
) -> Result<(), AppError> {
    let usecase = build_usecase(state.clone());
    let workflow = workflow_to_domain(&workflow);
    tokio::task::spawn_blocking(move || usecase.update_workflow_config(workflow))
        .await
        .map_err(map_join_error)?
        .map_err(AppError::from_failure)
}

pub(crate) fn get_performance_telemetry_enabled_shared(
    state: &Arc<dyn ConfigRepository>,
) -> Result<bool, AppError> {
    let usecase = build_usecase(state.clone());
    usecase
        .get_performance_telemetry_enabled()
        .map_err(AppError::from_failure)
}

pub(crate) async fn update_crash_reporting_shared(
    state: &Arc<dyn ConfigRepository>,
    enabled: bool,
) -> Result<(), AppError> {
    let usecase = build_usecase(state.clone());
    tokio::task::spawn_blocking(move || {
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
