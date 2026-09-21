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

    /// delegate child の結果を親 session へ続行指示として送る。
    /// session 側が `caller_request_id` ごとに受理を永続化してから terminal に書くため、
    /// 同じ識別子の再送は書かずに `AlreadyDispatched` を返す。
    pub(crate) async fn dispatch_continuation(
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
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(AgentSessionInitialInstructionError::NotFound)?;
        if !session.session().can_receive_workflow_instruction() {
            return Err(AgentSessionInitialInstructionError::InvalidInput);
        }
        let admission = self
            .sessions
            .admit_continuation(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error)?;
        if admission == AgentSessionInitialInstructionOutcome::AlreadyAdmitted {
            return Ok(AgentSessionInitialInstructionDeliveryOutcome::AlreadyDispatched);
        }
        match self.write_instruction(&session.session().terminal_surface_owner(), instruction) {
            Ok(()) => Ok(AgentSessionInitialInstructionDeliveryOutcome::Delivered),
            Err(ProviderAgentTerminalGatewayError::Unavailable) => {
                Ok(AgentSessionInitialInstructionDeliveryOutcome::DeliveryUnknown)
            }
        }
    }

    fn write_instruction(
        &self,
        owner: &crate::domain::terminal_surface::TerminalSurfaceOwner,
        instruction: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        let instruction = instruction.trim_end_matches(['\r', '\n']);
        self.terminal
            .write(owner, &format!("\u{1b}[200~{instruction}\u{1b}[201~\r"))
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
