use std::sync::Arc;

use crate::domain::mcp::gateway::{AgentConfigGateway, AgentConfigGenerateResult};
use crate::domain::mcp::services::{normalize_agent_types, validate_generation_credentials};
use crate::domain::mcp::value_objects::{AgentKind, McpConfigParams};
use crate::usecase::mcp::error::UsecaseError;

pub struct McpAgentConfigUsecase<G: AgentConfigGateway> {
    gateway: Arc<G>,
}

impl<G: AgentConfigGateway> McpAgentConfigUsecase<G> {
    pub fn new(gateway: Arc<G>) -> Self {
        Self { gateway }
    }

    pub fn normalize_agent_types(&self, agent_types: Vec<String>) -> Result<Vec<String>, String> {
        normalize_agent_types(agent_types)
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub fn generate_many(
        &self,
        agent_types: Vec<String>,
        params: &McpConfigParams,
    ) -> Result<Vec<AgentConfigGenerateResult>, String> {
        let agent_types = normalize_agent_types(agent_types)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        validate_generation_credentials(!agent_types.is_empty(), params.port, &params.token)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        agent_types
            .iter()
            .map(|agent| {
                let agent = AgentKind::parse(agent)
                    .map_err(UsecaseError::from)
                    .map_err(String::from)?;
                self.gateway
                    .generate(agent, params)
                    .map_err(UsecaseError::from)
                    .map_err(String::from)
            })
            .collect()
    }

    pub fn remove_many(&self, agent_types: Vec<String>) -> Result<Vec<String>, String> {
        let agent_types = normalize_agent_types(agent_types)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        let mut removed = Vec::new();
        for agent in &agent_types {
            let agent_kind = AgentKind::parse(agent)
                .map_err(UsecaseError::from)
                .map_err(String::from)?;
            if self
                .gateway
                .remove(agent_kind)
                .map_err(UsecaseError::from)
                .map_err(String::from)?
            {
                removed.push(agent.clone());
            }
        }
        Ok(removed)
    }

    pub fn configured_agents(&self) -> Result<Vec<String>, String> {
        self.gateway
            .configured_agents()
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub fn remove(&self, agent_type: String) -> Result<bool, String> {
        let agent = AgentKind::parse(&agent_type)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        self.gateway
            .remove(agent)
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub fn generate(
        &self,
        agent_type: String,
        params: &McpConfigParams,
    ) -> Result<AgentConfigGenerateResult, String> {
        let agent = AgentKind::parse(&agent_type)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        self.gateway
            .generate(agent, params)
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }

    pub fn preview(&self, agent_type: String, params: &McpConfigParams) -> Result<String, String> {
        let agent = AgentKind::parse(&agent_type)
            .map_err(UsecaseError::from)
            .map_err(String::from)?;
        self.gateway
            .preview(agent, params)
            .map_err(UsecaseError::from)
            .map_err(String::from)
    }
}

#[cfg(test)]
mod mcp_agent_config_usecase_tests {
    use std::sync::{Arc, Mutex};

    use crate::domain::mcp::error::McpError;

    use super::*;

    struct FakeAgentConfigGateway {
        generated: Mutex<Vec<String>>,
    }

    impl AgentConfigGateway for FakeAgentConfigGateway {
        fn configured_agents(&self) -> Result<Vec<String>, McpError> {
            Ok(vec!["claude".to_string()])
        }

        fn remove(&self, _agent: AgentKind) -> Result<bool, McpError> {
            Ok(true)
        }

        fn generate(
            &self,
            agent: AgentKind,
            _params: &McpConfigParams,
        ) -> Result<AgentConfigGenerateResult, McpError> {
            self.generated
                .lock()
                .unwrap()
                .push(agent.as_str().to_string());
            Ok(AgentConfigGenerateResult {
                agent: agent.as_str().to_string(),
                file_path: "/tmp/config".to_string(),
                content: "content".to_string(),
            })
        }

        fn preview(&self, agent: AgentKind, _params: &McpConfigParams) -> Result<String, McpError> {
            Ok(agent.as_str().to_string())
        }
    }

    #[test]
    fn test_エージェント設定生成_重複種別は一度だけ生成する() {
        // Given
        let gateway = Arc::new(FakeAgentConfigGateway {
            generated: Mutex::new(vec![]),
        });
        let usecase = McpAgentConfigUsecase::new(gateway.clone());
        let params = McpConfigParams {
            port: 19801,
            token: "token".to_string(),
        };

        // When
        let results = usecase
            .generate_many(
                vec![
                    "claude".to_string(),
                    " Claude ".to_string(),
                    "codex".to_string(),
                ],
                &params,
            )
            .unwrap();

        // Then
        assert_eq!(results.len(), 2);
        assert_eq!(
            *gateway.generated.lock().unwrap(),
            vec!["claude".to_string(), "codex".to_string()]
        );
    }
}
