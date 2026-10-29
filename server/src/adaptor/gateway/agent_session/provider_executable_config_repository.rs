use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::agent_session::aggregates::ProviderExecutable;
use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
use crate::domain::app_config::{AppConfig, AppConfigError, AppConfigRepository};
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) struct InMemoryProviderExecutableConfigRepository {
    config: Mutex<AppConfig>,
}

impl InMemoryProviderExecutableConfigRepository {
    pub(crate) fn new(
        claude: Option<String>,
        codex: Option<String>,
    ) -> Result<Self, AppConfigError> {
        let mut overrides = HashMap::new();
        for (provider, executable) in [(ProviderKind::Claude, claude), (ProviderKind::Codex, codex)]
        {
            if let Some(executable) = executable {
                overrides.insert(
                    provider,
                    ProviderExecutable::new(executable).map_err(|_| {
                        AppConfigError::InvalidInput("Invalid provider executable".into())
                    })?,
                );
            }
        }
        Ok(Self {
            config: Mutex::new(AppConfig::new(
                false,
                false,
                vec![],
                String::new(),
                String::new(),
                TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                WorkflowConfig {
                    approval_auto_approve: false,
                },
                overrides,
                HashMap::new(),
            )),
        })
    }
}

impl AppConfigRepository for InMemoryProviderExecutableConfigRepository {
    fn load(&self) -> Result<AppConfig, AppConfigError> {
        self.config
            .lock()
            .map(|config| config.clone())
            .map_err(|error| {
                AppConfigError::Technical(crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                })
            })
    }

    fn save(&self, config: AppConfig) -> Result<(), AppConfigError> {
        let mut current = self.config.lock().map_err(|error| {
            AppConfigError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: error.to_string(),
            })
        })?;
        *current = config;
        Ok(())
    }
}
