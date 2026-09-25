/// repository ドメインのエラー型（DomainError）。
///
/// 外部リソース（git2・ファイル I/O 等）への依存を持たないよう、
/// 具体的な外部エラー型はメッセージ文字列として畳み込んで保持する。
/// 外部エラー → `RepositoryError` への変換は gateway 層
/// （`adaptor/gateway/shared/error_handling.rs`）で行う。
#[derive(Debug)]
pub enum RepositoryError {
    Stopped(crate::domain::operation_context::OperationStopped),
    /// 外部リソース由来のエラー（git2・I/O 等）。メッセージを保持する。
    External(String),
    /// ビジネスルール違反（既定ブランチ削除拒否・worktree 未発見等）。
    Rule(String),
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped(error) => std::fmt::Display::fmt(error, f),
            Self::External(msg) | Self::Rule(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for RepositoryError {}

impl RepositoryError {
    pub fn rule(message: impl Into<String>) -> Self {
        Self::Rule(message.into())
    }
}

impl crate::domain::failure::ClassifiedFailure for RepositoryError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::Stopped(error) => crate::domain::failure::ClassifiedFailure::failure_kind(error),
            Self::External(_) => F::Internal,
            Self::Rule(_) => F::StateRequired,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;

impl From<crate::domain::operation_context::OperationStopped> for RepositoryError {
    fn from(error: crate::domain::operation_context::OperationStopped) -> Self {
        Self::Stopped(error)
    }
}
