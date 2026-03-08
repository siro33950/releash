use super::error::GitError;
use super::types::{AheadBehind, BranchInfo, WorktreeBranch, WorktreeEntry};

async fn blocking<T, F>(f: F) -> Result<T, GitError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, GitError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| GitError::Custom(format!("task join error: {e}")))?
}

// ── branch ──

#[tauri::command]
pub async fn list_branches(repo_path: String) -> Result<Vec<BranchInfo>, GitError> {
    blocking(move || super::branch::list_branches(repo_path)).await
}

#[tauri::command]
pub async fn get_current_branch(repo_path: String) -> Result<String, GitError> {
    blocking(move || super::branch::get_current_branch(repo_path)).await
}

#[tauri::command]
pub async fn get_current_branch_ahead_behind(repo_path: String) -> Result<AheadBehind, GitError> {
    blocking(move || super::branch::get_current_branch_ahead_behind(repo_path)).await
}

#[tauri::command]
pub async fn get_default_branch(repo_path: String) -> Result<String, GitError> {
    blocking(move || super::branch::get_default_branch(repo_path)).await
}

#[tauri::command]
pub async fn git_create_branch(repo_path: String, branch_name: String) -> Result<(), GitError> {
    blocking(move || super::branch::git_create_branch(repo_path, branch_name)).await
}

#[tauri::command]
pub async fn delete_branch(
    repo_path: String,
    branch_name: String,
    force: bool,
) -> Result<(), GitError> {
    blocking(move || super::branch::delete_branch(repo_path, branch_name, force)).await
}

// ── worktree ──

#[tauri::command]
pub async fn get_main_repo_path(any_path: String) -> Result<String, GitError> {
    blocking(move || super::worktree::get_main_repo_path(any_path)).await
}

#[tauri::command]
pub async fn get_worktree_dirty_count(worktree_path: String) -> Result<u32, GitError> {
    blocking(move || super::worktree::get_worktree_dirty_count(worktree_path)).await
}

#[tauri::command]
pub async fn list_worktrees(repo_path: String) -> Result<Vec<WorktreeEntry>, GitError> {
    blocking(move || super::worktree::list_worktrees(repo_path)).await
}

#[tauri::command]
pub async fn list_branches_with_status(repo_path: String) -> Result<Vec<WorktreeBranch>, GitError> {
    blocking(move || super::worktree::list_branches_with_status(repo_path)).await
}

#[tauri::command]
pub async fn create_worktree(
    repo_path: String,
    worktree_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<WorktreeEntry, GitError> {
    blocking(move || {
        super::worktree::create_worktree(
            repo_path,
            worktree_path,
            branch,
            create_branch,
            base_branch,
        )
    })
    .await
}

#[tauri::command]
pub async fn remove_worktree(
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), GitError> {
    blocking(move || super::worktree::remove_worktree(repo_path, worktree_path, force)).await
}

// ── util ──

#[tauri::command]
pub async fn get_cwd() -> Result<String, GitError> {
    blocking(super::util::get_cwd).await
}

#[tauri::command]
pub async fn get_repo_git_dir(file_path: String) -> Result<String, GitError> {
    blocking(move || super::util::get_repo_git_dir(file_path)).await
}

// ── config ──

#[tauri::command]
pub async fn get_releash_base(repo_path: String) -> Result<Option<String>, GitError> {
    blocking(move || super::config::get_releash_base(repo_path)).await
}

#[tauri::command]
pub async fn set_releash_base(repo_path: String, base: Option<String>) -> Result<(), GitError> {
    blocking(move || super::config::set_releash_base(repo_path, base)).await
}

#[tauri::command]
pub async fn get_branch_base(
    repo_path: String,
    branch_name: String,
) -> Result<Option<String>, GitError> {
    blocking(move || super::config::get_branch_base(repo_path, branch_name)).await
}

#[tauri::command]
pub async fn set_branch_base(
    repo_path: String,
    branch_name: String,
    base: Option<String>,
) -> Result<(), GitError> {
    blocking(move || super::config::set_branch_base(repo_path, branch_name, base)).await
}

// ── status ──

#[tauri::command]
pub async fn get_git_status(
    repo_path: String,
) -> Result<Vec<super::types::GitFileStatus>, GitError> {
    blocking(move || super::status::get_git_status(repo_path)).await
}

// ── log ──

#[tauri::command]
pub async fn get_git_log(
    repo_path: String,
    limit: Option<usize>,
) -> Result<Vec<super::types::CommitInfo>, GitError> {
    blocking(move || super::log::get_git_log(repo_path, limit)).await
}

// ── stage ──

#[tauri::command]
pub async fn git_stage(repo_path: String, paths: Vec<String>) -> Result<(), GitError> {
    blocking(move || super::stage::git_stage(repo_path, paths)).await
}

#[tauri::command]
pub async fn git_unstage(repo_path: String, paths: Vec<String>) -> Result<(), GitError> {
    blocking(move || super::stage::git_unstage(repo_path, paths)).await
}

#[tauri::command]
pub async fn git_stage_hunk(repo_path: String, patch: String) -> Result<(), GitError> {
    blocking(move || super::stage::git_stage_hunk(repo_path, patch)).await
}

#[tauri::command]
pub async fn git_unstage_hunk(repo_path: String, patch: String) -> Result<(), GitError> {
    blocking(move || super::stage::git_unstage_hunk(repo_path, patch)).await
}

#[tauri::command]
pub async fn git_discard(repo_path: String, paths: Vec<String>) -> Result<(), GitError> {
    blocking(move || super::stage::git_discard(repo_path, paths)).await
}

// ── commit ──

#[tauri::command]
pub async fn git_commit(repo_path: String, message: String) -> Result<String, GitError> {
    blocking(move || super::commit::git_commit(repo_path, message)).await
}

#[tauri::command]
pub async fn git_push(repo_path: String) -> Result<String, GitError> {
    blocking(move || super::commit::git_push(repo_path)).await
}

// ── diff ──

#[tauri::command]
pub async fn get_file_at_ref(file_path: String, git_ref: String) -> Result<String, GitError> {
    blocking(move || super::diff::get_file_at_ref(file_path, git_ref)).await
}

#[tauri::command]
pub async fn get_staged_content(file_path: String) -> Result<String, GitError> {
    blocking(move || super::diff::get_staged_content(file_path)).await
}

#[tauri::command]
pub async fn get_binary_file_at_ref(
    file_path: String,
    git_ref: String,
) -> Result<String, GitError> {
    blocking(move || super::diff::get_binary_file_at_ref(file_path, git_ref)).await
}

#[tauri::command]
pub async fn get_binary_staged_content(file_path: String) -> Result<String, GitError> {
    blocking(move || super::diff::get_binary_staged_content(file_path)).await
}

#[tauri::command]
pub async fn get_file_at_branch_base(file_path: String) -> Result<String, GitError> {
    blocking(move || super::diff::get_file_at_branch_base(file_path)).await
}

#[tauri::command]
pub async fn get_binary_file_at_branch_base(file_path: String) -> Result<String, GitError> {
    blocking(move || super::diff::get_binary_file_at_branch_base(file_path)).await
}

// ── review ──

#[tauri::command]
pub async fn get_review_diff_summary(
    repo_path: String,
    base_branch: Option<String>,
) -> Result<super::review::ReviewDiff, GitError> {
    blocking(move || super::review::get_review_diff(&repo_path, base_branch.as_deref(), None, None))
        .await
}
