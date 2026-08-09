use std::ffi::OsString;

use crate::domain::agent_session::ProviderAvailabilityGateway;
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) struct LocalProviderAvailabilityGateway {
    claude_executable: String,
    codex_executable: String,
    search_path: Option<OsString>,
}

impl LocalProviderAvailabilityGateway {
    pub(crate) fn new(claude_executable: String, codex_executable: String) -> Self {
        Self {
            claude_executable,
            codex_executable,
            search_path: std::env::var_os("PATH"),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_search_path(
        claude_executable: String,
        codex_executable: String,
        search_path: Option<OsString>,
    ) -> Self {
        Self {
            claude_executable,
            codex_executable,
            search_path,
        }
    }
}

impl ProviderAvailabilityGateway for LocalProviderAvailabilityGateway {
    fn is_available(&self, provider: ProviderKind) -> bool {
        let executable = match provider {
            ProviderKind::Claude => &self.claude_executable,
            ProviderKind::Codex => &self.codex_executable,
        };
        crate::infrastructure::process::executable_probe::is_executable(
            executable,
            self.search_path.as_deref(),
        )
    }
}
