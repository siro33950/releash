use super::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::domain::app_data_gc::{GcReport, RetentionPolicy};
use crate::domain::workflow::WORKFLOW_ARCHIVE_REASON_MANUAL;
use crate::usecase::agent_session::session::{ChatMessage, MessagePart, MessageRole, SessionState};
use sha2::{Digest, Sha256};

pub(super) const NOW: f64 = 10_000_000.0;

pub(super) struct TestFs;

impl GcFileSystem for TestFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        fs::read_dir(path)
            .map_err(gc_file_system_error)?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(gc_file_system_error)
            })
            .collect()
    }

    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        fs::read_to_string(path).map_err(gc_file_system_error)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        if !path.exists() {
            return Ok(false);
        }
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(gc_file_system_error)?;
        } else {
            fs::remove_file(path).map_err(gc_file_system_error)?;
        }
        Ok(true)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        if path.is_file() {
            return Ok(path.metadata().map_err(gc_file_system_error)?.len());
        }
        let mut size = 0;
        for entry in self.read_dir(path)? {
            size += self.recursive_size(&entry)?;
        }
        Ok(size)
    }
}

pub(super) struct TestArchivePruner;

impl WorkflowArchivePruner for TestArchivePruner {
    fn prune_workflow_archive_records(
        &self,
        app_data_dir: &Path,
        execution_ids: &HashSet<String>,
    ) -> Result<WorkflowArchivePruneResult, GcFileSystemError> {
        let path = app_data_dir.join("workflow_execution_archives.json");
        if !path.exists() {
            return Ok(WorkflowArchivePruneResult::default());
        }
        let before = path.metadata().map_err(gc_file_system_error)?.len();
        let content = fs::read_to_string(&path).map_err(gc_file_system_error)?;
        let mut value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|error| GcFileSystemError::other(error.to_string()))?;
        let Some(executions) = value
            .get_mut("executions")
            .and_then(|executions| executions.as_object_mut())
        else {
            return Ok(WorkflowArchivePruneResult::default());
        };
        let mut removed = 0;
        for execution_id in execution_ids {
            if executions.remove(execution_id).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(WorkflowArchivePruneResult::default());
        }
        let json = serde_json::to_string(&value)
            .map_err(|error| GcFileSystemError::other(error.to_string()))?;
        fs::write(&path, json).map_err(gc_file_system_error)?;
        let after = path.metadata().map_err(gc_file_system_error)?.len();
        Ok(WorkflowArchivePruneResult {
            records_removed: removed,
            reclaimed_bytes: before.saturating_sub(after),
        })
    }
}

pub(super) struct FailingReadDirFs {
    pub(super) failing_dir: PathBuf,
}

