use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::{ErrorCode, ObjectType, Repository, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CHECKPOINT_DIR: &str = "agent-worktree-checkpoints";
const BACKUP_DIR: &str = "agent-worktree-checkpoint-backups";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCheckpointFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub staged: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub staged_content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub staged_executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCheckpoint {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub head_oid: Option<String>,
    pub files: Vec<WorktreeCheckpointFile>,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RewindWorktreeCheckpointPreview {
    pub available: bool,
    pub target_dirty_file_count: usize,
    pub current_dirty_file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub head_oid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCheckpointRestoreResult {
    pub restored_file_count: usize,
    pub restored_index_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backup_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeCheckpointBackup<'a> {
    version: u32,
    worktree_path: &'a str,
    checkpoint: WorktreeCheckpoint,
}

fn checkpoint_path(app_data_dir: &Path, session_id: &str, message_id: &str) -> PathBuf {
    app_data_dir
        .join(CHECKPOINT_DIR)
        .join(session_id)
        .join(format!("{message_id}.json"))
}

fn backup_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join(BACKUP_DIR)
        .join(format!("{}.json", uuid::Uuid::new_v4()))
}

fn current_head_oid(repo: &Repository) -> Result<Option<String>, String> {
    match repo.head() {
        Ok(head) => {
            let Some(oid) = head.target() else {
                return Ok(None);
            };
            Ok(Some(oid.to_string()))
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
            Ok(None)
        }
        Err(e) => Err(format!("Failed to read repository HEAD: {e}")),
    }
}

fn is_safe_relative_path(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn safe_relative_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path.is_absolute() || !is_safe_relative_path(&path) {
        return Err("Checkpoint contains an unsafe path".to_string());
    }
    Ok(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read restored file metadata: {e}"))?;
    let mut permissions = metadata.permissions();
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o755;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|e| format!("Failed to update restored file permissions: {e}"))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

fn checkpoint_digest(files: &[WorktreeCheckpointFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update(file.content_base64.as_deref().unwrap_or("").as_bytes());
        hasher.update([0]);
        hasher.update(if file.executable { b"1" } else { b"0" });
        hasher.update([0]);
        hasher.update(if file.staged { b"1" } else { b"0" });
        hasher.update([0]);
        hasher.update(
            file.staged_content_base64
                .as_deref()
                .unwrap_or("")
                .as_bytes(),
        );
        hasher.update([0]);
        hasher.update(if file.staged_executable { b"1" } else { b"0" });
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

fn head_blob(repo: &Repository, path: &str) -> Result<Option<(Vec<u8>, bool)>, String> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
            return Ok(None);
        }
        Err(e) => return Err(format!("Failed to read repository HEAD: {e}")),
    };
    let tree = head
        .peel_to_tree()
        .map_err(|e| format!("Failed to read HEAD tree: {e}"))?;
    let entry = match tree.get_path(Path::new(path)) {
        Ok(entry) => entry,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read HEAD entry for {path}: {e}")),
    };
    if entry.kind() != Some(ObjectType::Blob) {
        return Ok(None);
    }
    let blob = repo
        .find_blob(entry.id())
        .map_err(|e| format!("Failed to read HEAD blob for {path}: {e}"))?;
    Ok(Some((
        blob.content().to_vec(),
        entry.filemode() == 0o100755,
    )))
}

fn index_blob(repo: &Repository, path: &str) -> Result<Option<(Vec<u8>, bool)>, String> {
    let mut index = repo
        .index()
        .map_err(|e| format!("Failed to read Git index: {e}"))?;
    index
        .read(true)
        .map_err(|e| format!("Failed to refresh Git index: {e}"))?;
    let Some(entry) = index.get_path(Path::new(path), 0) else {
        return Ok(None);
    };
    let blob = repo
        .find_blob(entry.id)
        .map_err(|e| format!("Failed to read index blob for {path}: {e}"))?;
    Ok(Some((blob.content().to_vec(), entry.mode == 0o100755)))
}

fn has_index_change(status: Status) -> bool {
    status.is_index_new()
        || status.is_index_modified()
        || status.is_index_deleted()
        || status.is_index_renamed()
        || status.is_index_typechange()
}

pub fn capture_worktree_checkpoint(worktree_path: &str) -> Result<WorktreeCheckpoint, String> {
    let repo = Repository::discover(worktree_path)
        .map_err(|e| format!("Failed to open Git repository for checkpoint: {e}"))?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| "Cannot checkpoint a bare repository".to_string())?;
    let head_oid = current_head_oid(&repo)?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|e| format!("Failed to read Git status for checkpoint: {e}"))?;
    let mut files = BTreeMap::<String, WorktreeCheckpointFile>::new();
    for entry in statuses.iter() {
        let Ok(path) = entry.path() else {
            continue;
        };
        let relative = safe_relative_path(path)?;
        let absolute = workdir.join(&relative);
        let content_base64 = if absolute.is_file() {
            Some(
                STANDARD.encode(
                    std::fs::read(&absolute)
                        .map_err(|e| format!("Failed to read checkpoint file {path}: {e}"))?,
                ),
            )
        } else {
            None
        };
        let staged = has_index_change(entry.status());
        let (staged_content_base64, staged_executable) = if staged {
            match index_blob(&repo, path)? {
                Some((content, executable)) => (Some(STANDARD.encode(content)), executable),
                None => (None, false),
            }
        } else {
            (None, false)
        };
        files.insert(
            path.to_string(),
            WorktreeCheckpointFile {
                path: path.to_string(),
                content_base64,
                executable: is_executable(&absolute),
                staged,
                staged_content_base64,
                staged_executable,
            },
        );
    }
    let files = files.into_values().collect::<Vec<_>>();
    let digest = checkpoint_digest(&files);
    Ok(WorktreeCheckpoint {
        version: 1,
        head_oid,
        files,
        digest,
    })
}

