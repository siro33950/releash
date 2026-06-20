use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::adaptor::gateway::mcp::{AgentConfigGatewayImpl, McpServerGatewayImpl};
use crate::domain::app_config::ConfigRepository;
use crate::domain::mcp::services::normalize_agent_types;
use crate::domain::mcp::value_objects::{McpConfigParams, McpServerStatus};
use crate::usecase::app_config::AppConfigUsecase;
use crate::usecase::mcp::dto::{GenerateResult, McpConnectionInfoDto, McpServerStatusDto};
use crate::usecase::mcp::{McpAgentConfigUsecase, McpLifecycleUsecase, McpQueryService};

fn app_config_usecase(app_config: Arc<dyn ConfigRepository>) -> AppConfigUsecase {
    AppConfigUsecase::new(app_config)
}

fn lifecycle_usecase() -> McpLifecycleUsecase<McpServerGatewayImpl> {
    McpLifecycleUsecase::new(Arc::new(McpServerGatewayImpl))
}

fn agent_config_usecase() -> McpAgentConfigUsecase<AgentConfigGatewayImpl> {
    McpAgentConfigUsecase::new(Arc::new(AgentConfigGatewayImpl::new()))
}

fn agent_query_service() -> McpQueryService<AgentConfigGatewayImpl> {
    McpQueryService::new(Arc::new(AgentConfigGatewayImpl::new()))
}

fn map_join_error(error: tokio::task::JoinError) -> String {
    format!("task join error: {error}")
}

#[tauri::command]
pub async fn start_mcp_server(app: tauri::AppHandle) -> Result<McpConnectionInfoDto, String> {
    lifecycle_usecase().start(&app).await.map(Into::into)
}

#[tauri::command]
pub async fn stop_mcp_server(app: tauri::AppHandle) -> Result<(), String> {
    lifecycle_usecase().stop(&app).await
}

#[tauri::command]
pub fn get_mcp_server_status(app: tauri::AppHandle) -> McpServerStatusDto {
    lifecycle_usecase()
        .status(&app)
        .unwrap_or(McpServerStatus {
            running: false,
            port: None,
        })
        .into()
}

#[tauri::command]
pub fn get_mcp_connection_info(app: tauri::AppHandle) -> Option<McpConnectionInfoDto> {
    lifecycle_usecase()
        .connection_info(&app)
        .ok()
        .flatten()
        .map(Into::into)
}

#[tauri::command]
pub fn get_configured_agents() -> Vec<String> {
    agent_query_service()
        .configured_agents()
        .unwrap_or_default()
}

#[tauri::command]
pub fn remove_agent_mcp_config(agent_type: String) -> Result<bool, String> {
    agent_config_usecase().remove(agent_type)
}

#[tauri::command]
pub async fn save_and_generate_mcp_configs(
    app: tauri::AppHandle,
    state: State<'_, Arc<dyn ConfigRepository>>,
    port: u16,
    token: String,
    agent_types: Vec<String>,
    removed_agents: Vec<String>,
) -> Result<Vec<GenerateResult>, String> {
    let token = token.trim().to_string();
    let agent_usecase = agent_config_usecase();
    let agent_types = agent_usecase.normalize_agent_types(agent_types)?;
    let removed_agents = agent_usecase.normalize_agent_types(removed_agents)?;

    let config_usecase = app_config_usecase(state.inner().clone());
    tokio::task::spawn_blocking(move || config_usecase.update_mcp_config(port, token))
        .await
        .map_err(map_join_error)?
        .map_err(String::from)?;

    lifecycle_usecase().restart_if_running(&app).await?;

    if !removed_agents.is_empty() {
        agent_usecase.remove_many(removed_agents)?;
    }

    if agent_types.is_empty() {
        return Ok(vec![]);
    }

    let config = state.load().map_err(|e| e.to_string())?;
    let params = McpConfigParams {
        port: config.server.mcp_port,
        token: config.server.mcp_token.clone(),
    };
    tokio::task::spawn_blocking(move || agent_config_usecase().generate_many(agent_types, &params))
        .await
        .map_err(map_join_error)?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSelectionResult {
    pub agent_types: Vec<String>,
    pub removed_agents: Vec<String>,
    pub generated: Vec<GenerateResult>,
}

#[tauri::command]
pub async fn save_mcp_agent_selection(
    app: tauri::AppHandle,
    state: State<'_, Arc<dyn ConfigRepository>>,
    agent_types: Vec<String>,
) -> Result<AgentSelectionResult, String> {
    let desired_agents = normalize_agent_types(agent_types).map_err(|e| e.to_string())?;
    let current_agents = agent_query_service().configured_agents()?;
    let removed_agents: Vec<String> = current_agents
        .into_iter()
        .filter(|agent| !desired_agents.contains(agent))
        .collect();
    let config = state.load().map_err(|e| e.to_string())?;
    let generated = save_and_generate_mcp_configs(
        app,
        state,
        config.server.mcp_port,
        config.server.mcp_token.clone(),
        desired_agents.clone(),
        removed_agents.clone(),
    )
    .await?;
    Ok(AgentSelectionResult {
        agent_types: desired_agents,
        removed_agents,
        generated,
    })
}

#[tauri::command]
pub async fn generate_agent_mcp_config(
    agent_type: String,
    state: State<'_, Arc<dyn ConfigRepository>>,
    port: Option<u16>,
    token: Option<String>,
) -> Result<GenerateResult, String> {
    let config = state.load().map_err(|e| e.to_string())?;
    let params = McpConfigParams {
        port: port.unwrap_or(config.server.mcp_port),
        token: token.unwrap_or_else(|| config.server.mcp_token.clone()),
    };
    tokio::task::spawn_blocking(move || agent_config_usecase().generate(agent_type, &params))
        .await
        .map_err(map_join_error)?
}

#[tauri::command]
pub fn preview_agent_mcp_config(
    agent_type: String,
    state: State<'_, Arc<dyn ConfigRepository>>,
    port: Option<u16>,
    token: Option<String>,
) -> Result<String, String> {
    let config = state.load().map_err(|e| e.to_string())?;
    let params = McpConfigParams {
        port: port.unwrap_or(config.server.mcp_port),
        token: token.unwrap_or_else(|| config.server.mcp_token.clone()),
    };
    agent_query_service().preview(agent_type, &params)
}