impl GcFileSystem for FailingReadDirFs {
    fn exists(&self, path: &Path) -> bool {
        TestFs.exists(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        TestFs.is_file(path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        if path == self.failing_dir {
            return Err(GcFileSystemError::other("permission denied"));
        }
        TestFs.read_dir(path)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        TestFs.read_to_string(path)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        TestFs.remove_path(path)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        TestFs.recursive_size(path)
    }
}

pub(super) struct FailingReadFileFs {
    pub(super) failing_file: PathBuf,
}

impl GcFileSystem for FailingReadFileFs {
    fn exists(&self, path: &Path) -> bool {
        TestFs.exists(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        TestFs.is_file(path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        TestFs.read_dir(path)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        if path == self.failing_file {
            return Err(GcFileSystemError::other("permission denied"));
        }
        TestFs.read_to_string(path)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        TestFs.remove_path(path)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        TestFs.recursive_size(path)
    }
}

pub(super) struct FailingRemoveFs {
    pub(super) failing_path: PathBuf,
}

impl GcFileSystem for FailingRemoveFs {
    fn exists(&self, path: &Path) -> bool {
        TestFs.exists(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        TestFs.is_file(path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        TestFs.read_dir(path)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        TestFs.read_to_string(path)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        if path == self.failing_path {
            return Err(GcFileSystemError::other("permission denied"));
        }
        TestFs.remove_path(path)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        TestFs.recursive_size(path)
    }
}

pub(super) fn modified_secs(path: &Path) -> Result<f64, String> {
    path.metadata()
        .map_err(|error| error.to_string())?
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .map_err(|error| error.to_string())
}

pub(super) fn set_mtime(path: &Path, secs: f64) {
    let time = filetime::FileTime::from_unix_time(secs as i64, 0);
    filetime::set_file_mtime(path, time).unwrap();
}

pub(super) fn run_gc(
    app_data_dir: &Path,
    live_worktrees: Option<LiveWorktreeSet>,
    process_records: Vec<ProcessRecord>,
    now_secs: f64,
) -> GcReport {
    run_startup_gc(
        startup_gc_request(
            app_data_dir,
            live_worktrees.map(full_resolution),
            RuntimeProtection::default(),
            process_records,
            now_secs,
        ),
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    )
}

pub(super) fn run_gc_with_runtime_protection(
    app_data_dir: &Path,
    live_worktrees: Option<LiveWorktreeSet>,
    runtime_protection: RuntimeProtection,
    process_records: Vec<ProcessRecord>,
    now_secs: f64,
) -> GcReport {
    run_startup_gc(
        startup_gc_request(
            app_data_dir,
            live_worktrees.map(full_resolution),
            runtime_protection,
            process_records,
            now_secs,
        ),
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    )
}

pub(super) fn run_gc_with_resolution(
    app_data_dir: &Path,
    live_worktrees: Option<LiveWorktreeResolution>,
    runtime_protection: RuntimeProtection,
    now_secs: f64,
) -> GcReport {
    run_startup_gc(
        startup_gc_request(
            app_data_dir,
            live_worktrees,
            runtime_protection,
            Vec::new(),
            now_secs,
        ),
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    )
}

pub(super) fn startup_gc_request(
    app_data_dir: &Path,
    live_worktrees: Option<LiveWorktreeResolution>,
    runtime_protection: RuntimeProtection,
    process_records: Vec<ProcessRecord>,
    now_secs: f64,
) -> StartupGcRequest {
    StartupGcRequest {
        app_data_dir: app_data_dir.to_path_buf(),
        live_worktrees,
        session_records: collect_test_session_records(app_data_dir),
        workflow_executions: collect_test_workflow_executions(app_data_dir),
        workspace_state_records: collect_test_workspace_state_records(app_data_dir),
        review_comment_records: collect_test_review_comment_records(app_data_dir),
        checkpoint_paths: collect_test_checkpoint_paths(app_data_dir),
        cache_records: collect_test_cache_records(app_data_dir),
        legacy_comment_paths: collect_test_legacy_comment_paths(app_data_dir),
        session_blob_stores: collect_test_session_blob_stores(app_data_dir),
        process_records,
        runtime_protection,
        now_secs,
        retention: RetentionPolicy::default(),
    }
}

fn collect_test_session_records(app_data_dir: &Path) -> Vec<SessionGcRecord> {
    let sessions_dir = app_data_dir.join("sessions");
    let Ok(entries) = fs::read_dir(&sessions_dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(meta) = parse_test_session_meta(&path.join("meta.json"), id) else {
                continue;
            };
            records.push(SessionGcRecord {
                id: meta.id.unwrap_or_else(|| id.to_string()),
                delete_paths: vec![path.clone()],
                dir_path: Some(path),
                worktree_path: meta.worktree_path,
                state: meta.state,
                updated_at: meta.updated_at,
            });
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem.ends_with(".meta") {
            continue;
        }
        let id = stem.to_string();
        let sidecar = sessions_dir.join(format!("{id}.meta.json"));
        let Some(meta) = (if sidecar.exists() {
            parse_test_session_meta(&sidecar, &id)
        } else {
            parse_test_session_meta(&path, &id)
        }) else {
            continue;
        };
        let mut delete_paths = vec![path];
        if sidecar.exists() {
            delete_paths.push(sidecar);
        }
        records.push(SessionGcRecord {
            id: meta.id.unwrap_or(id),
            delete_paths,
            dir_path: None,
            worktree_path: meta.worktree_path,
            state: meta.state,
            updated_at: meta.updated_at,
        });
    }
    records
}

#[derive(Default)]
struct TestSessionMeta {
    id: Option<String>,
    worktree_path: Option<GcWorktreePath>,
    state: Option<SessionState>,
    updated_at: Option<f64>,
}

fn parse_test_session_meta(path: &Path, fallback_id: &str) -> Option<TestSessionMeta> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(TestSessionMeta {
                id: Some(fallback_id.to_string()),
                ..TestSessionMeta::default()
            });
        }
        Err(error) => {
            log::warn!(
                "app data gc skipped session {} because meta {} could not be read: {error}",
                fallback_id,
                path.display()
            );
            return None;
        }
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Some(TestSessionMeta {
            id: Some(fallback_id.to_string()),
            ..TestSessionMeta::default()
        });
    };
    Some(test_session_meta_from_value(&value, fallback_id))
}

fn test_session_meta_from_value(value: &serde_json::Value, fallback_id: &str) -> TestSessionMeta {
    TestSessionMeta {
        id: value
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| Some(fallback_id.to_string())),
        worktree_path: value
            .get("worktreePath")
            .and_then(|value| value.as_str())
            .map(normalized_gc_worktree_path),
        state: value
            .get("state")
            .and_then(|value| serde_json::from_value::<SessionState>(value.clone()).ok()),
        updated_at: value.get("updatedAt").and_then(|value| value.as_f64()),
    }
}

fn collect_test_session_blob_stores(app_data_dir: &Path) -> Vec<SessionBlobStore> {
    let sessions_dir = app_data_dir.join("sessions");
    let Ok(entries) = fs::read_dir(&sessions_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|session_dir| SessionBlobStore {
            messages_dir: session_dir.join("messages"),
            tool_outputs_dir: session_dir.join("tool_outputs"),
            attachments_dir: session_dir.join("attachments"),
            session_dir,
        })
        .collect()
}

fn collect_test_workflow_executions(app_data_dir: &Path) -> Vec<WorkflowExecutionGcRecord> {
    let archived_at_by_execution = test_manual_archive_times(app_data_dir);
    let executions_dir = app_data_dir.join("workflow_executions");
    let Ok(entries) = fs::read_dir(&executions_dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(execution) = serde_json::from_str::<TestWorkflowExecutionMeta>(&content) else {
            continue;
        };
        records.push(WorkflowExecutionGcRecord {
            execution_id: execution.execution_id.clone(),
            worktree_path: normalized_gc_worktree_path(&execution.worktree_path),
            is_terminal: execution.status.is_terminal(),
            manual_archived_at: archived_at_by_execution
                .get(&execution.execution_id)
                .copied(),
            delete_paths: test_workflow_execution_delete_paths(
                app_data_dir,
                &execution.execution_id,
            ),
        });
    }
    records
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestWorkflowExecutionMeta {
    execution_id: String,
    status: TestWorkflowExecutionStatus,
    worktree_path: String,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum TestWorkflowExecutionStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

impl TestWorkflowExecutionStatus {
    fn is_terminal(self) -> bool {
        match self {
            Self::Completed | Self::Failed | Self::Aborted => true,
            Self::Running | Self::WaitingApproval => false,
        }
    }
}

fn test_manual_archive_times(app_data_dir: &Path) -> HashMap<String, f64> {
    let path = app_data_dir.join("workflow_execution_archives.json");
    let Ok(content) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return HashMap::new();
    };
    let Some(executions) = value
        .get("executions")
        .and_then(|executions| executions.as_object())
    else {
        return HashMap::new();
    };
    executions
        .iter()
        .filter_map(|(execution_id, record)| {
            let archived_at = record.get("archivedAt").and_then(|value| value.as_f64())?;
            let reason = record.get("archiveReason").and_then(|value| value.as_str());
            let restored_at = record.get("restoredAt").and_then(|value| value.as_f64());
            (reason == Some(WORKFLOW_ARCHIVE_REASON_MANUAL) && restored_at.is_none())
                .then(|| (execution_id.clone(), archived_at))
        })
        .collect()
}

fn test_workflow_execution_delete_paths(app_data_dir: &Path, execution_id: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(
        app_data_dir
            .join("workflow_executions")
            .join(format!("{execution_id}.json")),
    );
    for extension in ["ndjson", "json"] {
        paths.push(
            app_data_dir
                .join("workflow_execution_logs")
                .join(format!("{execution_id}.{extension}")),
        );
    }
    let workflow_dir = app_data_dir.join("workflow");
    for path in [
        workflow_dir.join(execution_id),
        workflow_dir.join(format!("{execution_id}.json")),
        workflow_dir.join(format!("{execution_id}.ndjson")),
    ] {
        if path.exists() {
            paths.push(path);
        }
    }
    for subdir in ["pending", "processing", "processed"] {
        let dir = app_data_dir.join("workflow_pending").join(subdir);
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if test_pending_command_execution_id(&content).as_deref() == Some(execution_id) {
                paths.push(path);
            }
        }
    }
    paths
}

fn test_pending_command_execution_id(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    value
        .get("execution_id")
        .or_else(|| value.get("executionId"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn collect_test_workspace_state_records(app_data_dir: &Path) -> Vec<WorkspaceStateGcRecord> {
    let dir = app_data_dir.join("workspace_state");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                return None;
            }
            let key = path.file_stem().and_then(|stem| stem.to_str())?.to_string();
            Some(WorkspaceStateGcRecord { path, key })
        })
        .collect()
}

fn collect_test_review_comment_records(app_data_dir: &Path) -> Vec<ReviewCommentGcRecord> {
    let dir = app_data_dir.join("review-comments");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name().and_then(|name| name.to_str())?;
            let key = name
                .strip_suffix(".events.json")
                .or_else(|| name.strip_suffix(".events.lock"))?
                .to_string();
            Some(ReviewCommentGcRecord { path, key })
        })
        .collect()
}

