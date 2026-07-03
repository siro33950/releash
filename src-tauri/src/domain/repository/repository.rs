//! repository ドメインの永続化／外部リソース抽象（trait）。
//!
//! git2・ファイル I/O 等の具体実装は知らない。具体実装は
//! `adaptor/gateway/repository/` に配置する。
//!
//! git2 のブロッキング呼び出しは gateway 内で同期的に行い、非同期境界
//! （`spawn_blocking`）は controller 層で被せる方針のため、各 trait の
//! メソッドは同期シグネチャで定義する。

use super::entities::{Branch, Commit, FileStatus, RepositoryStatusScan, Worktree};
use super::error::RepositoryError;

/// ブランチの参照・作成・削除。
pub trait BranchRepository: Send + Sync {
    fn list(&self, repo_path: &str) -> Result<Vec<Branch>, RepositoryError>;
    fn current(&self, repo_path: &str) -> Result<String, RepositoryError>;
    fn default(&self, repo_path: &str) -> Result<String, RepositoryError>;
    fn create(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError>;
    /// 単一ブランチを削除する純粋プリミティブ。既定/チェックアウト中ブランチの
    /// 拒否や紐づく worktree の事前削除といった業務手順は usecase が担う。
    fn delete(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError>;
}

/// コミット履歴の読み取り。
pub trait LogRepository: Send + Sync {
    fn log(&self, repo_path: &str, limit: Option<usize>) -> Result<Vec<Commit>, RepositoryError>;
}

/// 作業ツリー状態の読み取り。
pub trait StatusRepository: Send + Sync {
    fn status_with_options(
        &self,
        repo_path: &str,
        include_ignored: bool,
    ) -> Result<Vec<FileStatus>, RepositoryError>;
    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, RepositoryError>;
}

/// ワークツリーの参照・作成・削除。
pub trait WorktreeRepository: Send + Sync {
    fn main_repo_path(&self, any_path: &str) -> Result<String, RepositoryError>;
    fn dirty_count(&self, worktree_path: &str) -> Result<u32, RepositoryError>;
    fn list(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryError>;
    fn create(
        &self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<Worktree, RepositoryError>;
    /// worktree を削除し、削除した worktree が指していたブランチ名を返す
    /// （導出できない場合は `None`）。`releash-base` の後始末は usecase が
    /// この戻り値を使ってオーケストレーションする（gateway は worktree 集約
    /// のみの純粋 I/O に徹する）。
    fn remove(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, RepositoryError>;
    /// 壊れた（`validate()` 失敗）linked worktree を prune する。ブランチ削除
    /// 前のリカバリー等で用いる。個別エントリの prune 失敗は無視する。
    fn prune_invalid(&self, repo_path: &str) -> Result<(), RepositoryError>;
}

/// git config 上の releash base（global / per-branch）の読み書き。
///
/// read（`get_branch_base`）は per-branch override → global → default branch の
/// 解決後の実効値を返すのに対し、write（`set_branch_base_override`）は per-branch
/// override キー `branch.<name>.releash-base` のみを書く。read/write で対象が異なる
/// ため、write 側の名前で「override を書く」ことを明示する。
pub trait GitConfigRepository: Send + Sync {
    fn get_releash_base(&self, repo_path: &str) -> Result<Option<String>, RepositoryError>;
    fn set_releash_base(&self, repo_path: &str, base: Option<&str>) -> Result<(), RepositoryError>;
    fn get_branch_base(
        &self,
        repo_path: &str,
        branch_name: &str,
    ) -> Result<Option<String>, RepositoryError>;
    fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError>;
    /// `existing_branches` に含まれないブランチの `branch.*.releash-base`
    /// エントリを掃除する（ブランチ一覧取得時の GC）。
    fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryError>;
    /// `path_hint`（repo パスまたはファイルパス）から現在ブランチのベースブランチ名を
    /// 解決する（per-branch override → global → default branch）。detached HEAD /
    /// unborn / 解決不可は `None`。ref 存在検証・merge-base 計算は行わない。
    fn resolve_current_base_branch(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, RepositoryError>;
    /// 現在ブランチの実効ベースブランチ名を返す（agent プロセスへ渡す
    /// `RELEASH_BASE_BRANCH` 用）。`resolve_current_base_branch` の解決結果に加え、
    /// base が local/remote ref として実在し、かつ現在 HEAD と merge-base が計算できる
    /// ことを検証する。解決不可・detached・unborn・ref 不在・merge-base 不成立は `None`。
    #[allow(dead_code)] // issues-1301 D-5/G-1: retained for agent child-env base branch propagation.
    fn resolve_effective_base_branch(
        &self,
        repo_path: &str,
    ) -> Result<Option<String>, RepositoryError>;
    /// `path_hint` 配下のリポジトリで `base_name` の ref（local `refs/heads/<name>` →
    /// remote `refs/remotes/origin/<name>` の順）を解決し、base コミットの OID(hex) を返す。
    /// ref が実在しない場合は `None`。base ref → コミット OID の解決ルールを所有し、
    /// 呼び出し側（`code` ドメインの merge-base 計算）が ref 解決を重複実装しないようにする。
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, RepositoryError>;
}

/// リポジトリパスの解決ユーティリティ。
pub trait RepoLocator: Send + Sync {
    fn cwd(&self) -> Result<String, RepositoryError>;
    fn git_dir(&self, file_path: &str) -> Result<String, RepositoryError>;
}

/// 登録済みリポジトリパス一覧（メモリ共有リスト + アプリ設定への永続化）。
pub trait RepoPathsRepository: Send + Sync {
    fn get(&self) -> Vec<String>;
    /// 新規追加できた場合に `true`、既存・空文字で追加されなかった場合に `false`。
    fn add(&self, path: &str) -> Result<bool, RepositoryError>;
    /// 削除できた場合に `true`、存在せず削除されなかった場合に `false`。
    fn remove(&self, path: &str) -> Result<bool, RepositoryError>;
}

/// repo_paths 変更通知の port。実装は adaptor/gateway 層で Tauri/WS 等の送信
/// infra（`repo-paths-changed` イベント）を呼ぶ。「成功時に現在の一覧 payload で
/// 通知する」gating は usecase が担い、本 trait は送信手段のみを抽象化する。
pub trait RepoPathsNotifier: Send + Sync {
    /// 変更後の現在の一覧 payload で変更通知を発火する。
    fn notify_changed(&self, paths: Vec<String>);
}
