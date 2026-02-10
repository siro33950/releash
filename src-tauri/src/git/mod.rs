pub mod error;
pub mod types;

pub(crate) mod branch;
pub(crate) mod commit;
pub(crate) mod config;
pub(crate) mod diff;
pub(crate) mod log;
pub(crate) mod stage;
pub(crate) mod status;
pub(crate) mod util;
pub(crate) mod worktree;

pub(crate) use branch::get_current_branch;
pub(crate) use commit::{git_commit, git_push};
pub(crate) use diff::{get_file_at_ref, get_staged_content};
pub(crate) use stage::{git_stage, git_stage_hunk, git_unstage};
pub(crate) use status::get_git_status;
pub(crate) use worktree::{get_main_repo_path, list_branches_with_status, list_worktrees};

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