fn collect_test_checkpoint_paths(app_data_dir: &Path) -> Vec<PathBuf> {
    [
        "agent-worktree-checkpoints",
        "agent-worktree-checkpoint-backups",
    ]
    .into_iter()
    .map(|dirname| app_data_dir.join(dirname))
    .filter(|path| path.exists())
    .collect()
}

fn collect_test_cache_records(app_data_dir: &Path) -> Vec<CacheGcRecord> {
    let mut records = Vec::new();
    let lsp_dir = app_data_dir.join("lsp");
    for relative in ["jdtls", "typescript", "jdtls.version"] {
        push_test_cache_record(&lsp_dir.join(relative), &mut records);
    }
    let jdtls_workspaces = lsp_dir.join("jdtls-workspaces");
    if let Ok(entries) = fs::read_dir(jdtls_workspaces) {
        for entry in entries.flatten() {
            push_test_cache_record(&entry.path(), &mut records);
        }
    }
    records
}

fn push_test_cache_record(path: &Path, records: &mut Vec<CacheGcRecord>) {
    if path.exists() {
        records.push(CacheGcRecord {
            path: path.to_path_buf(),
            updated_at: latest_mtime_secs(path).unwrap_or(NOW),
        });
    }
}

fn latest_mtime_secs(path: &Path) -> Result<f64, String> {
    let mut latest = modified_secs(path)?;
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            latest = latest.max(latest_mtime_secs(
                &entry.map_err(|error| error.to_string())?.path(),
            )?);
        }
    }
    Ok(latest)
}

