use super::{
    ProviderKind, ProviderLifecycleInputError, ProviderLifecycleScope, ProviderLifecycleSignal,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPayloadInterpretation {
    Signal(ProviderLifecycleSignal),
    Subagent,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPayloadError {
    InvalidPayload,
    UnsupportedEvent(String),
    InvalidSignal(ProviderLifecycleInputError),
}
impl std::fmt::Display for ProviderPayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload => f.write_str("Provider lifecycle payload is invalid"),
            Self::UnsupportedEvent(event) => {
                write!(f, "unsupported Provider lifecycle event: {event}")
            }
            Self::InvalidSignal(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ProviderPayloadError {}
impl From<ProviderLifecycleInputError> for ProviderPayloadError {
    fn from(error: ProviderLifecycleInputError) -> Self {
        Self::InvalidSignal(error)
    }
}
pub trait ProviderPayloadInterpreter: Send + Sync {
    fn interpret(
        &self,
        provider: ProviderKind,
        binding_id: &str,
        scope: ProviderLifecycleScope,
        payload: &[u8],
    ) -> Result<ProviderPayloadInterpretation, ProviderPayloadError>;
}
