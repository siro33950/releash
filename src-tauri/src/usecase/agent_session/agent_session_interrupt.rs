use std::sync::Arc;

use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};

use super::{AgentSessionUsecase, AgentSessionUsecaseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionInterruptError {
    NotFound,
    StorageUnavailable,
    TerminalUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionInterruptUsecase {
    sessions: Arc<AgentSessionUsecase>,
    terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
}

impl AgentSessionInterruptUsecase {
    pub(crate) fn new(
        sessions: Arc<AgentSessionUsecase>,
        terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
    ) -> Self {
        Self { sessions, terminal }
    }

    pub(crate) async fn interrupt(
        &self,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionInterruptError> {
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(AgentSessionInterruptError::NotFound)?;
        self.terminal
            .write(&session.session().terminal_surface_owner(), "\u{3}")
            .map_err(|ProviderAgentTerminalGatewayError::Unavailable| {
                AgentSessionInterruptError::TerminalUnavailable
            })
    }
}

fn map_session_error(error: AgentSessionUsecaseError) -> AgentSessionInterruptError {
    match error {
        AgentSessionUsecaseError::NotFound => AgentSessionInterruptError::NotFound,
        AgentSessionUsecaseError::Unavailable => AgentSessionInterruptError::StorageUnavailable,
        AgentSessionUsecaseError::Corrupt => AgentSessionInterruptError::Corrupt,
        AgentSessionUsecaseError::InvalidOperation
        | AgentSessionUsecaseError::Conflict
        | AgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            AgentSessionInterruptError::Corrupt
        }
    }
}
