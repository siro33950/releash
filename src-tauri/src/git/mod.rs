pub mod error;
pub mod types;

mod branch;
mod commit;
mod config;
mod diff;
mod log;
mod stage;
pub(crate) mod status;
mod util;
mod worktree;

pub use branch::{get_current_branch, get_default_branch, git_create_branch, list_branches};
pub use commit::{git_commit, git_push};
pub use config::{get_releash_base, set_releash_base};
pub use diff::{get_file_at_ref, get_staged_content};
pub use log::get_git_log;
pub use stage::{git_stage, git_stage_hunk, git_unstage, git_unstage_hunk};
pub use status::get_git_status;
pub use util::{get_cwd, get_repo_git_dir};
pub use worktree::{
    create_worktree, get_main_repo_path, get_worktree_dirty_count, list_branches_with_status,
    list_worktrees, remove_worktree,
};

#[cfg(test)]
pub(crate) mod test_helpers {
    use git2::{Repository, Signature};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    pub fn create_test_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        (dir, repo)
    }

    pub fn create_initial_commit(repo: &Repository) -> git2::Oid {
        let sig = Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap()
    }

    pub fn add_and_commit(
        repo: &Repository,
        path: &str,
        content: &str,
        message: &str,
    ) -> git2::Oid {
        let workdir = repo.workdir().unwrap();
        fs::write(workdir.join(path), content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();

        let sig = Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap()
    }
}
