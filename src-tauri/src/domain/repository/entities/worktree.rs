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

impl Worktree {
    pub fn authorize_removal(
        &self,
        force: bool,
        dirty_count: u32,
    ) -> Result<(), crate::domain::repository::RepositoryError> {
        use crate::domain::repository::RepositoryError;
        if self.is_main {
            return Err(RepositoryError::rule("cannot remove the main worktree"));
        }
        if !force {
            if self.is_locked {
                return Err(RepositoryError::rule("worktree is locked"));
            }
            if dirty_count > 0 {
                return Err(RepositoryError::rule(format!(
                    "worktree has {dirty_count} uncommitted change(s). Use force to remove."
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "worktree_test.rs"]
mod worktree_tests;
