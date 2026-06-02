/// repository ドメインのエラー型（DomainError）。
///
/// 外部リソース（git2・ファイル I/O 等）への依存を持たないよう、
/// 具体的な外部エラー型はメッセージ文字列として畳み込んで保持する。
/// 外部エラー → `RepositoryError` への変換は gateway 層
/// （`adaptor/gateway/shared/error_handling.rs`）で行う。
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    /// 外部リソース由来のエラー（git2・I/O 等）。メッセージを保持する。
    #[error("{0}")]
    External(String),
    /// ビジネスルール違反（既定ブランチ削除拒否・worktree 未発見等）。
    #[error("{0}")]
    Rule(String),
}

impl RepositoryError {
    pub fn rule(message: impl Into<String>) -> Self {
        Self::Rule(message.into())
    }
}
