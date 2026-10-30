use super::GitError;

/// リポジトリパスの解決ユーティリティ。
pub trait RepoLocator: Send + Sync {
    fn cwd(&self) -> Result<String, GitError>;
}

/// 登録済み repository の一覧を取得する。
pub trait RepoPathsRepository: Send + Sync {
    fn get(&self) -> Vec<String>;
}
