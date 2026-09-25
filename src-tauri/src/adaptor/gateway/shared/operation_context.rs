use crate::common::operation_context::OperationStopped;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
impl ClassifiedFailure for OperationStopped {
    fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Expired => FailureKind::Expired,
            Self::Cancelled => FailureKind::Cancelled,
        }
    }
}

#[cfg(test)]
#[test]
fn test_停止理由_既存の分類へ変換する() {
    assert_eq!(
        OperationStopped::Cancelled.failure_kind(),
        FailureKind::Cancelled
    );
    assert_eq!(
        OperationStopped::Expired.failure_kind(),
        FailureKind::Expired
    );
}

impl From<OperationStopped> for crate::domain::failure::TechnicalFailure {
    fn from(error: OperationStopped) -> Self {
        Self {
            kind: error.failure_kind(),
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
