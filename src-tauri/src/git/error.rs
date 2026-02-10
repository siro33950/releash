use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{0}")]
    Git2(#[from] git2::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("{0}")]
    StripPrefix(#[from] std::path::StripPrefixError),
    #[error("{0}")]
    Custom(String),
}

impl Serialize for GitError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
