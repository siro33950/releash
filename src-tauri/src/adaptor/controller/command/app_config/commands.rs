use std::sync::Arc;

use tauri::State;

use crate::adaptor::gateway::app_config::{
    app_to_domain, app_to_model, workflow_to_domain, workflow_to_model, AppSection, WorkflowSection,
};
use crate::domain::app_config::ConfigRepository;
use crate::usecase::app_config::AppConfigUsecase;

fn build_usecase(app_config: Arc<dyn ConfigRepository>) -> AppConfigUsecase {
    AppConfigUsecase::new(app_config)
}

fn map_join_error(error: tokio::task::JoinError) -> String {
    format!("task join error: {error}")
}

#[tauri::command]
pub async fn update_performance_telemetry(
    state: State<'_, Arc<dyn ConfigRepository>>,
    enabled: bool,
) -> Result<(), String> {
    let usecase = build_usecase(state.inner().clone());
    tokio::task::spawn_blocking(move || usecase.update_performance_telemetry(enabled))
        .await
        .map_err(map_join_error)?
        .map_err(String::from)?;
    crate::other::telemetry::set_performance_enabled(
        crate::infrastructure::telemetry::config::telemetry_active(
            crate::infrastructure::telemetry::config::BuildType::current(),
            crate::infrastructure::telemetry::config::endpoint(),
            crate::infrastructure::telemetry::config::license_key(),
            enabled,
        ),
    );
    Ok(())
}

#[tauri::command]
pub fn get_app_settings(state: State<'_, Arc<dyn ConfigRepository>>) -> Result<AppSection, String> {
    let usecase = build_usecase(state.inner().clone());
    let app = usecase.query().get_app_settings().map_err(String::from)?;
    Ok(app_to_model(app))
}

#[tauri::command]
pub async fn update_app_settings(
    state: State<'_, Arc<dyn ConfigRepository>>,
    app: AppSection,
) -> Result<(), String> {
    let usecase = build_usecase(state.inner().clone());
    let app = app_to_domain(&app);
    tokio::task::spawn_blocking(move || usecase.update_app_settings(app))
        .await
        .map_err(map_join_error)?
        .map_err(String::from)
}

#[tauri::command]
pub fn get_workflow_config(
    state: State<'_, Arc<dyn ConfigRepository>>,
) -> Result<WorkflowSection, String> {
    let usecase = build_usecase(state.inner().clone());
    let workflow = usecase
        .query()
        .get_workflow_config()
        .map_err(String::from)?;
    Ok(workflow_to_model(workflow))
}

#[tauri::command]
pub async fn update_workflow_config(
    state: State<'_, Arc<dyn ConfigRepository>>,
    workflow: WorkflowSection,
) -> Result<(), String> {
    let usecase = build_usecase(state.inner().clone());
    let workflow = workflow_to_domain(&workflow);
    tokio::task::spawn_blocking(move || usecase.update_workflow_config(workflow))
        .await
        .map_err(map_join_error)?
        .map_err(String::from)
}

#[tauri::command]
pub fn get_crash_reporting_enabled(
    state: State<'_, Arc<dyn ConfigRepository>>,
) -> Result<bool, String> {
    let usecase = build_usecase(state.inner().clone());
    usecase
        .query()
        .get_crash_reporting_enabled()
        .map_err(String::from)
}

#[tauri::command]
pub fn get_performance_telemetry_enabled(
    state: State<'_, Arc<dyn ConfigRepository>>,
) -> Result<bool, String> {
    let usecase = build_usecase(state.inner().clone());
    usecase
        .query()
        .get_performance_telemetry_enabled()
        .map_err(String::from)
}

#[tauri::command]
pub async fn update_crash_reporting(
    state: State<'_, Arc<dyn ConfigRepository>>,
    enabled: bool,
) -> Result<(), String> {
    let usecase = build_usecase(state.inner().clone());
    tokio::task::spawn_blocking(move || usecase.update_crash_reporting(enabled))
        .await
        .map_err(map_join_error)?
        .map_err(String::from)?;
    crate::infrastructure::telemetry::crash::set_crash_reporting_enabled(enabled);
    Ok(())
}
