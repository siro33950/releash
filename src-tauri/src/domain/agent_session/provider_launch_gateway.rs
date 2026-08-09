#[cfg(test)]
use std::path::Path;

use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderLifecycleUnavailableReason,
};
use crate::domain::terminal_surface::TerminalProcessLaunch;

use super::ProviderSessionLaunch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedProviderLaunch {
    process: TerminalProcessLaunch,
    resource_directory: Option<std::path::PathBuf>,
    initial_hook_warning: Option<ProviderLifecycleUnavailableReason>,
}

impl PreparedProviderLaunch {
    pub(crate) fn new(
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

    #[cfg(test)]
    pub(crate) fn process(&self) -> &TerminalProcessLaunch {
        &self.process
    }

    pub(crate) fn into_process(self) -> TerminalProcessLaunch {
        self.process
    }

    pub(crate) fn initial_hook_warning(&self) -> Option<ProviderLifecycleUnavailableReason> {
        self.initial_hook_warning
    }

    #[cfg(test)]
    pub(crate) fn resource_directory(&self) -> Option<&Path> {
        self.resource_directory.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderAgentLaunchGatewayError {
    InvalidInput,
    Unavailable,
}

pub(crate) trait ProviderAgentLaunchGateway: Send + Sync {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError>;

    fn cleanup(&self, agent_session_id: &str) -> Result<(), ProviderAgentLaunchGatewayError>;
}