pub fn save_message_worktree_checkpoint(
    app_data_dir: &Path,
    session_id: &str,
    message_id: &str,
    checkpoint: &WorktreeCheckpoint,
) -> Result<(), String> {
    let path = checkpoint_path(app_data_dir, session_id, message_id);
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid checkpoint path".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create checkpoint directory: {e}"))?;
    let json = serde_json::to_string_pretty(checkpoint)
        .map_err(|e| format!("Failed to serialize checkpoint: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save checkpoint: {e}"))
}

pub fn capture_and_save_message_worktree_checkpoint(
    app_data_dir: &Path,
    session_id: &str,
    message_id: &str,
    worktree_path: &str,
) -> Result<(), String> {
    let checkpoint = capture_worktree_checkpoint(worktree_path)?;
    save_message_worktree_checkpoint(app_data_dir, session_id, message_id, &checkpoint)
}

pub fn load_message_worktree_checkpoint(
    app_data_dir: &Path,
    session_id: &str,
    message_id: &str,
) -> Result<Option<WorktreeCheckpoint>, String> {
    let path = checkpoint_path(app_data_dir, session_id, message_id);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read checkpoint: {e}"))?;
    let checkpoint: WorktreeCheckpoint = serde_json::from_str(&content).map_err(|e| {
        format!("Failed to parse checkpoint for session {session_id} message {message_id}: {e}")
    })?;
    let expected = checkpoint_digest(&checkpoint.files);
    if checkpoint.digest != expected {
        return Err(format!(
            "Checkpoint digest mismatch for session {session_id} message {message_id}"
        ));
    }
    Ok(Some(checkpoint))
}

pub fn copy_message_checkpoints(
    app_data_dir: &Path,
    from_session_id: &str,
    to_session_id: &str,
    message_ids: &[String],
) -> Result<(), String> {
    for message_id in message_ids {
        let Some(checkpoint) =
            load_message_worktree_checkpoint(app_data_dir, from_session_id, message_id)?
        else {
            continue;
        };
        save_message_worktree_checkpoint(app_data_dir, to_session_id, message_id, &checkpoint)?;
    }
    Ok(())
}

