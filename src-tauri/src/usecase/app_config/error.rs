use crate::domain::app_config::AppConfigError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    AppConfig(#[from] AppConfigError),
}

impl From<UsecaseError> for String {
    fn from(value: UsecaseError) -> Self {
        value.to_string()
    }
}
