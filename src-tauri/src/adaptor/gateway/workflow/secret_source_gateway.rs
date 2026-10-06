use std::sync::Arc;

use crate::domain::app_config::ConfigSecretRepository;
use crate::domain::workflow::{secret_masker, SecretSourceGateway, WorkflowError};

#[derive(Clone)]
pub struct WorkflowSecretSourceConfigGateway {
    config: Arc<dyn ConfigSecretRepository>,
}

impl WorkflowSecretSourceConfigGateway {
    pub fn new(config: Arc<dyn ConfigSecretRepository>) -> Self {
        Self { config }
    }
}

impl SecretSourceGateway for WorkflowSecretSourceConfigGateway {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
        let mut values = self
            .config
            .configured_secret_values()
            .map_err(|e| WorkflowError::External(e.to_string()))?;
        values.extend(secret_masker::collect_secret_values_from_env_vars(
            std::env::vars(),
        ));
        Ok(secret_masker::normalize_secret_values(values))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct EmptySecretSourceGateway;

#[cfg(any(test, feature = "test-support"))]
impl SecretSourceGateway for EmptySecretSourceGateway {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
        Ok(Vec::new())
    }
}
