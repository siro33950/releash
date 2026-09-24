//! code ユースケースのエラー型。
//!
//! ドメインエラー（`CodeError`）から `#[from]` で変換し、adaptor 層で `AppError` に
//! 集約される。`#[error(transparent)]` により serialize 表現（文字列）を移行前の
//! `GitError`（プレーン文字列）と等価に保つ。

use crate::domain::code::CodeError;

#[derive(Debug, thiserror::Error)]
pub enum CodeUsecaseError {
    #[error(transparent)]
    Code(#[from] CodeError),
}

impl crate::domain::failure::ClassifiedFailure for CodeUsecaseError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        match self {
            Self::Code(error) => error.failure_kind(),
        }
    }
}

#[cfg(test)]
#[path = "code_error_test.rs"]
mod code_error_tests;
