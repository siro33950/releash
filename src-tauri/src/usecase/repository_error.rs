//! repository ユースケースのエラー型。
//!
//! ドメインエラー（`RepositoryError`）から `#[from]` で変換し、adaptor 層で
//! `AppError` に集約される（`UsecaseError → AppError` 変換は adaptor 層に置く）。
//! `#[error(transparent)]` により serialize 表現（文字列）を移行前と等価に保つ。

use crate::domain::repository::RepositoryError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// ユースケースの業務ルール違反（削除拒否ポリシー等）。
    /// `#[error("{0}")]` によりメッセージ文字列をそのまま serialize し、
    /// 移行前（gateway が返していた `RepositoryError::rule`）と等価に保つ。
    #[error("{0}")]
    Rule(String),
}
