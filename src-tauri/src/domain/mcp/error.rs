#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    InvalidInput(String),
    Gateway(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(msg) | Self::Gateway(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for McpError {}
