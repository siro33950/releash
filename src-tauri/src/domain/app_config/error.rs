#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppConfigError {
    Repository(String),
    InvalidInput(String),
}

impl std::fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(msg) | Self::InvalidInput(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for AppConfigError {}
