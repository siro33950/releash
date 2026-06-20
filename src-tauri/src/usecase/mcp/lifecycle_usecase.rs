use std::sync::Arc;

use crate::domain::mcp::gateway::McpServerGateway;
use crate::domain::mcp::value_objects::{McpConnectionInfo, McpServerStatus};
use crate::usecase::mcp::error::UsecaseError;

pub struct McpLifecycleUsecase<G: McpServerGateway> {
    gateway: Arc<G>,
}

impl<G: McpServerGateway> McpLifecycleUsecase<G> {
    pub fn new(gateway: Arc<G>) -> Self {
        Self { gateway }
    }

    pub async fn start(&self, context: &G::Context) -> Result<McpConnectionInfo, String> {
        self.gateway
            .start(context)
            .await
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub async fn stop(&self, context: &G::Context) -> Result<(), String> {
        self.gateway
            .stop(context)
            .await
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub async fn restart_if_running(
        &self,
        context: &G::Context,
    ) -> Result<McpConnectionInfo, String> {
        self.gateway
            .restart_if_running(context)
            .await
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub fn status(&self, context: &G::Context) -> Result<McpServerStatus, String> {
        let running = self
            .gateway
            .is_running(context)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        let port = self
            .gateway
            .connection_info(context)
            .map_err(UsecaseError::from)
            .map_err(String::from)?
            .and_then(|info| {
                info.url
                    .trim_start_matches("http://127.0.0.1:")
                    .trim_end_matches("/mcp")
                    .parse::<u16>()
                    .ok()
            });
        Ok(McpServerStatus { running, port })
    }

    pub fn connection_info(
        &self,
        context: &G::Context,
    ) -> Result<Option<McpConnectionInfo>, String> {
        self.gateway
            .connection_info(context)
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }
}
