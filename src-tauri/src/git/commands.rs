use super::error::GitError;
use super::types::{BranchInfo, WorktreeBranch, WorktreeEntry};

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

#[tauri::command]
pub async fn get_status_diff_stats(
    repo_path: String,
) -> Result<Vec<super::types::StatusFileStat>, GitError> {
    blocking(move || super::status::get_status_diff_stats(repo_path)).await
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

#[tauri::command]
pub async fn get_binary_file_at_ref(
    file_path: String,
    git_ref: String,
) -> Result<String, GitError> {
    blocking(move || super::diff::get_binary_file_at_ref(file_path, git_ref)).await
}

// ── diff tree ──

#[tauri::command]
pub async fn build_diff_file_tree(
    entries: Vec<super::diff_tree::DiffFileEntry>,
) -> Result<Vec<super::diff_tree::DiffTreeNode>, GitError> {
    blocking(move || Ok(super::diff_tree::build_tree(entries))).await
}

#[tauri::command]
pub async fn get_file_navigation(
    tree: Vec<super::diff_tree::DiffTreeNode>,
    current_file: String,
) -> Result<super::diff_tree::FileNavigationResult, GitError> {
    blocking(move || Ok(super::diff_tree::get_file_navigation(&tree, &current_file))).await
}

// ── hunk / patch ──

#[tauri::command]
pub async fn compute_diff_hunks(
    original: String,
    modified: String,
    file_path: Option<String>,
) -> Result<super::types::DiffHunksResult, GitError> {
    blocking(move || {
        Ok(super::hunk::compute_diff_hunks(
            &original,
            &modified,
            file_path.as_deref(),
        ))
    })
    .await
}

#[tauri::command]
pub async fn generate_group_patch(
    file_path: String,
    hunk: super::types::Hunk,
    group: super::types::ChangeGroup,
) -> Result<String, GitError> {
    blocking(move || Ok(super::hunk::generate_group_patch(&file_path, &hunk, &group))).await
}

#[tauri::command]
pub async fn compute_hidden_ranges(
    hunks: Vec<super::types::Hunk>,
    total_lines: u32,
    context_lines: u32,
) -> Result<Vec<super::types::HiddenRange>, GitError> {
    blocking(move || {
        Ok(super::hunk::compute_hidden_ranges(
            &hunks,
            total_lines,
            context_lines,
        ))
    })
    .await
}

#[tauri::command]
pub async fn compute_hidden_ranges_from_content(
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<super::types::HiddenRange>, GitError> {
    blocking(move || {
        Ok(super::hunk::compute_hidden_ranges_from_content(
            &original,
            &modified,
            context_lines,
        ))
    })
    .await
}

#[tauri::command]
pub async fn compute_visible_markdown_blocks(
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<super::types::VisibleBlock>, GitError> {
    blocking(move || {
        Ok(super::hunk::compute_visible_markdown_blocks(
            &original,
            &modified,
            context_lines,
        ))
    })
    .await
}

#[tauri::command]
pub async fn get_language_from_path(file_path: String) -> Result<String, GitError> {
    blocking(move || Ok(super::lang::get_language_from_path(&file_path))).await
}

#[tauri::command]
pub async fn get_relative_path(
    root_path: String,
    file_path: String,
) -> Result<Option<String>, GitError> {
    blocking(move || Ok(super::hunk::get_relative_path(&root_path, &file_path))).await
}

// ── branch diff ──

#[tauri::command]
pub async fn get_branch_diff_summary(
    repo_path: String,
    base_branch: Option<String>,
) -> Result<super::branch_diff::BranchDiffSummary, GitError> {
    blocking(move || {
        super::branch_diff::get_branch_diff_summary(&repo_path, base_branch.as_deref())
    })
    .await
}
