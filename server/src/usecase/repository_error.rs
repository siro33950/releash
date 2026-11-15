//! repository ユースケースのエラー型。
//!
//! ドメインエラー（`GitError`）から `#[from]` で変換し、adaptor 層で
//! `AppError` に集約される（`UsecaseError → AppError` 変換は adaptor 層に置く）。
//! `#[error(transparent)]` により serialize 表現（文字列）を移行前と等価に保つ。

use crate::domain::git::GitError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error(transparent)]
    AppConfig(#[from] crate::domain::app_config::AppConfigError),
    #[error(transparent)]
    Session(#[from] crate::usecase::agent_session::AgentSessionLaunchUsecaseError),
    #[error(transparent)]
    Workflow(#[from] crate::domain::workflow::WorkflowError),
    #[error(transparent)]
    Repository(#[from] GitError),
    /// ユースケースの業務ルール違反（削除拒否ポリシー等）。
    /// `#[error("{0}")]` によりメッセージ文字列をそのまま serialize し、
    /// 移行前（gateway が返していた `GitError::rule`）と等価に保つ。
    #[error("{0}")]
    Rule(String),
    /// commit を作った後の Push か Sync が失敗した。commit は残っている。中身は、その Push か Sync の失敗である。
    #[error("{0}")]
    AfterCommit(Box<UsecaseError>),
}

impl UsecaseError {
    /// commit を作った後の操作の失敗にする。
    pub fn after_commit(error: Self) -> Self {
        Self::AfterCommit(Box::new(error))
    }
}

#[cfg(test)]
#[path = "repository_error_test.rs"]
mod repository_error_tests;