fn collect_test_legacy_comment_paths(app_data_dir: &Path) -> Vec<PathBuf> {
    ["comments", "diff-comments", "threads"]
        .into_iter()
        .map(|dirname| app_data_dir.join(dirname))
        .filter(|path| path.exists())
        .collect()
}

pub(super) fn full_resolution(live_worktrees: LiveWorktreeSet) -> LiveWorktreeResolution {
    LiveWorktreeResolution::new(live_worktrees, Vec::new(), HashSet::new())
}

pub(super) fn partial_resolution(
    live_worktrees: LiveWorktreeSet,
    unresolved_repo_paths: Vec<String>,
    unresolved_workspace_state_key_prefixes: HashSet<String>,
) -> LiveWorktreeResolution {
    LiveWorktreeResolution::new(
        live_worktrees,
        unresolved_repo_paths,
        unresolved_workspace_state_key_prefixes,
    )
}

pub(super) fn live_set(worktrees: &[(&str, &Path)]) -> LiveWorktreeSet {
    LiveWorktreeSet::from_worktrees(worktrees.iter().map(|(name, path)| LiveWorktree {
        path: normalized_worktree_path(&path.to_string_lossy()),
        workspace_state_keys: workspace_state_keys(name, path),
        review_comment_keys: review_comment_keys(path),
    }))
}

