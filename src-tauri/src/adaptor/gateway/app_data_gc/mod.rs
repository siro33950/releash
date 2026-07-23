#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[cfg(test)]
use crate::adaptor::gateway::agent_session::session_storage::FileSessionStorage;
use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
#[cfg(test)]
use crate::adaptor::gateway::workflow::WorkflowExecutionFileRepository;
use crate::infrastructure::process::pid_registry::{
    process_group_status, read_pid_file, recorded_pid_status, ProcessStatus,
};
#[cfg(test)]
use crate::usecase::agent_session::session::SessionState;
use crate::usecase::app_data_gc::{
    CacheGcRecord, GcRevalidationReader, LiveWorktree, LiveWorktreeResolution, LiveWorktreeSet,
    ProcessRecord, ProcessRecordStatus, ReviewCommentGcRecord, RuntimeProtection, StartupGcRequest,
    WorkspaceStateGcRecord,
};
#[cfg(test)]
use crate::usecase::app_data_gc::{
    CurrentSessionState, CurrentWorkflowExecutionState, RevalidationRead,
};

mod std_file_system;

pub(crate) use std_file_system::{StdGcFileSystem, StdWorkflowArchivePruner};

pub(crate) fn build_startup_gc_request(
    app_data_dir: PathBuf,
    shared_repo_paths: SharedRepoPaths,
) -> StartupGcRequest {
    let process_records = collect_process_records(&app_data_dir);
    let runtime_protection = collect_post_cutover_runtime_protection(&process_records);
    StartupGcRequest {
        app_data_dir: app_data_dir.clone(),
        live_worktrees: resolve_live_worktrees(shared_repo_paths),
        #[cfg(test)]
        session_records: Vec::new(),
        #[cfg(test)]
        workflow_executions: Vec::new(),
        workspace_state_records: collect_workspace_state_records(&app_data_dir),
        review_comment_records: collect_review_comment_records(&app_data_dir),
        checkpoint_paths: collect_checkpoint_paths(&app_data_dir),
        cache_records: collect_cache_records(&app_data_dir),
        legacy_comment_paths: collect_legacy_comment_paths(&app_data_dir),
        #[cfg(test)]
        session_blob_stores: Vec::new(),
        process_records,
        runtime_protection,
        now_secs: crate::other::utils::unix_timestamp_seconds(),
        retention: crate::domain::app_data_gc::RetentionPolicy::default(),
    }
}

fn collect_post_cutover_runtime_protection(process_records: &[ProcessRecord]) -> RuntimeProtection {
    let active_session_ids = process_records
        .iter()
        .filter(|record| record.status == ProcessRecordStatus::Live)
        .filter_map(|record| record.session_id.clone())
        .collect();
    // Session/workflow migration sources are immutable after cutover. Until
    // startup GC gains canonical SQLite projection inputs, conservatively
    // mark workspace-keyed protection incomplete instead of inspecting those
    // legacy roots.
    RuntimeProtection::new(active_session_ids, Vec::new(), LiveWorktreeSet::default())
        .with_workspace_keyed_protection_complete(false)
}

fn resolve_live_worktrees(shared_repo_paths: SharedRepoPaths) -> Option<LiveWorktreeResolution> {
    let repo_paths = shared_repo_paths.read().clone();
    if repo_paths.is_empty() {
        return None;
    }
    let mut live_worktrees = Vec::new();
    let mut unresolved_repo_paths = Vec::new();
    let mut unresolved_workspace_state_key_prefixes = HashSet::new();
    for repo_path in repo_paths {
        let worktrees = match crate::adaptor::gateway::repository::worktree::list_worktrees(
            &repo_path,
        ) {
            Ok(worktrees) => worktrees,
            Err(error) => {
                let normalized_repo_path = normalize_path(&repo_path);
                log::warn!(
                    "app data gc skipped workspace-dependent cleanup for data that may belong to {}: failed to list worktrees: {error}",
                    repo_path
                );
                unresolved_workspace_state_key_prefixes
                    .extend(workspace_state_key_prefixes(&repo_path));
                unresolved_repo_paths.push(normalized_repo_path);
                continue;
            }
        };
        live_worktrees.extend(
            worktrees
                .into_iter()
                .map(|worktree| live_worktree(worktree.name, worktree.path)),
        );
    }
    Some(LiveWorktreeResolution::new(
        LiveWorktreeSet::from_worktrees(live_worktrees),
        unresolved_repo_paths,
        unresolved_workspace_state_key_prefixes,
    ))
}

