use std::sync::Arc;

use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::TerminalSurfaceOwner;

use super::{AgentSessionLifecycleUsecase, AgentSessionLifecycleUsecaseError};

#[async_trait::async_trait]
pub(crate) trait AgentSessionExitPort: Send + Sync {
    async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        runtime_generation: u64,
        exit_code: Option<i32>,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError>;
}

pub(crate) struct AgentSessionExitUsecase {
    terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
    sessions: Arc<dyn AgentSessionExitPort>,
}

impl AgentSessionExitUsecase {
    pub(crate) fn new(
        terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
        sessions: Arc<dyn AgentSessionExitPort>,
    ) -> Self {
        Self { terminal, sessions }
    }

    pub(crate) async fn observe_exit(
        &self,
        session_key: &str,
        runtime_generation: u64,
        caller_request_id: &str,
    ) -> Result<bool, AgentSessionLifecycleUsecaseError> {
        let Some(owner) = self
            .terminal
            .owner_for_runtime_generation(session_key, runtime_generation)
        else {
            return Ok(false);
        };
        let TerminalSurfaceOwner::Session { ref session_id, .. } = owner else {
            return Ok(false);
        };
        let session_id = session_id.clone();
        let exit_code = self.terminal.session_exit_code(&owner);
        self.sessions
            .observe_process_exit(
                &session_id,
                runtime_generation,
                exit_code,
                caller_request_id,
            )
            .await?;
        Ok(true)
    }

    pub(crate) async fn reconcile_exited(
        &self,
        caller_request_id: &str,
    ) -> Result<usize, AgentSessionLifecycleUsecaseError> {
        let mut observed = 0;
        for (runtime_generation, owner, exit_code) in self.terminal.exited_session_owners() {
            let TerminalSurfaceOwner::Session { session_id, .. } = owner else {
                continue;
            };
            self.sessions
                .observe_process_exit(
                    &session_id,
                    runtime_generation,
                    exit_code,
                    &format!("{caller_request_id}.{session_id}"),
                )
                .await?;
            observed += 1;
        }
        Ok(observed)
    }
}

#[async_trait::async_trait]
impl AgentSessionExitPort for AgentSessionLifecycleUsecase {
    async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        runtime_generation: u64,
        exit_code: Option<i32>,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        AgentSessionLifecycleUsecase::observe_process_exit(
            self,
            agent_session_id,
            runtime_generation,
            exit_code,
            caller_request_id,
        )
        .await
    }
}