pub fn preview_rewind_worktree_checkpoint(
    app_data_dir: &Path,
    session_id: &str,
    message_id: &str,
    worktree_path: &str,
) -> Result<RewindWorktreeCheckpointPreview, String> {
    let Some(checkpoint) = load_message_worktree_checkpoint(app_data_dir, session_id, message_id)?
    else {
        return Ok(RewindWorktreeCheckpointPreview {
            available: false,
            target_dirty_file_count: 0,
            current_dirty_file_count: 0,
            head_oid: None,
            reason: Some("No worktree checkpoint is stored for this message".to_string()),
        });
    };
    let current = capture_worktree_checkpoint(worktree_path)?;
    let head_matches = current.head_oid == checkpoint.head_oid;
    Ok(RewindWorktreeCheckpointPreview {
        available: head_matches,
        target_dirty_file_count: checkpoint.files.len(),
        current_dirty_file_count: current.files.len(),
        head_oid: checkpoint.head_oid,
        reason: if head_matches {
            None
        } else {
            Some("Repository HEAD has changed since this checkpoint".to_string())
        },
    })
}

fn write_file(workdir: &Path, file: &WorktreeCheckpointFile) -> Result<(), String> {
    let content = file
        .content_base64
        .as_deref()
        .ok_or_else(|| "Checkpoint file has no content".to_string())
        .and_then(|content| {
            STANDARD
                .decode(content)
                .map_err(|e| format!("Failed to decode checkpoint content: {e}"))
        })?;
    write_file_bytes(workdir, &file.path, &content, file.executable)
}

fn write_file_bytes(
    workdir: &Path,
    path: &str,
    content: &[u8],
    executable: bool,
) -> Result<(), String> {
    let relative = safe_relative_path(path)?;
    let absolute = workdir.join(relative);
    if let Some(parent) = absolute.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create restored file directory: {e}"))?;
    }
    std::fs::write(&absolute, content)
        .map_err(|e| format!("Failed to write restored file {path}: {e}"))?;
    set_executable(&absolute, executable)
}

fn remove_path(workdir: &Path, path: &str) -> Result<(), String> {
    let absolute = workdir.join(safe_relative_path(path)?);
    if absolute.is_dir() {
        std::fs::remove_dir_all(&absolute)
            .map_err(|e| format!("Failed to remove restored directory {path}: {e}"))?;
    } else if absolute.exists() {
        std::fs::remove_file(&absolute)
            .map_err(|e| format!("Failed to remove restored file {path}: {e}"))?;
    }
    Ok(())
}

fn restore_head_or_remove(repo: &Repository, workdir: &Path, path: &str) -> Result<(), String> {
    if let Some((content, executable)) = head_blob(repo, path)? {
        let file = WorktreeCheckpointFile {
            path: path.to_string(),
            content_base64: Some(STANDARD.encode(content)),
            executable,
            staged: false,
            staged_content_base64: None,
            staged_executable: false,
        };
        write_file(workdir, &file)
    } else {
        remove_path(workdir, path)
    }
}

fn remove_index_path(index: &mut git2::Index, path: &str) -> Result<(), String> {
    match index.remove_path(&safe_relative_path(path)?) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to remove restored index entry {path}: {e}")),
    }
}

fn add_workdir_path_to_index(index: &mut git2::Index, path: &str) -> Result<(), String> {
    index
        .add_path(&safe_relative_path(path)?)
        .map_err(|e| format!("Failed to add restored index entry {path}: {e}"))
}

