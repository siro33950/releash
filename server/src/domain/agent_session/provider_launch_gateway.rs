#[cfg(any(test, feature = "test-support"))]
use std::path::Path;

use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderLifecycleUnavailableReason,
};
use crate::domain::terminal_surface::TerminalProcessLaunch;

use super::aggregates::ResolvedProviderExecutable;
use super::ProviderSessionLaunch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedProviderLaunch {
    process: TerminalProcessLaunch,
    resource_directory: Option<std::path::PathBuf>,
    initial_hook_warning: Option<ProviderLifecycleUnavailableReason>,
}

impl PreparedProviderLaunch {
    pub fn new(
        process: TerminalProcessLaunch,
        resource_directory: Option<std::path::PathBuf>,
        initial_hook_warning: Option<ProviderLifecycleUnavailableReason>,
    ) -> Self {
        Self {
            process,
            resource_directory,
            initial_hook_warning,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn process(&self) -> &TerminalProcessLaunch {
        &self.process
    }

    pub(crate) fn into_process(self) -> TerminalProcessLaunch {
        self.process
    }

    pub fn initial_hook_warning(&self) -> Option<ProviderLifecycleUnavailableReason> {
        self.initial_hook_warning
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn resource_directory(&self) -> Option<&Path> {
        self.resource_directory.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAgentLaunchGatewayError {
    Technical(crate::domain::failure::TechnicalFailure),
    InvalidInput,
}

pub trait ProviderAgentLaunchGateway: Send + Sync {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        executable: ResolvedProviderExecutable,
        launch: ProviderSessionLaunch,
        worktree_path: &str,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError>;

    fn cleanup(&self, agent_session_id: &str) -> Result<(), ProviderAgentLaunchGatewayError>;
}
