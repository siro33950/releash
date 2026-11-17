use super::{
    Branch, CommitMessage, Continuation, GitError, LineChanges, OperationKind, Push,
    RepositoryStatusScan, Worktree, WorktreeBranch,
};

/// Git のブランチ、作業ツリー、履歴を操作するドメインサービス。
pub trait GitService: Send + Sync {
    /// repository のブランチを列挙する。リモートのブランチは `origin/main` のように remote の名前を付けて返す。
    fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, GitError>;
    /// repository のデフォルトブランチの名前を返す。決められないときは `None`。
    fn default_branch(&self, repo_path: &str) -> Result<Option<String>, GitError>;
    /// 現在のブランチ、またはブランチが無い状態の表示を返す。
    fn current(&self, repo_path: &str) -> Result<String, GitError>;
    /// 指定したブランチを作成し、そのブランチに切り替える。
    fn create_branch(&self, repo_path: &str, branch_name: &str) -> Result<(), GitError>;

    /// 作業ツリーの変更と差分の状態を返す。
    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, GitError>;

    /// worktree が checkout しているブランチと、その状態を返す。
    ///
    /// - merge か rebase の途中なら、状態は `Operating` である。rebase の途中の名前は、rebase しているブランチの名前である。
    /// - upstream があれば `Tracking`、無ければ `Unpublished` である。
    /// - ブランチが無い状態では、`current` と同じ表示を名前にし、状態は `Unpublished` である。
    fn worktree_branch(&self, worktree: &str) -> Result<WorktreeBranch, GitError>;
    /// stage 済みの変更と、まだ stage していない変更の、ファイルごとの行数を返す。
    fn line_changes(&self, worktree: &str) -> Result<LineChanges, GitError>;

    /// stage 済みの変更から、メッセージを付けた commit を作る。署名は git の設定に従う。
    fn commit(&self, worktree: &str, message: &CommitMessage) -> Result<(), GitError>;
    /// ブランチの commit を、指定した送り方で送る。送り先は `Push` が決める。
    fn push(&self, worktree: &str, push: Push) -> Result<(), GitError>;
    /// upstream の commit をブランチに取り込む。取り込み方は git の設定に従う。
    /// conflict で止まったときは失敗を返し、worktree は merge か rebase の途中のまま残る。
    fn pull(&self, worktree: &str) -> Result<(), GitError>;
    /// upstream がある remote の状態を取得する。upstream が無ければ origin の状態を取得する。
    /// worktree のファイルとブランチの commit は変えない。
    fn fetch(&self, worktree: &str) -> Result<(), GitError>;
    /// 指定したファイルの worktree の内容を、index の内容に戻す。index は変えない。`paths` が空なら何もしない。
    fn restore_worktree_files(&self, worktree: &str, paths: &[String]) -> Result<(), GitError>;
    /// 指定したファイルを OS のゴミ箱に移す。`paths` が空なら何もしない。
    fn trash_files(&self, worktree: &str, paths: &[String]) -> Result<(), GitError>;
    /// 進行中の merge か rebase を、指定した続け方で先へ進める。
    /// rebase のメッセージは、止まっている commit だけに使う。同じ続行で進む後の commit のメッセージは変えない。
    /// rebase が次の commit の conflict で止まったときは失敗を返し、worktree は rebase の途中のまま残る。
    fn continue_operation(
        &self,
        worktree: &str,
        continuation: &Continuation,
    ) -> Result<(), GitError>;
    /// 進行中の merge か rebase を中止し、ブランチと worktree のファイルを、始める前の状態に戻す。
    fn abort_operation(&self, worktree: &str, operation: &OperationKind) -> Result<(), GitError>;

    /// repository の origin の URL を返す。repository を開けない、または origin が無いときは `None`。
    fn origin_url(&self, repo_path: &str) -> Result<Option<String>, GitError>;

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
    /// remote の名前を付けた `refs/remotes/<name>` → `refs/remotes/origin/<name>` の順）を解決し、
    /// base コミットの OID(hex) を返す。
    /// ref が実在しない場合は `None`。base ref → コミット OID の解決ルールを所有し、
    /// 呼び出し側（`code` ドメインの merge-base 計算）が ref 解決を重複実装しないようにする。
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, GitError>;
}
