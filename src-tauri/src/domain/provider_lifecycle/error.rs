#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleInputError {
    Empty(&'static str),
    InvalidAttempt,
}

impl std::fmt::Display for ProviderLifecycleInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty(field) => write!(formatter, "{field} must not be empty"),
            Self::InvalidAttempt => write!(formatter, "attempt must be greater than zero"),
        }
    }
}

impl std::error::Error for ProviderLifecycleInputError {}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleReplayError {
    EmptyHistory,
    FirstEventNotBindingArmed,
    DuplicateBindingArmed,
    BindingMismatch,
    InvalidTransition,
}

#[cfg(test)]
impl std::fmt::Display for ProviderLifecycleReplayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyHistory => formatter.write_str("provider lifecycle history is empty"),
            Self::FirstEventNotBindingArmed => {
                formatter.write_str("provider lifecycle history does not start with binding_armed")
            }
            Self::DuplicateBindingArmed => {
                formatter.write_str("provider lifecycle binding_armed is duplicated")
            }
            Self::BindingMismatch => {
                formatter.write_str("provider lifecycle event binding_id does not match")
            }
            Self::InvalidTransition => {
                formatter.write_str("provider lifecycle history contains an invalid transition")
            }
        }
    }
}

#[cfg(test)]
impl std::error::Error for ProviderLifecycleReplayError {}
