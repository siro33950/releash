use crate::adaptor::presenter::connect::ConnectFailure;
use serde::ser::SerializeStruct;
use serde::Serialize;

/// アプリ横断のエラー型。adaptor 層（Tauri コマンドなど）の
/// 戻り値で用いる。
///
/// フロント／リモートへ返却される serialize 表現は、通常エラーでは移行前の
/// `GitError`（プレーン文字列）と等価であることを契約とする。特定の回復可能な
/// エラーだけ、表示文言とは別の機械可読 code を持つ object として返す。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("{error}")]
    Presented {
        kind: connectrpc::ErrorCode,
        cause: Option<String>,
        error: Box<AppError>,
    },
    #[error("{0}")]
    Internal(String),
    #[error("{message}")]
    Coded {
        code: String,
        message: String,
        kind: connectrpc::ErrorCode,
    },
}

impl AppError {
    pub fn from_failure(
        error: impl crate::adaptor::presenter::connect::ConnectFailure + std::fmt::Display,
    ) -> Self {
        let kind = error.connect_code();
        Self::new(error.to_string()).with_status(kind)
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(message).with_status(connectrpc::ErrorCode::InvalidArgument)
    }
    pub(crate) fn missing_target(message: impl Into<String>) -> Self {
        Self::new(message).with_status(connectrpc::ErrorCode::NotFound)
    }
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(message).with_status(connectrpc::ErrorCode::Unavailable)
    }
    pub(crate) fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(message).with_status(connectrpc::ErrorCode::FailedPrecondition)
    }
    pub(crate) fn capacity_exceeded(message: impl Into<String>) -> Self {
        Self::new(message).with_status(connectrpc::ErrorCode::ResourceExhausted)
    }
    pub(crate) fn with_code(self, code: impl Into<String>) -> Self {
        let status = self.connect_code();
        Self::coded(code, self.to_string(), status)
    }
    pub fn with_status(self, kind: connectrpc::ErrorCode) -> Self {
        Self::Presented {
            kind,
            cause: None,
            error: Box::new(self),
        }
    }

    pub(crate) fn with_cause(self, cause: Option<String>) -> Self {
        match self {
            Self::Presented { kind, error, .. } => Self::Presented { kind, cause, error },
            error @ (Self::Internal(_) | Self::Coded { .. }) => Self::Presented {
                kind: error.connect_code(),
                cause,
                error: Box::new(error),
            },
        }
    }

    pub(crate) fn cause(&self) -> Option<&str> {
        match self {
            Self::Presented { cause, error, .. } => cause.as_deref().or_else(|| error.cause()),
            _ => None,
        }
    }

    pub fn new(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn coded(
        code: impl Into<String>,
        message: impl Into<String>,
        kind: connectrpc::ErrorCode,
    ) -> Self {
        Self::Coded {
            kind,
            code: code.into(),
            message: message.into(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Presented { error, .. } => error.serialize(serializer),
            Self::Internal(message) => serializer.serialize_str(message),
            Self::Coded { code, message, .. } => {
                let mut state = serializer.serialize_struct("AppError", 2)?;
                state.serialize_field("code", code)?;
                state.serialize_field("message", message)?;
                state.end()
            }
        }
    }
}

use crate::domain::code::CodeError;
use crate::usecase::code_error::CodeUsecaseError;
const STALE_REVIEW_GROUP_TARGET_ERROR_CODE: &str = "STALE_REVIEW_GROUP_TARGET";
const STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE: &str =
    "The review changed before the operation completed. Reload the review and try again.";

/// ユースケースエラー → アプリエラーの集約変換（adaptor 層が担う）。
/// `#[error(transparent)]` な `CodeUsecaseError` の `Display` を保持するため、
/// 通常エラーの serialize 表現は移行前（`GitError` のプレーン文字列）と等価に保たれる。
/// frontend が回復判断を必要とする stale review group だけ機械可読 code を付ける。
impl From<CodeUsecaseError> for AppError {
    fn from(e: CodeUsecaseError) -> Self {
        let kind = e.connect_code();
        let message = e.to_string();
        let result = match &e {
            CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { group_id }) => {
                log::warn!(
                    "code command failed: code={} group_id={}",
                    STALE_REVIEW_GROUP_TARGET_ERROR_CODE,
                    group_id
                );
                AppError::coded(
                    STALE_REVIEW_GROUP_TARGET_ERROR_CODE,
                    STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE,
                    kind,
                )
            }
            _ => AppError::Internal(message),
        };
        result.with_status(kind)
    }
}

pub(crate) fn workflow_storage_message(failure: &crate::domain::failure::StorageFailure) -> String {
    if let Some(message) = &failure.context {
        return message.clone();
    }
    let label = match failure.connect_code() {
        connectrpc::ErrorCode::Unavailable => "Temporary",
        connectrpc::ErrorCode::Aborted => "RestartRequired",
        connectrpc::ErrorCode::FailedPrecondition => "StateRequired",
        connectrpc::ErrorCode::InvalidArgument => "InvalidInput",
        connectrpc::ErrorCode::DeadlineExceeded => "Expired",
        connectrpc::ErrorCode::NotFound => "Missing",
        connectrpc::ErrorCode::AlreadyExists => "AlreadyPresent",
        connectrpc::ErrorCode::PermissionDenied => "Permission",
        connectrpc::ErrorCode::ResourceExhausted => "Capacity",
        connectrpc::ErrorCode::Unimplemented => "Unsupported",
        connectrpc::ErrorCode::Internal => "Internal",
        connectrpc::ErrorCode::DataLoss => "Corrupt",
        connectrpc::ErrorCode::Canceled => "Cancelled",
        connectrpc::ErrorCode::Unknown => "Unknown",
        connectrpc::ErrorCode::OutOfRange => "OutsideRange",
        connectrpc::ErrorCode::Unauthenticated => "AuthenticationRequired",
        _ => unreachable!("presenter emits a known status"),
    };
    format!("Store failure: {label}")
}

#[cfg(test)]
#[path = "error_test.rs"]
pub(crate) mod error_tests;
