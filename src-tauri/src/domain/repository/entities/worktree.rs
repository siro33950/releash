/// ワークツリー（メイン / リンク済み）と、そこに checkout されたブランチ。
///
/// worktree 単一集約に属する不変条件・配置情報と、ブランチが base に取り込まれたかを持つ。
/// `dirty_count`（status 由来）や `base_branch`（git_config 由来）、PR（git_host 由来）といった
/// 別集約の値はここに持たない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub is_merged: bool,
}

impl Worktree {
    /// git の登録が先に消えた、削除中の worktree。
    pub fn being_deleted(path: &str, branch: String) -> Self {
        Self {
            name: branch.clone(),
            path: path.to_string(),
            branch,
            is_main: false,
            is_locked: false,
            is_merged: false,
        }
    }

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