fn restore_index_path(
    repo: &Repository,
    workdir: &Path,
    index: &mut git2::Index,
    path: &str,
    target_file: Option<&WorktreeCheckpointFile>,
) -> Result<bool, String> {
    if let Some(file) = target_file.filter(|file| file.staged) {
        if let Some(content) = file.staged_content_base64.as_deref() {
            let content = STANDARD
                .decode(content)
                .map_err(|e| format!("Failed to decode checkpoint index content: {e}"))?;
            write_file_bytes(workdir, &file.path, &content, file.staged_executable)?;
            add_workdir_path_to_index(index, &file.path)?;
        } else {
            remove_index_path(index, &file.path)?;
        }
        return Ok(true);
    }

    if let Some((content, executable)) = head_blob(repo, path)? {
        write_file_bytes(workdir, path, &content, executable)?;
        add_workdir_path_to_index(index, path)?;
    } else {
        remove_index_path(index, path)?;
    }
    Ok(false)
}

fn save_backup(
    app_data_dir: &Path,
    worktree_path: &str,
    checkpoint: &WorktreeCheckpoint,
) -> Result<Option<String>, String> {
    if checkpoint.files.is_empty() {
        return Ok(None);
    }
    let path = backup_path(app_data_dir);
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid checkpoint backup path".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create checkpoint backup directory: {e}"))?;
    let backup = WorktreeCheckpointBackup {
        version: 1,
        worktree_path,
        checkpoint: checkpoint.clone(),
    };
    let json = serde_json::to_string_pretty(&backup)
        .map_err(|e| format!("Failed to serialize checkpoint backup: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save checkpoint backup: {e}"))?;
    Ok(Some(path.display().to_string()))
}

