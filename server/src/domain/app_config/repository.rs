use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::{AppConfig, RepositoryConfig};

/// アプリ全体の設定を取得・保存する Repository。
pub trait AppConfigRepository: Send + Sync {
    /// 保存されているアプリ全体の設定を、その設定の版とともに取得する。
    fn load(&self) -> Result<AppConfig, AppConfigError>;
    /// アプリ全体の設定を保存する。失敗時は保存済みの設定と版を保つ。
    fn save(&self, config: AppConfig) -> Result<(), AppConfigError>;
}

pub trait ConfigSecretRepository: Send + Sync {
    fn configured_secret_values(&self) -> Result<Vec<String>, AppConfigError>;
}

/// repository ごとの設定を取得・保存する Repository。
pub trait RepositoryConfigRepository: Send + Sync {
    /// 指定した repository の設定を取得する。
    fn load(&self, repo_path: &str) -> Result<RepositoryConfig, AppConfigError>;
    /// Entity が属する repository の設定を保存する。失敗時は保存済みの値を保つ。
    fn save(&self, config: RepositoryConfig) -> Result<(), AppConfigError>;
}
