use crate::domain::mcp::error::McpError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error(transparent)]
    Mcp(#[from] McpError),
}

impl From<UsecaseError> for String {
    fn from(value: UsecaseError) -> Self {
        value.to_string()
    }
}