fn workspace_state_keys(name: &str, path: &Path) -> Vec<String> {
    let path_string = path.to_string_lossy();
    let normalized = normalized_worktree_path(&path_string);
    let mut keys = vec![
        workspace_state_storage_key(name),
        workspace_state_storage_key(&path_string),
        workspace_state_storage_key(&normalized),
    ];
    if let Some(file_name) = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
    {
        keys.push(workspace_state_storage_key(file_name));
    }
    keys
}

fn review_comment_keys(path: &Path) -> Vec<String> {
    let path_string = path.to_string_lossy();
    vec![
        review_comment_storage_key(&path_string),
        review_comment_storage_key(&normalized_worktree_path(&path_string)),
    ]
}

pub(super) fn normalized_worktree_path(path: &str) -> String {
    let mut trimmed = path.trim().to_string();
    while trimmed.len() > 1 && (trimmed.ends_with('/') || trimmed.ends_with('\\')) {
        trimmed.pop();
    }
    Path::new(&trimmed)
        .canonicalize()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or(trimmed)
}

pub(super) fn normalized_gc_worktree_path(path: &str) -> GcWorktreePath {
    let mut trimmed = path.trim().to_string();
    while trimmed.len() > 1 && (trimmed.ends_with('/') || trimmed.ends_with('\\')) {
        trimmed.pop();
    }
    match Path::new(&trimmed).canonicalize() {
        Ok(path) => path
            .to_str()
            .map(|path| GcWorktreePath::resolved(path.to_string()))
            .unwrap_or_else(|| GcWorktreePath::resolved(trimmed)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            GcWorktreePath::not_found(trimmed)
        }
        Err(_) => GcWorktreePath::unresolved(trimmed),
    }
}

pub(super) struct TestRevalidationReader;

impl GcRevalidationReader for TestRevalidationReader {
    fn runtime_protection(
        &self,
        _app_data_dir: &Path,
        _process_records: &[ProcessRecord],
    ) -> RuntimeProtection {
        RuntimeProtection::default()
    }

    fn session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> RevalidationRead<CurrentSessionState> {
        let sessions_dir = app_data_dir.join("sessions");
        let session_dir = sessions_dir.join(session_id);
        if session_dir.is_dir() {
            return read_current_test_session_meta(&session_dir.join("meta.json"), session_id);
        }
        let session_file = sessions_dir.join(format!("{session_id}.json"));
        let sidecar = sessions_dir.join(format!("{session_id}.meta.json"));
        if sidecar.exists() {
            return read_current_test_session_meta(&sidecar, session_id);
        }
        if session_file.exists() {
            return read_current_test_session_meta(&session_file, session_id);
        }
        RevalidationRead::Missing
    }

    fn workflow_execution_state(
        &self,
        app_data_dir: &Path,
        execution_id: &str,
    ) -> RevalidationRead<CurrentWorkflowExecutionState> {
        let archived_at_by_execution = test_manual_archive_times(app_data_dir);
        read_current_test_workflow_execution_state(
            app_data_dir,
            execution_id,
            &archived_at_by_execution,
        )
    }

    fn workflow_execution_states(
        &self,
        app_data_dir: &Path,
        execution_ids: &HashSet<String>,
    ) -> HashMap<String, RevalidationRead<CurrentWorkflowExecutionState>> {
        let archived_at_by_execution = test_manual_archive_times(app_data_dir);
        execution_ids
            .iter()
            .map(|execution_id| {
                (
                    execution_id.clone(),
                    read_current_test_workflow_execution_state(
                        app_data_dir,
                        execution_id,
                        &archived_at_by_execution,
                    ),
                )
            })
            .collect()
    }
}

