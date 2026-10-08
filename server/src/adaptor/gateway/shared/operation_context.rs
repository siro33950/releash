use crate::common::operation_context::OperationStopped;
impl From<OperationStopped> for crate::domain::failure::TechnicalFailure {
    fn from(error: OperationStopped) -> Self {
        Self {
            nature: match error {
                OperationStopped::Expired => {
                    crate::domain::failure::TechnicalFailureNature::TimedOut
                }
                OperationStopped::Cancelled => {
                    crate::domain::failure::TechnicalFailureNature::Cancelled
                }
            },
            message: error.to_string(),
        }
    }
}
impl From<OperationStopped> for crate::domain::code::CodeError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::repository::RepositoryError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::notion::NotionError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::comment::ReviewError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::local_event::CommitBatchError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::local_event::LocalEventQueryError {
    fn from(error: OperationStopped) -> Self {
        let mut failure: crate::domain::failure::TechnicalFailure = error.into();
        if error == OperationStopped::Expired {
            failure.message = "deadline exceeded".into();
        }
        Self::Technical(failure)
    }
}
impl From<OperationStopped> for crate::domain::workflow::WorkflowError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}
impl From<OperationStopped> for crate::domain::git_host::GitHostError {
    fn from(error: OperationStopped) -> Self {
        Self::Technical(error.into())
    }
}

#[cfg(test)]
#[path = "operation_context_test.rs"]
mod operation_context_tests;

impl From<tokio::task::JoinError> for crate::domain::failure::TechnicalFailure {
    fn from(error: tokio::task::JoinError) -> Self {
        Self {
            nature: if error.is_cancelled() {
                crate::domain::failure::TechnicalFailureNature::Cancelled
            } else {
                crate::domain::failure::TechnicalFailureNature::Other
            },
            message: error.to_string(),
        }
    }
}
