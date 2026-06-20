/// ワークツリー（メイン / リンク済み）の識別情報。
///
/// worktree 単一集約に属する不変条件・配置情報のみを持つ。`dirty_count`（status 由来）や
/// `base_branch`（git_config 由来）といった別集約の表示・集計値はここに持たず、
/// 一覧表示用 read model（`WorktreeEntryDto`）を usecase が複数集約から合成する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
}
