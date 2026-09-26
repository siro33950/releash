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
            error: Box::new(self),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializeはプレーン文字列を返す() {
        // 移行前の GitError（プレーン文字列）と等価な観測表現の契約。
        // オブジェクトに包まず、メッセージそのものを JSON 文字列として返す。
        let err = AppError::new("リポジトリパスが設定されていません");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, "\"リポジトリパスが設定されていません\"");
    }

    #[test]
    fn displayはメッセージそのもの() {
        let err = AppError::new("boom");
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn coded_errorはcodeとmessageを返す() {
        let err = AppError::coded(
            "STALE_REVIEW_GROUP_TARGET",
            "review group target stale: g:old",
            connectrpc::ErrorCode::Aborted,
        );
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "STALE_REVIEW_GROUP_TARGET");
        assert_eq!(json["message"], "review group target stale: g:old");
        assert_eq!(err.to_string(), "review group target stale: g:old");
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

#[cfg(test)]
mod code_error_tests {
    use super::*;

    #[test]
    fn code由来のdisplayが変換チェーンを通じて保持される() {
        // CodeError → CodeUsecaseError → AppError の変換でメッセージ文字列が保持され、
        // serialize 表現が移行前（GitError のプレーン文字列）と等価であることをガードする。
        let usecase_err = CodeUsecaseError::Code(CodeError::Rule("file not staged".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "file not staged");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"file not staged\""
        );
    }

    #[test]
    fn external由来のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = CodeUsecaseError::Code(CodeError::External("git2 boom".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "git2 boom");
        assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
    }

    #[test]
    fn test_review_group操作_staleな対象はcodeと利用者向けmessageを持つerrorとして返す() {
        // Given
        let internal_group_id = "g:old:0";
        let usecase_err = CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget {
            group_id: internal_group_id.to_string(),
        });

        // When
        let app_err = AppError::from(usecase_err);

        // Then
        assert_eq!(app_err.to_string(), STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE);
        assert_eq!(
            serde_json::to_value(&app_err).unwrap(),
            serde_json::json!({
                "code": STALE_REVIEW_GROUP_TARGET_ERROR_CODE,
                "message": STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE
            })
        );
        assert!(!app_err.to_string().contains(internal_group_id));
        assert!(!app_err.to_string().contains("review group target stale"));
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