pub fn restore_worktree_checkpoint(
    app_data_dir: &Path,
    worktree_path: &str,
    checkpoint: &WorktreeCheckpoint,
) -> Result<WorktreeCheckpointRestoreResult, String> {
    let repo = Repository::discover(worktree_path)
        .map_err(|e| format!("Failed to open Git repository for checkpoint restore: {e}"))?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| "Cannot restore a checkpoint in a bare repository".to_string())?;
    let current_head_oid = current_head_oid(&repo)?;
    if current_head_oid != checkpoint.head_oid {
        return Err("Repository HEAD has changed since this checkpoint".to_string());
    }
    let current = capture_worktree_checkpoint(worktree_path)?;
    let backup_path = save_backup(app_data_dir, worktree_path, &current)?;
    let target_files = checkpoint
        .files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let mut paths = current
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    paths.extend(target_files.keys().cloned());
    let restored_file_count = paths.len();
    for path in &paths {
        match target_files.get(path) {
            Some(file) if file.content_base64.is_some() => write_file(workdir, file)?,
            Some(_) => remove_path(workdir, path)?,
            None => restore_head_or_remove(&repo, workdir, path)?,
        }
    }
    let mut index = repo
        .index()
        .map_err(|e| format!("Failed to open Git index for checkpoint restore: {e}"))?;
    let mut restored_index_count = 0;
    for path in &paths {
        if restore_index_path(
            &repo,
            workdir,
            &mut index,
            path,
            target_files.get(path).copied(),
        )? {
            restored_index_count += 1;
        }
    }
    index
        .write()
        .map_err(|e| format!("Failed to write restored Git index: {e}"))?;
    for path in &paths {
        match target_files.get(path) {
            Some(file) if file.content_base64.is_some() => write_file(workdir, file)?,
            Some(_) => remove_path(workdir, path)?,
            None => restore_head_or_remove(&repo, workdir, path)?,
        }
    }
    Ok(WorktreeCheckpointRestoreResult {
        restored_file_count,
        restored_index_count,
        backup_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{IndexAddOption, Signature};
    use tempfile::TempDir;

    fn init_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "base\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["tracked.txt"], IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("Releash", "releash@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        drop(tree);
        (dir, repo)
    }

    #[test]
    fn restore_worktree_checkpoint_restores_dirty_state_and_removes_later_changes() {
        let app_data = TempDir::new().unwrap();
        let (repo_dir, _repo) = init_repo();
        std::fs::write(repo_dir.path().join("tracked.txt"), "checkpoint\n").unwrap();
        std::fs::write(repo_dir.path().join("untracked.txt"), "new\n").unwrap();
        let checkpoint = capture_worktree_checkpoint(repo_dir.path().to_str().unwrap()).unwrap();

        std::fs::write(repo_dir.path().join("tracked.txt"), "later\n").unwrap();
        std::fs::write(repo_dir.path().join("later.txt"), "later\n").unwrap();

        let result = restore_worktree_checkpoint(
            app_data.path(),
            repo_dir.path().to_str().unwrap(),
            &checkpoint,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(repo_dir.path().join("tracked.txt")).unwrap(),
            "checkpoint\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo_dir.path().join("untracked.txt")).unwrap(),
            "new\n"
        );
        assert!(!repo_dir.path().join("later.txt").exists());
        assert!(result.backup_path.is_some());
    }

    #[test]
    fn restore_worktree_checkpoint_restores_staged_and_unstaged_versions() {
        let app_data = TempDir::new().unwrap();
        let (repo_dir, repo) = init_repo();
        std::fs::write(repo_dir.path().join("tracked.txt"), "staged\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        std::fs::write(repo_dir.path().join("tracked.txt"), "worktree\n").unwrap();
        let checkpoint = capture_worktree_checkpoint(repo_dir.path().to_str().unwrap()).unwrap();

        std::fs::write(repo_dir.path().join("tracked.txt"), "later staged\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        std::fs::write(repo_dir.path().join("tracked.txt"), "later worktree\n").unwrap();

        let result = restore_worktree_checkpoint(
            app_data.path(),
            repo_dir.path().to_str().unwrap(),
            &checkpoint,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(repo_dir.path().join("tracked.txt")).unwrap(),
            "worktree\n"
        );
        let (staged_content, _) = index_blob(&repo, "tracked.txt").unwrap().unwrap();
        assert_eq!(staged_content, b"staged\n");
        let statuses = repo.statuses(None).unwrap();
        let status = statuses
            .iter()
            .find(|entry| entry.path().ok() == Some("tracked.txt"))
            .unwrap()
            .status();
        assert!(status.is_index_modified());
        assert!(status.is_wt_modified());
        assert_eq!(result.restored_index_count, 1);
    }

    #[test]
    fn restore_worktree_checkpoint_rejects_changed_head() {
        let app_data = TempDir::new().unwrap();
        let (repo_dir, repo) = init_repo();
        let checkpoint = capture_worktree_checkpoint(repo_dir.path().to_str().unwrap()).unwrap();
        std::fs::write(repo_dir.path().join("second.txt"), "second\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["second.txt"], IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = Signature::now("Releash", "releash@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&head_commit])
            .unwrap();

        let err = restore_worktree_checkpoint(
            app_data.path(),
            repo_dir.path().to_str().unwrap(),
            &checkpoint,
        )
        .unwrap_err();

        assert_eq!(err, "Repository HEAD has changed since this checkpoint");
    }

    #[test]
    fn load_message_worktree_checkpoint_rejects_digest_mismatch() {
        let app_data = TempDir::new().unwrap();
        let checkpoint = WorktreeCheckpoint {
            version: 1,
            head_oid: None,
            files: vec![WorktreeCheckpointFile {
                path: "a.txt".to_string(),
                content_base64: Some(STANDARD.encode("content")),
                executable: false,
                staged: false,
                staged_content_base64: None,
                staged_executable: false,
            }],
            digest: "not-the-real-digest".to_string(),
        };
        save_message_worktree_checkpoint(app_data.path(), "session-1", "message-1", &checkpoint)
            .unwrap();

        let err = load_message_worktree_checkpoint(app_data.path(), "session-1", "message-1")
            .unwrap_err();

        assert!(err.contains("Checkpoint digest mismatch"));
    }
}
