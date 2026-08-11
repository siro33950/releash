use std::sync::Arc;

use crate::domain::agent_session::aggregates::AgentSessionInitialInstructionOutcome;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};

use super::{AgentSessionUsecase, AgentSessionUsecaseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionInitialInstructionDeliveryOutcome {
    Delivered,
    DeliveryUnknown,
    AlreadyDispatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionInitialInstructionError {
    InvalidInput,
    NotFound,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionInitialInstructionUsecase {
    sessions: Arc<AgentSessionUsecase>,
    terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
}

impl AgentSessionInitialInstructionUsecase {
    pub(crate) fn new(
        sessions: Arc<AgentSessionUsecase>,
        terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
    ) -> Self {
        Self { sessions, terminal }
    }

    pub(crate) async fn dispatch(
        &self,
        agent_session_id: &str,
        instruction: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionInitialInstructionDeliveryOutcome, AgentSessionInitialInstructionError>
    {
        if instruction.trim().is_empty() || caller_request_id.trim().is_empty() {
            return Err(AgentSessionInitialInstructionError::InvalidInput);
        }
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let admission = self
            .sessions
            .admit_initial_instruction(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error)?;
        if admission == AgentSessionInitialInstructionOutcome::AlreadyAdmitted {
            return Ok(AgentSessionInitialInstructionDeliveryOutcome::AlreadyDispatched);
        }
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(AgentSessionInitialInstructionError::NotFound)?;
        let instruction = instruction.trim_end_matches(['\r', '\n']);
        let submitted_instruction = format!("\u{1b}[200~{instruction}\u{1b}[201~\r");
        match self.terminal.write(
            &session.session().terminal_surface_owner(),
            &submitted_instruction,
        ) {
            Ok(()) => Ok(AgentSessionInitialInstructionDeliveryOutcome::Delivered),
            Err(ProviderAgentTerminalGatewayError::Unavailable) => {
                Ok(AgentSessionInitialInstructionDeliveryOutcome::DeliveryUnknown)
            }
        }
    }
}

fn map_session_error(error: AgentSessionUsecaseError) -> AgentSessionInitialInstructionError {
    match error {
        AgentSessionUsecaseError::NotFound => AgentSessionInitialInstructionError::NotFound,
        AgentSessionUsecaseError::InvalidOperation => {
            AgentSessionInitialInstructionError::InvalidInput
        }
        AgentSessionUsecaseError::Conflict
        | AgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            AgentSessionInitialInstructionError::Conflict
        }
        AgentSessionUsecaseError::Unavailable => {
            AgentSessionInitialInstructionError::StorageUnavailable
        }
        AgentSessionUsecaseError::Corrupt => AgentSessionInitialInstructionError::Corrupt,
    }
}
