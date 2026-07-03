use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::agent_session::gateway::ForkSessionRequest;
use crate::domain::agent_session::PermissionMode;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{
    AgentSessionBackendLifecycleGateway, AgentSessionRuntimeCloser, BackendSessionLifecycleRequest,
};

pub(crate) struct RegistryAgentSessionBackendLifecycleGateway {
    registry: Arc<AgentBackendRegistry>,
}

impl RegistryAgentSessionBackendLifecycleGateway {
    pub(crate) fn new(registry: Arc<AgentBackendRegistry>) -> Self {
        Self { registry }
    }

    fn resolve_backend(
        &self,
        request: &BackendSessionLifecycleRequest,
    ) -> Result<Arc<dyn crate::domain::agent_session::gateway::AgentBackend>, String> {
        let backend_id = request
            .backend_id
            .as_deref()
            .ok_or_else(|| "Session is missing backend id".to_string())?;
        self.registry
            .get(backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))
    }

    fn permission_mode(request: &BackendSessionLifecycleRequest) -> Result<PermissionMode, String> {
        PermissionMode::parse(&request.permission_mode).map_err(|e| e.to_string())
    }
}

#[async_trait]
impl AgentSessionBackendLifecycleGateway for RegistryAgentSessionBackendLifecycleGateway {
    async fn archive_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<(), String> {
        let Some(agent_session_id) = request.agent_session_id.as_deref() else {
            return Ok(());
        };
        self.resolve_backend(&request)?
            .archive_session(agent_session_id, &request.cwd)
            .await
            .map_err(|e| e.to_string())
    }

    async fn unarchive_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<(), String> {
        let Some(agent_session_id) = request.agent_session_id.as_deref() else {
            return Ok(());
        };
        self.resolve_backend(&request)?
            .unarchive_session(agent_session_id, &request.cwd)
            .await
            .map_err(|e| e.to_string())
    }

    async fn fork_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<Option<String>, String> {
        let Some(agent_session_id) = request.agent_session_id.clone() else {
            return Ok(None);
        };
        let permission_mode = Self::permission_mode(&request)?;
        self.resolve_backend(&request)?
            .fork_session(ForkSessionRequest {
                backend_session_id: agent_session_id,
                cwd: request.cwd,
                model: request.model,
                permission_mode,
                plan_mode: request.plan_mode,
                permission_profile_id: request.permission_profile_id,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

pub(crate) struct RuntimeAgentSessionCloser {
    runtime: Arc<AgentSessionRuntimeUsecase>,
}

impl RuntimeAgentSessionCloser {
    pub(crate) fn new(runtime: Arc<AgentSessionRuntimeUsecase>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl AgentSessionRuntimeCloser for RuntimeAgentSessionCloser {
    async fn close_agent_session(&self, session_id: &str) -> Result<(), String> {
        self.runtime
            .close_session(session_id)
            .await
            .map_err(|e| e.to_string())
    }
}
