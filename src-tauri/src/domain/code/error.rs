//! code ドメインのエラー型（DomainError）。
//!
//! 外部リソース（git2・ファイル I/O 等）への依存を持たないよう、具体的な外部
//! エラー型はメッセージ文字列として畳み込んで保持する。外部エラー → `CodeError`
//! への変換は gateway 層（`adaptor/gateway/code/error.rs`）で行う。
//!
//! フロント／リモートへ返却される serialize 表現は、移行前の `GitError`
//! （プレーン文字列）と等価であることを契約とする。いずれの variant も
//! メッセージ文字列のみを保持し、`Display` はメッセージそのものを返す。
#[derive(Debug)]
pub enum CodeError {
    /// 外部リソース由来のエラー（git2・I/O・UTF-8・パス変換等）。メッセージを保持する。
    External(String),
    /// ドメインルール／前提条件の違反（bare repository・未ステージ・unborn branch 等）。
    /// 移行前の `GitError::Custom` が表していたメッセージと等価に保つ。
    Rule(String),
    /// review-blob URI が参照する snapshot version が現在の snapshot と一致しない。
    StaleReviewBlobVersion { requested: u64, current: u64 },
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::External(msg) | Self::Rule(msg) => f.write_str(msg),
            Self::StaleReviewBlobVersion { requested, current } => write!(
                f,
                "stale review blob version: requested {requested}, current {current}"
            ),
        }
    }
}

impl std::error::Error for CodeError {}
