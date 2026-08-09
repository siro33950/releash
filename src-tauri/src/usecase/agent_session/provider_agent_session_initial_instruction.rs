use std::sync::Arc;

use crate::domain::agent_session::aggregates::AgentSessionInitialInstructionOutcome;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};

use super::{ProviderAgentSessionUsecase, ProviderAgentSessionUsecaseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentInitialInstructionOutcome {
    Delivered,
    DeliveryUnknown,
    AlreadyDispatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentInitialInstructionError {
    InvalidInput,
    NotFound,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

pub(crate) struct ProviderAgentInitialInstructionUsecase {
    sessions: Arc<ProviderAgentSessionUsecase>,
    terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
}

impl ProviderAgentInitialInstructionUsecase {
    pub(crate) fn new(
        sessions: Arc<ProviderAgentSessionUsecase>,
        terminal: Arc<dyn ProviderAgentTerminalInputGateway>,
    ) -> Self {
        Self { sessions, terminal }
    }

    pub(crate) async fn dispatch(
        &self,
        agent_session_id: &str,
        instruction: &str,
        caller_request_id: &str,
    ) -> Result<ProviderAgentInitialInstructionOutcome, ProviderAgentInitialInstructionError> {
        if instruction.trim().is_empty() || caller_request_id.trim().is_empty() {
            return Err(ProviderAgentInitialInstructionError::InvalidInput);
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
            return Ok(ProviderAgentInitialInstructionOutcome::AlreadyDispatched);
        }
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(ProviderAgentInitialInstructionError::NotFound)?;
        let instruction = instruction.trim_end_matches(['\r', '\n']);
        let submitted_instruction = format!("\u{1b}[200~{instruction}\u{1b}[201~\r");
        match self.terminal.write(
            &session.session().terminal_surface_owner(),
            &submitted_instruction,
        ) {
            Ok(()) => Ok(ProviderAgentInitialInstructionOutcome::Delivered),
            Err(ProviderAgentTerminalGatewayError::Unavailable) => {
                Ok(ProviderAgentInitialInstructionOutcome::DeliveryUnknown)
            }
        }
    }
}

fn map_session_error(
    error: ProviderAgentSessionUsecaseError,
) -> ProviderAgentInitialInstructionError {
    match error {
        ProviderAgentSessionUsecaseError::NotFound => {
            ProviderAgentInitialInstructionError::NotFound
        }
        ProviderAgentSessionUsecaseError::InvalidOperation => {
            ProviderAgentInitialInstructionError::InvalidInput
        }
        ProviderAgentSessionUsecaseError::Conflict
        | ProviderAgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            ProviderAgentInitialInstructionError::Conflict
        }
        ProviderAgentSessionUsecaseError::Unavailable => {
            ProviderAgentInitialInstructionError::StorageUnavailable
        }
        ProviderAgentSessionUsecaseError::Corrupt => ProviderAgentInitialInstructionError::Corrupt,
    }
}
