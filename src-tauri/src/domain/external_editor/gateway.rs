use crate::domain::external_editor::EditorInfo;

pub trait InstalledEditorGateway: Send + Sync {
    fn scan(&self) -> Vec<EditorInfo>;
}

pub trait EditorLauncherGateway: Send + Sync {
    fn open_path(&self, path: &str, editor: &str, label: &str) -> Result<(), EditorError>;
}

pub trait EditorSettingsGateway: Send + Sync {
    fn selected_editor(&self) -> Result<String, EditorError>;
    fn update_selected_editor(&self, editor: String) -> Result<(), EditorError>;
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
impl crate::domain::failure::ClassifiedFailure for EditorError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::InvalidInput(_) => F::InvalidInput,
            Self::Settings(error) => error.failure_kind(),
            Self::Launch(_) => F::StateRequired,
        }
    }
}

#[cfg(test)]
#[path = "gateway_test.rs"]
mod gateway_tests;
