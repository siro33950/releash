#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupFailureKind {
    StoreInUse,
    StorageUnavailable(crate::domain::failure::TechnicalFailureNature),
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
    /// 旧保存先から新保存先への移行を行わなかった。理由は `RelocationBlock` が持つ。
    DataRelocationBlocked(super::data_home::RelocationBlock),
    /// 旧保存先から新保存先への移行の途中で、観測か移動に失敗した。
    DataRelocationFailed(crate::domain::failure::TechnicalFailureNature),
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
            Self::DataRelocationBlocked(super::data_home::RelocationBlock::LegacyServerRunning) => {
                "Another Releash server is still running on the previous data folder. Stop it, then start again."
            }
            Self::DataRelocationBlocked(
                super::data_home::RelocationBlock::LegacyServerUnverifiable,
            ) => {
                "The previous data folder may still be in use. Make sure no Releash server is running, then start again."
            }
            Self::DataRelocationBlocked(super::data_home::RelocationBlock::BothHaveData) => {
                "Both the previous and the new data folders contain Releash data. Keep one of them, then start again."
            }
            Self::DataRelocationFailed(_) => {
                "Releash data could not be moved to the new data folder safely."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    pub(crate) kind: StartupFailureKind,
    pub(crate) safe_description: &'static str,
    pub(crate) correlation_id: String,
}

impl StartupFailure {
    pub(crate) fn new(kind: StartupFailureKind, correlation_id: String) -> Self {
        Self {
            kind: kind.clone(),
            safe_description: kind.safe_description(),
            correlation_id,
        }
    }
}