fn read_current_test_workflow_execution_state(
    app_data_dir: &Path,
    execution_id: &str,
    archived_at_by_execution: &HashMap<String, f64>,
) -> RevalidationRead<CurrentWorkflowExecutionState> {
    let path = app_data_dir
        .join("workflow_executions")
        .join(format!("{execution_id}.json"));
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RevalidationRead::Missing;
        }
        Err(error) => return RevalidationRead::Unavailable(error.to_string()),
    };
    let execution: TestWorkflowExecutionMeta = match serde_json::from_str(&content) {
        Ok(execution) => execution,
        Err(error) => return RevalidationRead::Unavailable(error.to_string()),
    };
    RevalidationRead::Present(CurrentWorkflowExecutionState {
        worktree_path: normalized_gc_worktree_path(&execution.worktree_path),
        is_terminal: execution.status.is_terminal(),
        manual_archived_at: archived_at_by_execution.get(execution_id).copied(),
    })
}

fn read_current_test_session_meta(
    path: &Path,
    fallback_id: &str,
) -> RevalidationRead<CurrentSessionState> {
    match parse_test_session_meta(path, fallback_id) {
        Some(meta) => RevalidationRead::Present(CurrentSessionState {
            worktree_path: meta.worktree_path,
            state: meta.state,
            updated_at: meta.updated_at,
        }),
        None => RevalidationRead::Unavailable("session meta could not be read".to_string()),
    }
}

pub(super) fn workspace_state_storage_key(worktree_name: &str) -> String {
    worktree_name.replace(['/', '\\'], "_")
}

pub(super) fn review_comment_storage_key(worktree: &str) -> String {
    let trimmed = worktree.trim();
    let canonical = Path::new(trimmed)
        .canonicalize()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| trimmed.to_string());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let label = Path::new(&canonical)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("worktree")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{label}-{}", &digest[..24])
}

pub(super) fn write_session(
    app_data_dir: &Path,
    id: &str,
    worktree_path: &Path,
    state: &str,
    updated_at: f64,
) -> PathBuf {
    let dir = app_data_dir.join("sessions").join(id);
    fs::create_dir_all(dir.join("messages")).unwrap();
    fs::write(dir.join("index.json"), "[]").unwrap();
    fs::write(
        dir.join("meta.json"),
        serde_json::json!({
            "id": id,
            "worktreePath": worktree_path.to_string_lossy(),
            "state": state,
            "updatedAt": updated_at
        })
        .to_string(),
    )
    .unwrap();
    dir
}

pub(super) fn write_message(session_dir: &Path, seq: u64, parts: Vec<MessagePart>) {
    fs::write(
        session_dir.join("messages").join(format!("{seq}.json")),
        serde_json::to_string(&ChatMessage {
            id: format!("m{seq}"),
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(parts),
            streaming_final_seq: 0,
            timestamp: NOW,
            mentions: None,
        })
        .unwrap(),
    )
    .unwrap();
}

pub(super) fn write_workflow_execution(
    app_data_dir: &Path,
    id: &str,
    worktree_path: &Path,
    status: &str,
) {
    fs::create_dir_all(app_data_dir.join("workflow_executions")).unwrap();
    fs::create_dir_all(app_data_dir.join("workflow_execution_logs")).unwrap();
    fs::write(
        app_data_dir
            .join("workflow_executions")
            .join(format!("{id}.json")),
        serde_json::json!({
            "executionId": id,
            "status": status,
            "worktreePath": worktree_path.to_string_lossy()
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        app_data_dir
            .join("workflow_execution_logs")
            .join(format!("{id}.ndjson")),
        "log",
    )
    .unwrap();
}

pub(super) fn write_archive_index(app_data_dir: &Path, entries: &[(&str, f64, Option<f64>)]) {
    let executions = entries
        .iter()
        .map(|(id, archived_at, restored_at)| {
            (
                (*id).to_string(),
                serde_json::json!({
                    "archivedAt": archived_at,
                    "archiveReason": WORKFLOW_ARCHIVE_REASON_MANUAL,
                    "restoredAt": restored_at
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    fs::write(
        app_data_dir.join("workflow_execution_archives.json"),
        serde_json::json!({ "executions": executions }).to_string(),
    )
    .unwrap();
}
