use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::agent_session::{
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalInputGateway, ProviderAgentTerminalObservationGateway,
};
use crate::domain::terminal_surface::{
    TerminalActivity, TerminalProcessLaunch, TerminalSurfaceOwner,
};
use crate::usecase::terminal_surface::application::{
    OwnedTerminalSummaryLookup, TerminalSurfaceApplication,
};

impl ProviderAgentTerminalGateway for TerminalSurfaceApplication {
    fn spawn(
        &self,
        owner: TerminalSurfaceOwner,
        worktree_path: &str,
        process: TerminalProcessLaunch,
        rows: u16,
        cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.get_or_spawn_process(
            rows,
            cols,
            Some(worktree_path.to_string()),
            owner,
            None,
            process,
        )
        .map(|_| ())
        .map_err(|_| ProviderAgentTerminalGatewayError::Unavailable)
    }

    fn presence(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError> {
        Ok(match self.find_owned_summary(owner) {
            OwnedTerminalSummaryLookup::Absent => ManagedPtyPresence::ConfirmedAbsent,
            OwnedTerminalSummaryLookup::OwnerMismatch => ManagedPtyPresence::Unknown,
            OwnedTerminalSummaryLookup::Found(summary) => {
                if summary.process_state.is_exited() {
                    ManagedPtyPresence::ConfirmedAbsent
                } else {
                    ManagedPtyPresence::Live
                }
            }
        })
    }

    fn stop_preserving_checkpoint(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.stop_preserving_checkpoint(owner)
            .map_err(|_| ProviderAgentTerminalGatewayError::Unavailable)
    }

    fn delete(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.delete_surface(owner)
            .map_err(|_| ProviderAgentTerminalGatewayError::Unavailable)
    }

    fn is_current_runtime_generation(
        &self,
        owner: &TerminalSurfaceOwner,
        runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        Ok(matches!(
            self.find_owned_summary(owner),
            OwnedTerminalSummaryLookup::Found(summary)
                if summary.runtime_generation.value() == runtime_generation
        ))
    }
}

impl ProviderAgentTerminalInputGateway for TerminalSurfaceApplication {
    fn write(
        &self,
        owner: &TerminalSurfaceOwner,
        input: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.write(owner, input)
            .map_err(|_| ProviderAgentTerminalGatewayError::Unavailable)
    }
}

impl ProviderAgentTerminalObservationGateway for TerminalSurfaceApplication {
    fn owner_for_runtime_generation(
        &self,
        session_key: &str,
        runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        self.summaries()
            .into_iter()
            .find(|surface| {
                surface.session_key == session_key
                    && surface.runtime_generation.value() == runtime_generation
            })
            .map(|surface| surface.owner)
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        self.summaries()
            .into_iter()
            .filter(|surface| surface.process_state.is_exited())
            .map(|surface| {
                (
                    surface.runtime_generation.value(),
                    surface.owner,
                    surface.process_state.exit_code(),
                )
            })
            .collect()
    }

    fn session_exit_code(&self, owner: &TerminalSurfaceOwner) -> Option<i32> {
        match self.find_owned_summary(owner) {
            OwnedTerminalSummaryLookup::Found(summary) => summary.process_state.exit_code(),
            OwnedTerminalSummaryLookup::Absent | OwnedTerminalSummaryLookup::OwnerMismatch => None,
        }
    }

    fn session_activity(&self, owner: &TerminalSurfaceOwner) -> TerminalActivity {
        match self.find_owned_summary(owner) {
            OwnedTerminalSummaryLookup::Found(summary) if !summary.process_state.is_exited() => {
                TerminalActivity::classify(summary.last_output_at.map(|at| at.elapsed()))
            }
            _ => TerminalActivity::Idle,
        }
    }

    fn session_worktree_path(&self, session_key: &str) -> Option<String> {
        self.summaries().into_iter().find_map(|surface| {
            (surface.session_key == session_key
                && matches!(surface.owner, TerminalSurfaceOwner::Session { .. }))
            .then_some(surface.worktree_path)
            .flatten()
        })
    }
}

#[cfg(test)]
#[path = "provider_agent_terminal_gateway_test.rs"]
mod provider_agent_terminal_gateway_tests;