fn collect_process_records(app_data_dir: &Path) -> Vec<ProcessRecord> {
    let mut records = Vec::new();
    records.extend(collect_agent_process_records(app_data_dir));
    records.extend(collect_legacy_pid_records(app_data_dir));
    records
}

fn collect_agent_process_records(app_data_dir: &Path) -> Vec<ProcessRecord> {
    let mut records = Vec::new();
    let dir = app_data_dir.join("agent-processes");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return records;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let record = match read_pid_file(&path) {
            Ok(pid_file) => ProcessRecord {
                path,
                session_id: Some(pid_file.session_id.clone()),
                status: process_status_to_gc(recorded_pid_status(&pid_file)),
            },
            Err(error) => {
                log::warn!(
                    "app data gc skipped unreadable process record {}: {error}",
                    path.display()
                );
                ProcessRecord {
                    path,
                    session_id: None,
                    status: ProcessRecordStatus::Unknown,
                }
            }
        };
        records.push(record);
    }
    records
}

fn collect_legacy_pid_records(app_data_dir: &Path) -> Vec<ProcessRecord> {
    let mut records = Vec::new();
    let dir = app_data_dir.join("pids");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return records;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("pid") {
            continue;
        }
        let session_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string);
        let status = match read_legacy_pid_value(&path) {
            Ok(pgid) => process_status_to_gc(process_group_status(pgid)),
            Err(error) => {
                log::warn!(
                    "app data gc skipped unreadable legacy pid record {}: {error}",
                    path.display()
                );
                ProcessRecordStatus::Unknown
            }
        };
        records.push(ProcessRecord {
            path,
            session_id,
            status,
        });
    }
    records
}

fn read_legacy_pid_value(path: &Path) -> Result<i32, String> {
    let content = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    content
        .trim()
        .parse::<i32>()
        .map_err(|error| error.to_string())
}

fn process_status_to_gc(status: ProcessStatus) -> ProcessRecordStatus {
    match status {
        ProcessStatus::Live => ProcessRecordStatus::Live,
        ProcessStatus::Stale => ProcessRecordStatus::Stale,
        ProcessStatus::Unknown => ProcessRecordStatus::Unknown,
    }
}

