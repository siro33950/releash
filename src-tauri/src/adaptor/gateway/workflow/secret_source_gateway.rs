use std::sync::Arc;

use crate::adaptor::gateway::workflow::secret_source;
use crate::config::AppConfig;
use crate::domain::workflow::{secret_masker, SecretSourceGateway};

#[derive(Clone)]
pub(crate) struct WorkflowSecretSourceConfigGateway {
    config: Arc<AppConfig>,
}

impl WorkflowSecretSourceConfigGateway {
    pub(crate) fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

impl SecretSourceGateway for WorkflowSecretSourceConfigGateway {
    fn configured_secret_values(&self) -> Vec<String> {
        let mut values = self
            .config
            .get_config()
            .map(|cfg| secret_source::collect_configured_secret_values_from_config(&cfg))
            .unwrap_or_default();
        values.extend(secret_masker::collect_secret_values_from_env_vars(
            std::env::vars(),
        ));
        secret_masker::normalize_secret_values(values)
    }
}

#[cfg(test)]
pub(crate) struct EmptySecretSourceGateway;

#[cfg(test)]
impl SecretSourceGateway for EmptySecretSourceGateway {
    fn configured_secret_values(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReleashConfig;
    use tempfile::TempDir;

    #[test]
    fn collects_and_normalizes_configured_secret_values() {
        let tmp = TempDir::new().unwrap();
        let mut config = ReleashConfig::default();
        config.server.token = "token-12345678".to_string();
        config.server.mcp_token = "token-12345678".to_string();
        config.server.notify.webhook_url = "https://hooks.example/secret-abcdef".to_string();
        let app_config = Arc::new(AppConfig::new(config, tmp.path().join("config.toml")));

        let secrets = WorkflowSecretSourceConfigGateway::new(app_config).configured_secret_values();

        assert!(secrets.contains(&"token-12345678".to_string()));
        assert!(secrets.contains(&"https://hooks.example/secret-abcdef".to_string()));
        assert_eq!(
            secrets
                .iter()
                .filter(|secret| secret.as_str() == "token-12345678")
                .count(),
            1
        );
    }
}
