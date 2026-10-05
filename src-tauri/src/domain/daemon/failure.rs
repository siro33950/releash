#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupFailureKind {
    StoreInUse,
    StorageUnavailable(crate::domain::failure::TechnicalFailureNature),
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
}

impl StartupFailureKind {
    pub(crate) fn safe_description(&self) -> &'static str {
        match self {
            Self::StoreInUse => "Local data is currently in use by another Releash process.",
            Self::StorageUnavailable(_) => "Local data storage is currently unavailable.",
            Self::UnsupportedRuntime => {
                "This Releash build cannot use the bundled local database runtime."
            }
            Self::UnsupportedStoreVersion => {
                "This Releash build does not support the local data version."
            }
            Self::InitializationStateInvalid => {
                "Local data initialization could not be verified safely."
            }
            Self::StoreValidationFailed => "The local data store could not be verified safely.",
            Self::SchemaEvolutionFailed => {
                "The local data store could not be updated safely during startup."
            }
        }
    }

    pub(crate) fn retry_on_next_launch(&self) -> bool {
        matches!(
            self,
            Self::StoreInUse
                | Self::StorageUnavailable(
                    crate::domain::failure::TechnicalFailureNature::Transient
                        | crate::domain::failure::TechnicalFailureNature::TimedOut
                )
                | Self::SchemaEvolutionFailed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupFailure {
    pub(crate) kind: StartupFailureKind,
    pub(crate) safe_description: &'static str,
    pub(crate) correlation_id: String,
    pub(crate) retry_on_next_launch: bool,
}

impl StartupFailure {
    pub(crate) fn new(kind: StartupFailureKind, correlation_id: String) -> Self {
        Self {
            kind: kind.clone(),
            safe_description: kind.safe_description(),
            correlation_id,
            retry_on_next_launch: kind.retry_on_next_launch(),
        }
    }
}
