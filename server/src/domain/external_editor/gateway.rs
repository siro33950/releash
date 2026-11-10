use crate::domain::external_editor::EditorInfo;

pub trait InstalledEditorGateway: Send + Sync {
    fn scan(&self) -> Vec<EditorInfo>;
}

pub trait EditorLauncherGateway: Send + Sync {
    /// 指定されたパスを一度に外部エディタで開く。
    fn open_paths(
        &self,
        paths: &[std::path::PathBuf],
        editor: &str,
        label: &str,
    ) -> Result<(), EditorError>;
}

pub trait EditorSettingsGateway: Send + Sync {
    fn selected_editor(&self) -> Result<String, EditorError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditorError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Settings(crate::domain::app_config::AppConfigError),
    #[error("{0}")]
    Launch(String),
}
