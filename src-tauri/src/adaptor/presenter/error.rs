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
    Classified {
        kind: crate::domain::failure::FailureKind,
        error: Box<AppError>,
    },
    #[error("{0}")]
    Internal(String),
    #[error("{message}")]
    Coded {
        code: String,
        message: String,
        kind: crate::domain::failure::FailureKind,
    },
}

impl AppError {
    pub fn from_failure(
        error: impl crate::domain::failure::ClassifiedFailure + std::fmt::Display,
    ) -> Self {
        let kind = error.failure_kind();
        Self::new(error.to_string()).with_failure_kind(kind)
    }

    pub fn with_failure_kind(self, kind: crate::domain::failure::FailureKind) -> Self {
        Self::Classified {
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
        kind: crate::domain::failure::FailureKind,
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
            Self::Classified { error, .. } => error.serialize(serializer),
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
            crate::domain::failure::FailureKind::RestartRequired,
        );
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "STALE_REVIEW_GROUP_TARGET");
        assert_eq!(json["message"], "review group target stale: g:old");
        assert_eq!(err.to_string(), "review group target stale: g:old");
    }
}

impl crate::domain::failure::ClassifiedFailure for AppError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Classified { kind, .. } => *kind,
            Self::Internal(_) => FailureKind::Internal,
            Self::Coded { kind, .. } => *kind,
        }
    }
}