fn collect_workspace_state_records(app_data_dir: &Path) -> Vec<WorkspaceStateGcRecord> {
    let dir = app_data_dir.join("workspace_state");
    let Ok(entries) = std::fs::read_dir(&dir) else {
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

fn collect_review_comment_records(app_data_dir: &Path) -> Vec<ReviewCommentGcRecord> {
    let dir = app_data_dir.join("review-comments");
    let Ok(entries) = std::fs::read_dir(&dir) else {
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

fn collect_checkpoint_paths(app_data_dir: &Path) -> Vec<PathBuf> {
    [
        "agent-worktree-checkpoints",
        "agent-worktree-checkpoint-backups",
    ]
    .into_iter()
    .map(|dirname| app_data_dir.join(dirname))
    .filter(|path| path.exists())
    .collect()
}

fn collect_cache_records(app_data_dir: &Path) -> Vec<CacheGcRecord> {
    let mut records = Vec::new();
    let lsp_dir = app_data_dir.join("lsp");
    for relative in ["jdtls", "typescript", "jdtls.version"] {
        let path = lsp_dir.join(relative);
        push_cache_record(&path, &mut records);
    }
    let jdtls_workspaces = lsp_dir.join("jdtls-workspaces");
    if let Ok(entries) = std::fs::read_dir(&jdtls_workspaces) {
        for entry in entries.flatten() {
            push_cache_record(&entry.path(), &mut records);
        }
    }
    records
}

fn push_cache_record(path: &Path, records: &mut Vec<CacheGcRecord>) {
    if !path.exists() {
        return;
    }
    match latest_mtime_secs(path) {
        Ok(updated_at) => records.push(CacheGcRecord {
            path: path.to_path_buf(),
            updated_at,
        }),
        Err(error) => {
            log::warn!(
                "app data gc skipped cache entry {}: {error}",
                path.display()
            );
        }
    }
}

fn collect_legacy_comment_paths(app_data_dir: &Path) -> Vec<PathBuf> {
    ["comments", "diff-comments", "threads"]
        .into_iter()
        .map(|dirname| app_data_dir.join(dirname))
        .filter(|path| path.exists())
        .collect()
}

fn latest_mtime_secs(path: &Path) -> Result<f64, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let mut latest = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .map_err(|error| error.to_string())?;
    if metadata.file_type().is_dir() {
        for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
            latest = latest.max(latest_mtime_secs(
                &entry.map_err(|error| error.to_string())?.path(),
            )?);
        }
    }
    Ok(latest)
}

#[cfg(test)]
#[derive(Debug, Default)]
struct RunningWorkflowPathCollection {
    paths: Vec<String>,
    is_complete: bool,
}

#[cfg(test)]
fn collect_runtime_protection(
    app_data_dir: &Path,
    process_records: &[ProcessRecord],
) -> RuntimeProtection {
    let live_process_sessions = process_records
        .iter()
        .filter(|record| record.status == ProcessRecordStatus::Live)
        .filter_map(|record| record.session_id.as_deref())
        .collect::<HashSet<_>>();
    let mut active_session_ids = live_process_sessions
        .iter()
        .map(|session_id| (*session_id).to_string())
        .collect::<HashSet<_>>();
    let mut worktree_paths = Vec::new();
    let session_meta = FileSessionStorage::default().list_gc_session_protection_meta(app_data_dir);
    for meta in session_meta.items {
        let id_is_live = meta
            .id
            .as_deref()
            .is_some_and(|id| live_process_sessions.contains(id));
        let state_is_active = meta.state == Some(SessionState::Active);
        if id_is_live || state_is_active {
            if let Some(id) = meta.id {
                active_session_ids.insert(id);
            }
            if let Some(path) = meta.worktree_path {
                worktree_paths.push(path);
            }
        }
    }
    let running_workflows = collect_running_workflow_paths(app_data_dir);
    let running_workflows_complete = running_workflows.is_complete;
    let running_worktree_paths = running_workflows.paths;
    worktree_paths.extend(running_worktree_paths.iter().cloned());
    let protected_worktrees =
        LiveWorktreeSet::from_worktrees(worktree_paths.into_iter().map(|path| {
            let name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("worktree")
                .to_string();
            live_worktree(name, path)
        }));
    RuntimeProtection::new(
        active_session_ids,
        running_worktree_paths,
        protected_worktrees,
    )
    .with_workspace_keyed_protection_complete(
        session_meta.is_complete && running_workflows_complete,
    )
}

#[cfg(test)]
fn collect_running_workflow_paths(app_data_dir: &Path) -> RunningWorkflowPathCollection {
    let scan = WorkflowExecutionFileRepository::new(app_data_dir).scan_gc_metadata();
    let mut result = Vec::new();
    for execution in scan.executions {
        if !execution.status.is_terminal() {
            let normalized = normalize_path(&execution.worktree_path);
            let raw = execution.worktree_path;
            if raw != normalized {
                result.push(raw);
                result.push(normalized);
            } else {
                result.push(raw);
            }
        }
    }
    RunningWorkflowPathCollection {
        paths: result,
        is_complete: scan.is_complete,
    }
}

fn live_worktree(name: String, path: String) -> LiveWorktree {
    let normalized = normalize_path(&path);
    LiveWorktree {
        workspace_state_keys: workspace_state_keys(&name, &path),
        review_comment_keys: review_comment_keys(&path),
        path: normalized,
    }
}

fn workspace_state_keys(name: &str, path: &str) -> Vec<String> {
    let normalized = normalize_path(path);
    let mut keys = vec![
        crate::adaptor::gateway::workspace_state::storage_key(name),
        crate::adaptor::gateway::workspace_state::storage_key(path),
        crate::adaptor::gateway::workspace_state::storage_key(&normalized),
    ];
    if let Some(file_name) = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
    {
        keys.push(crate::adaptor::gateway::workspace_state::storage_key(
            file_name,
        ));
    }
    keys
}

fn workspace_state_key_prefixes(path: &str) -> HashSet<String> {
    let normalized = normalize_path(path);
    [
        crate::adaptor::gateway::workspace_state::storage_key(path),
        crate::adaptor::gateway::workspace_state::storage_key(&normalized),
    ]
    .into_iter()
    .collect()
}

fn review_comment_keys(path: &str) -> Vec<String> {
    let normalized = normalize_path(path);
    vec![
        crate::adaptor::gateway::comment::worktree_storage_key(path),
        crate::adaptor::gateway::comment::worktree_storage_key(&normalized),
    ]
}

fn normalize_path(path: &str) -> String {
    let trimmed = trim_path_separators(path.trim());
    Path::new(&trimmed)
        .canonicalize()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or(trimmed)
}

fn trim_path_separators(path: &str) -> String {
    let mut value = path.to_string();
    while value.len() > 1 && (value.ends_with('/') || value.ends_with('\\')) {
        value.pop();
    }
    value
}

pub(crate) struct StdGcRevalidationReader;

impl GcRevalidationReader for StdGcRevalidationReader {
    fn runtime_protection(
        &self,
        _app_data_dir: &Path,
        process_records: &[ProcessRecord],
    ) -> RuntimeProtection {
        collect_post_cutover_runtime_protection(process_records)
    }

    #[cfg(test)]
    fn session_state(
        &self,
        _app_data_dir: &Path,
        _session_id: &str,
    ) -> RevalidationRead<CurrentSessionState> {
        RevalidationRead::Unavailable(
            "canonical session GC revalidation is not configured".to_string(),
        )
    }

    #[cfg(test)]
    fn workflow_execution_state(
        &self,
        _app_data_dir: &Path,
        _execution_id: &str,
    ) -> RevalidationRead<CurrentWorkflowExecutionState> {
        RevalidationRead::Unavailable(
            "canonical workflow GC revalidation is not configured".to_string(),
        )
    }

    #[cfg(test)]
    fn workflow_execution_states(
        &self,
        _app_data_dir: &Path,
        execution_ids: &HashSet<String>,
    ) -> HashMap<String, RevalidationRead<CurrentWorkflowExecutionState>> {
        execution_ids
            .iter()
            .map(|execution_id| {
                (
                    execution_id.clone(),
                    RevalidationRead::Unavailable(
                        "canonical workflow GC revalidation is not configured".to_string(),
                    ),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::process::pid_registry::PidFileV1;
    use parking_lot::RwLock;
    use std::sync::Arc;

    #[test]
    fn resolve_live_worktrees_returns_none_when_no_repo_paths_are_known() {
        let shared: SharedRepoPaths = Arc::new(RwLock::new(Vec::new()));

        assert!(resolve_live_worktrees(shared).is_none());
    }

    #[test]
    fn resolve_live_worktrees_keeps_successful_repos_when_one_repo_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        git2::Repository::init(&repo_path).unwrap();
        let missing_repo = tmp.path().join("missing-repo");
        let shared: SharedRepoPaths = Arc::new(RwLock::new(vec![
            repo_path.to_string_lossy().into_owned(),
            missing_repo.to_string_lossy().into_owned(),
        ]));

        assert!(resolve_live_worktrees(shared).is_some());
    }

    #[test]
    fn runtime_protection_collects_active_sessions_live_processes_and_running_workflows() {
        let tmp = tempfile::tempdir().unwrap();
        let active_worktree = tempfile::tempdir().unwrap();
        let pid_worktree = tempfile::tempdir().unwrap();
        let running_worktree = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(sessions_dir.join("active-session")).unwrap();
        std::fs::write(
            sessions_dir.join("active-session/meta.json"),
            serde_json::json!({
                "worktreePath": active_worktree.path().to_string_lossy(),
                "state": "active"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(sessions_dir.join("pid-session")).unwrap();
        std::fs::write(
            sessions_dir.join("pid-session/meta.json"),
            serde_json::json!({
                "id": "pid-session",
                "worktreePath": pid_worktree.path().to_string_lossy(),
                "state": "archived"
            })
            .to_string(),
        )
        .unwrap();
        write_workflow_execution_metadata(
            tmp.path(),
            "00000000-0000-4000-8000-000000000001",
            running_worktree.path(),
            "running",
        )
        .unwrap();

        let protection = collect_runtime_protection(
            tmp.path(),
            &[ProcessRecord {
                path: tmp.path().join("agent-processes/pid-session.json"),
                session_id: Some("pid-session".to_string()),
                status: ProcessRecordStatus::Live,
            }],
        );

        assert!(protection.active_session_ids.contains("active-session"));
        assert!(protection.active_session_ids.contains("pid-session"));
        assert!(protection.running_worktrees.contains(
            &running_worktree
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ));
        assert!(protection.workspace_keyed_protection_complete);
    }

    #[test]
    fn runtime_protection_marks_incomplete_when_live_pid_session_meta_is_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("sessions/pid-session/meta.json")).unwrap();

        let protection = collect_runtime_protection(
            tmp.path(),
            &[ProcessRecord {
                path: tmp.path().join("agent-processes/pid-session.json"),
                session_id: Some("pid-session".to_string()),
                status: ProcessRecordStatus::Live,
            }],
        );

        assert!(protection.active_session_ids.contains("pid-session"));
        assert!(!protection.workspace_keyed_protection_complete);
    }

    #[test]
    fn runtime_protection_marks_incomplete_when_workflow_metadata_is_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("workflow_executions/running.json")).unwrap();

        let protection = collect_runtime_protection(tmp.path(), &[]);

        assert!(!protection.workspace_keyed_protection_complete);
    }

    #[test]
    fn collect_process_records_keeps_invalid_records_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agent-processes");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.json"), "{").unwrap();

        let records = collect_process_records(tmp.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ProcessRecordStatus::Unknown);
        assert_eq!(records[0].session_id, None);
    }

    #[test]
    fn json_process_record_uses_recorded_pid_liveness_not_owner_liveness() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agent-processes");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("session.codex.999999.json"),
            serde_json::to_string(&PidFileV1 {
                version: 1,
                session_id: "session".to_string(),
                backend_id: "codex".to_string(),
                pid: 999_999,
                pgid: 999_999,
                owner_app_pid: Some(std::process::id()),
                owner_start_time: None,
                created_at_ms: 1,
            })
            .unwrap(),
        )
        .unwrap();

        let records = collect_process_records(tmp.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id.as_deref(), Some("session"));
        assert_eq!(records[0].status, ProcessRecordStatus::Stale);
    }

    #[test]
    fn legacy_pid_file_is_collected_by_recorded_pgid_liveness() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pids");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session.pid"), "999999").unwrap();

        let records = collect_process_records(tmp.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id.as_deref(), Some("session"));
        assert_eq!(records[0].status, ProcessRecordStatus::Stale);
    }

    #[test]
    fn collect_workspace_state_records_keeps_only_json_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("workspace_state");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("repo.json"), "{}").unwrap();
        std::fs::write(dir.join("repo.tmp"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("nested.json")).unwrap();

        let records = collect_workspace_state_records(tmp.path());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "repo");
        assert_eq!(records[0].path, dir.join("repo.json"));
    }

    #[test]
    fn startup_gc_never_plans_or_deletes_legacy_migration_source_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        git2::Repository::init(repo.path()).unwrap();
        let deleted_worktree = repo.path().join("deleted-worktree");
        let session_dir = tmp.path().join("sessions/legacy-session");
        let session_meta = session_dir.join("meta.json");
        let session_message = session_dir.join("messages/00000000000000000001.json");
        std::fs::create_dir_all(session_message.parent().unwrap()).unwrap();
        std::fs::write(
            &session_meta,
            serde_json::json!({
                "id": "legacy-session",
                "worktreePath": deleted_worktree.to_string_lossy(),
                "state": "archived",
                "updatedAt": 0.0
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(&session_message, b"legacy-message-source").unwrap();

        let execution_id = "00000000-0000-4000-8000-000000000001";
        write_workflow_execution_metadata(tmp.path(), execution_id, &deleted_worktree, "completed")
            .unwrap();
        let workflow_meta = tmp
            .path()
            .join("workflow_executions")
            .join(format!("{execution_id}.json"));
        let workflow_log = tmp
            .path()
            .join("workflow_execution_logs")
            .join(format!("{execution_id}.ndjson"));
        std::fs::create_dir_all(workflow_log.parent().unwrap()).unwrap();
        std::fs::write(&workflow_log, b"legacy-workflow-source\n").unwrap();
        let source_files = [session_meta, session_message, workflow_meta, workflow_log];
        let before = source_files
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();

        let shared: SharedRepoPaths = Arc::new(RwLock::new(vec![repo
            .path()
            .to_string_lossy()
            .into_owned()]));
        let request = build_startup_gc_request(tmp.path().to_path_buf(), shared);
        assert!(request.session_records.is_empty());
        assert!(request.session_blob_stores.is_empty());
        assert!(request.workflow_executions.is_empty());

        let fs = StdGcFileSystem;
        let archive_pruner = StdWorkflowArchivePruner;
        let revalidation_reader = StdGcRevalidationReader;
        crate::usecase::app_data_gc::run_startup_gc(
            request,
            &fs,
            &archive_pruner,
            &revalidation_reader,
        );

        for (path, expected) in source_files.iter().zip(before) {
            assert_eq!(std::fs::read(path).unwrap(), expected);
        }
    }

    fn write_workflow_execution_metadata(
        app_data_dir: &std::path::Path,
        execution_id: &str,
        worktree_path: &std::path::Path,
        status: &str,
    ) -> std::io::Result<()> {
        let executions_dir = app_data_dir.join("workflow_executions");
        std::fs::create_dir_all(&executions_dir)?;
        std::fs::write(
            executions_dir.join(format!("{execution_id}.json")),
            serde_json::json!({
                "executionId": execution_id,
                "workflowName": "wf",
                "status": status,
                "worktreePath": worktree_path.to_string_lossy(),
                "createdFrom": "desktop_ui",
                "startedAt": 1.0,
                "updatedAt": 1.0
            })
            .to_string(),
        )
    }
}
