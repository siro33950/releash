use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::value_objects::{
    AppSettings, RemoteConfig, ServerConfig, WorkflowConfig,
};
use crate::usecase::app_config::dto::McpConfigDto;
use crate::usecase::app_config::error::UsecaseError;

pub struct AppConfigQueryService {
    repository: Arc<dyn ConfigRepository>,
}

impl AppConfigQueryService {
    pub fn new(repository: Arc<dyn ConfigRepository>) -> Self {
        Self { repository }
    }

    pub fn get_server_config(&self) -> Result<ServerConfig, UsecaseError> {
        Ok(self.repository.load()?.server)
    }

    pub fn get_mcp_config(&self) -> Result<McpConfigDto, UsecaseError> {
        let server = self.repository.load()?.server;
        Ok(McpConfigDto {
            port: server.mcp_port,
            token: server.mcp_token,
        })
    }

    pub fn get_app_settings(&self) -> Result<AppSettings, UsecaseError> {
        Ok(self.repository.load()?.app)
    }

    pub fn get_remote_config(&self) -> Result<RemoteConfig, UsecaseError> {
        Ok(self.repository.load()?.remote)
    }

    pub fn get_workflow_config(&self) -> Result<WorkflowConfig, UsecaseError> {
        Ok(self.repository.load()?.workflow)
    }

    pub fn get_crash_reporting_enabled(&self) -> Result<bool, UsecaseError> {
        Ok(self.repository.load()?.telemetry.crash_reporting)
    }
}
