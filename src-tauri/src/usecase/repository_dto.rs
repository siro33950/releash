//! repository ユースケースの read model（DTO）。
//!
//! Entity と同形になる単純な読み取りは Entity（`domain::repository` の各 Entity）を
//! そのまま返すため、ここに 1:1 の DTO は置かない。ここに定義するのは、対応する単一
//! Entity を持たない集約 read model のみ。

use serde::{Deserialize, Serialize};

/// ワークツリー一覧の 1 エントリ（旧 `WorktreeEntry`）の read model。
///
/// worktree 識別情報（`domain::repository::Worktree`）に `dirty_count`（status 由来）と
/// `base_branch`（git_config 由来）を合成した表示・転送向けモデル。単一 Entity の 1:1 写像では
/// なく、複数集約を usecase が合成して組み立てる read model であり domain Entity ではない。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeEntryDto {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    pub base_branch: Option<String>,
}

/// ブランチカード（旧 `WorktreeBranch`）の read model。
///
/// 単一の読み取りクエリ結果を denormalize しただけの表示・転送向けモデルであり
/// domain Entity ではない。Query 経路（[`BranchCardQuery`](super::repository_query_service::BranchCardQuery)）の
/// gateway 実装がデータソース（git2）から直接組み立てる。PR 情報（別ドメイン git_host 由来）は
/// この repository read model には含めず、呼び出し側が別途取得・合成する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCardDto {
    pub name: String,
    pub is_main_worktree: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
    pub base_ahead: usize,
}
