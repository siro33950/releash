use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::value_objects::{AppConfigDocument, NotionRepoConfig};

pub type ConfigUpdate =
    Box<dyn FnOnce(&mut AppConfigDocument) -> Result<(), AppConfigError> + Send>;

pub trait ConfigRepository: Send + Sync {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError>;
    fn save(&self, config: AppConfigDocument) -> Result<(), AppConfigError>;
    fn update(&self, f: ConfigUpdate) -> Result<(), AppConfigError>;
}

pub trait AgentConfigRepository: Send + Sync {
    fn default_agent_backend(&self) -> Result<Option<String>, AppConfigError>;
    fn models_for_backend(&self, backend_id: &str) -> Result<Vec<String>, AppConfigError>;
    fn codex_cli_path(&self) -> Result<Option<String>, AppConfigError>;
}

pub trait ConfigSecretRepository: Send + Sync {
    fn configured_secret_values(&self) -> Result<Vec<String>, AppConfigError>;
}

pub trait NotionConfigRepository: Send + Sync {
    fn get(&self, repo_path: &str) -> Result<Option<NotionRepoConfig>, AppConfigError>;
    fn upsert(&self, repo_path: String, config: NotionRepoConfig) -> Result<(), AppConfigError>;
    fn remove(&self, repo_path: &str) -> Result<(), AppConfigError>;
}
