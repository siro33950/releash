//! code ドメインのエラー型（DomainError）。
//!
//! 外部リソース（git2・ファイル I/O 等）への依存を持たないよう、具体的な外部
//! エラー型はメッセージ文字列として畳み込んで保持する。外部エラー → `CodeError`
//! への変換は gateway 層（`adaptor/gateway/code/error.rs`）で行う。
//!
//! フロント／リモートへ返却される serialize 表現は、移行前の `GitError`
//! （プレーン文字列）と等価であることを契約とする。いずれの variant も
//! メッセージ文字列のみを保持し、`Display` はメッセージそのものを返す。
#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    /// 外部リソース由来のエラー（git2・I/O・UTF-8・パス変換等）。メッセージを保持する。
    #[error("{0}")]
    External(String),
    /// ドメインルール／前提条件の違反（bare repository・未ステージ・unborn branch 等）。
    /// 移行前の `GitError::Custom` が表していたメッセージと等価に保つ。
    #[error("{0}")]
    Rule(String),
}
