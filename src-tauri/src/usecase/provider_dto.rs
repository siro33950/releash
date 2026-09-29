use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentSessionProviderDto {
    Claude,
    Codex,
}

impl From<crate::domain::provider_lifecycle::ProviderKind> for AgentSessionProviderDto {
    fn from(provider: crate::domain::provider_lifecycle::ProviderKind) -> Self {
        match provider {
            crate::domain::provider_lifecycle::ProviderKind::Claude => Self::Claude,
            crate::domain::provider_lifecycle::ProviderKind::Codex => Self::Codex,
        }
    }
}
