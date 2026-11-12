//! code ドメインの外部リソース抽象（trait）。
//!
//! git2・ファイル I/O 等の具体実装は知らない。具体実装は
//! `adaptor/gateway/code/` に配置する。git2 のブロッキング呼び出しは gateway 内で
//! 同期的に行い、非同期境界（`spawn_blocking`）は controller 層で被せる方針のため、
//! 各 trait のメソッドは同期シグネチャで定義する。

use super::error::CodeError;
use super::value_objects::file_list::{WalkStep, WorktreeEntry};
use super::value_objects::Hunk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSideMetadata {
    Present { size_bytes: u64 },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewSideBytes {
    Present(Vec<u8>),
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewBlobSide {
    Original,
    Modified,
}

/// 各リビジョン時点のファイル内容参照（テキスト／バイナリ bytes / Base64）。
pub trait FileContentRepository: Send + Sync {
    fn review_file_metadata_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideMetadata, CodeError>;
    fn review_file_bytes_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideBytes, CodeError>;
    fn review_working_tree_metadata(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideMetadata, CodeError>;
    fn review_working_tree_bytes(&self, file_path: &str) -> Result<ReviewSideBytes, CodeError>;
    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeError>;
    fn review_working_tree_matches_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<bool, CodeError>;
}

/// 差分の Approve（staging）に関わる index 書き込み操作。
#[async_trait::async_trait]
pub trait StagingRepository: Send + Sync {
    fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError>;
    fn unstage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError>;
}

/// 2 つのテキストバッファの差分を hunk 列として計算する。
///
/// 差分アルゴリズム自体（git2 依存）を gateway へ閉じ込めるための抽象。
/// patch 生成といった後段の純粋ロジックはドメインサービス（`services::file_hunk`）が担う。
pub trait DiffComputer: Send + Sync {
    fn diff_buffers(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<Vec<Hunk>, CodeError>;
}

/// 現在ブランチのベースブランチ名解決の抽象。
///
/// ベースブランチ名の解決ルール（per-branch override → global → default branch）自体は
/// `repository` ドメインが所有する。`code` ドメインの branch diff / branch base 参照は
/// merge-base 計算のために base 名を必要とするため、この port 越しに名前だけを受け取り、
/// 解決ルールの実装は持たない。`path_hint` は repo パスまたはファイルパス。
/// detached HEAD / unborn / 解決不可は `None`。
pub trait BranchBaseResolver: Send + Sync {
    /// 現在ブランチの base 名を解決する（ref 実在の検証はしない）。merge-base 計算の
    /// 入力に用い、ref の存在検証・error は後段の gateway（`FileContentRepository`）が担う。
    fn resolve_base_branch_name(&self, path_hint: &str) -> Result<Option<String>, CodeError>;

    /// 解決済み base 名の ref を解決し、base コミット OID(hex) を返す。ref 実在検証
    /// （local → remote）は repository ドメインが所有し、後段の gateway は本 OID と現在
    /// HEAD から merge-base を計算する。ref 不在は `None`。
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, CodeError>;
}

/// worktree の中の項目の列挙。
pub trait WorktreeEntries: Send + Sync {
    /// `directory` の直下の項目を返す。
    ///
    /// `worktree_path_is_visible` が偽になる項目は返さない。
    fn entries(&self, worktree: &str, directory: &str) -> Result<Vec<WorktreeEntry>, CodeError>;

    /// worktree の項目を上から順に列挙し、一つずつ `visit` に渡す。
    ///
    /// - ディレクトリは、その中の項目より先に渡す。
    /// - `worktree_path_is_visible` が偽になる項目と、その中の項目は渡さない。
    /// - `visit` が `SkipChildren` を返したディレクトリの中は列挙しない。
    /// - `visit` が `Stop` を返したら、列挙を終えて成功を返す。
    /// - 読めないディレクトリの中の項目と、名前が UTF-8 として読めない項目は渡さず、ほかの項目の列挙を続ける。
    /// - worktree そのものを読めないときは失敗する。
    fn walk(
        &self,
        worktree: &str,
        visit: &mut dyn FnMut(WorktreeEntry) -> WalkStep,
    ) -> Result<(), CodeError>;
}
