use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::adaptor::gateway::provider_lifecycle::{ProviderLaunchContext, ProviderLaunchSpec};
use crate::domain::agent_session::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{ArmedProviderLifecycle, ProviderKind};

pub(crate) struct LocalProviderAgentLaunchGateway {
    data_dir: PathBuf,
    root: PathBuf,
    claude_executable: String,
    codex_executable: String,
    hook_cli_alias: String,
}

impl LocalProviderAgentLaunchGateway {
    pub(crate) fn new(
        data_dir: PathBuf,
        claude_executable: String,
        codex_executable: String,
        hook_cli_alias: String,
    ) -> Self {
        Self {
            root: data_dir.join("provider-launches"),
            data_dir,
            claude_executable,
            codex_executable,
            hook_cli_alias,
        }
    }

    fn session_directory(&self, agent_session_id: &str) -> PathBuf {
        self.root.join(digest(agent_session_id))
    }
}

impl ProviderAgentLaunchGateway for LocalProviderAgentLaunchGateway {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError> {
        let session_directory = self.session_directory(armed.scope().agent_session_id());
        let resource_directory = session_directory.join(digest(armed.binding_id()));
        let context = ProviderLaunchContext::new(
            armed.slot_id().clone(),
            armed.binding_id(),
            armed.capability(),
            armed.scope().clone(),
        )
        .map_err(|_| ProviderAgentLaunchGatewayError::InvalidInput)?;
        let spec = ProviderLaunchSpec::for_provider(
            armed.provider(),
            context,
            &self.hook_cli_alias,
            (armed.provider() == ProviderKind::Claude).then_some(resource_directory.as_path()),
        )
        .map_err(|_| ProviderAgentLaunchGatewayError::InvalidInput)?;
        let initial_hook_warning = spec.requires_hook_trust().then_some(
            crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        );
        let files = spec
            .files()
            .iter()
            .map(|file| (file.relative_path().to_path_buf(), file.contents().to_vec()))
            .collect::<Vec<_>>();
        crate::infrastructure::provider_lifecycle::materialize_launch_files(
            &resource_directory,
            &files,
        )
        .map_err(|_| ProviderAgentLaunchGatewayError::Unavailable)?;
        let executable = match armed.provider() {
            ProviderKind::Claude => &self.claude_executable,
            ProviderKind::Codex => &self.codex_executable,
        };
        let process = spec
            .terminal_process(executable, launch)
            .map_err(|_| ProviderAgentLaunchGatewayError::InvalidInput)?;
        let mut environment = process.environment().to_vec();
        environment.push((
            "RELEASH_SESSION_ID".to_string(),
            armed.scope().agent_session_id().to_string(),
        ));
        environment.push((
            "RELEASH_DATA_DIR".to_string(),
            self.data_dir.to_string_lossy().into_owned(),
        ));
        environment.push((
            "RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE".to_string(),
            resource_directory
                .join("hook-health.json")
                .to_string_lossy()
                .into_owned(),
        ));
        let process = crate::domain::terminal_surface::TerminalProcessLaunch::new(
            process.executable(),
            process.arguments().to_vec(),
            environment,
        )
        .map_err(|_| ProviderAgentLaunchGatewayError::InvalidInput)?;
        Ok(PreparedProviderLaunch::new(
            process,
            Some(resource_directory),
            initial_hook_warning,
        ))
    }

    fn cleanup(&self, agent_session_id: &str) -> Result<(), ProviderAgentLaunchGatewayError> {
        if agent_session_id.trim().is_empty() {
            return Err(ProviderAgentLaunchGatewayError::InvalidInput);
        }
        crate::infrastructure::provider_lifecycle::cleanup_launch_files(
            &self.session_directory(agent_session_id),
        )
        .map_err(|_| ProviderAgentLaunchGatewayError::Unavailable)
    }
}

fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
