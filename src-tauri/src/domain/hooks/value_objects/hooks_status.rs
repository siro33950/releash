#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksStatus {
    Active,
    NotConfigured,
    TokenMismatch,
}

impl HooksStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::NotConfigured => "not_configured",
            Self::TokenMismatch => "token_mismatch",
        }
    }
}
