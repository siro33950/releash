use super::{Branch, GitError, RepositoryStatusScan, Worktree};

/// Git のブランチ、作業ツリー、履歴を操作するドメインサービス。
pub trait GitService: Send + Sync {
    /// repository のブランチを列挙する。
    fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, GitError>;
    /// 現在のブランチ、またはブランチが無い状態の表示を返す。
    fn current(&self, repo_path: &str) -> Result<String, GitError>;
    /// 指定したブランチを作成し、そのブランチに切り替える。
    fn create_branch(&self, repo_path: &str, branch_name: &str) -> Result<(), GitError>;

    /// 作業ツリーの変更と差分の状態を返す。
    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, GitError>;

    /// 指定したパスが属する主 repository のパスを返す。
    fn main_repo_path(&self, any_path: &str) -> Result<String, GitError>;
    /// 指定したパスが repository に属するとき、その主 repository のパスを返す。
    fn find_main_repo_path(&self, any_path: &str) -> Result<Option<String>, GitError>;
    /// repository の worktree を列挙する。
    fn list_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, GitError>;
    /// 指定したブランチと作成条件に従って worktree を作成する。
    fn create_worktree(
        &self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<Worktree, GitError>;
    /// worktree を削除できるか確かめ、削除対象のパスを返す。
    fn validate_removal(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<String, GitError>;
    /// worktree を削除し、取得できた場合はそのブランチ名を返す。
    fn remove(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, GitError>;
    /// ブランチ個別のベース指定を保存する。None は指定の削除を表す。
    fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), GitError>;
    /// 指定したブランチの個別のベース指定を削除する。
    fn remove_branch_base_overrides(
        &self,
        repo_path: &str,
        branches: &[String],
    ) -> Result<(), GitError>;
    /// `path_hint`（repo パスまたはファイルパス）から現在ブランチのベースブランチ名を
    /// 解決する（per-branch override → global → default branch）。detached HEAD /
    /// unborn / 解決不可は `None`。ref 存在検証・merge-base 計算は行わない。
    fn resolve_current_base_branch(&self, path_hint: &str) -> Result<Option<String>, GitError>;
    /// `path_hint` 配下のリポジトリで `base_name` の ref（local `refs/heads/<name>` →
    /// remote `refs/remotes/origin/<name>` の順）を解決し、base コミットの OID(hex) を返す。
    /// ref が実在しない場合は `None`。base ref → コミット OID の解決ルールを所有し、
    /// 呼び出し側（`code` ドメインの merge-base 計算）が ref 解決を重複実装しないようにする。
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, GitError>;
}
