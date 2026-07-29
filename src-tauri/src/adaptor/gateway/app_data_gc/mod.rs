use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::adaptor::gateway::local_event_store::layout::{
    NoopStorePathObserver, StorePathObserver, StorePathOperation,
};
use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
use crate::domain::app_data_gc::RetentionPolicy;
use crate::infrastructure::process::pid_registry::{
    process_group_status, recorded_pid_status, PidFileV1, ProcessStatus,
};
use crate::usecase::app_data_gc::{
    CacheGcRecord, CanonicalRuntimeOwners, GcFileSystem, GcFileSystemError, GcFileType, GcMetadata,
    LiveWorktree, LiveWorktreeResolution, LiveWorktreeSet, ProcessRecord, ProcessRecordStatus,
    ReviewCommentGcRecord, RuntimeProtection, StartupGcRequest, WorkspaceStateGcRecord,
};

#[derive(Clone)]
pub(crate) struct StdGcFileSystem {
    observer: Arc<dyn StorePathObserver>,
}

impl Default for StdGcFileSystem {
    fn default() -> Self {
        Self::with_observer(Arc::new(NoopStorePathObserver))
    }
}

impl StdGcFileSystem {
    pub(crate) fn with_observer(observer: Arc<dyn StorePathObserver>) -> Self {
        Self { observer }
    }

    fn observe(&self, operation: StorePathOperation, path: &Path) {
        self.observer.observe(operation, path);
    }
}

impl GcFileSystem for StdGcFileSystem {
    fn metadata(&self, path: &Path) -> Result<GcMetadata, GcFileSystemError> {
        self.observe(StorePathOperation::Metadata, path);
        let metadata = std::fs::symlink_metadata(path).map_err(GcFileSystemError::from)?;
        let file_type = if metadata.file_type().is_symlink() {
            GcFileType::Symlink
        } else if metadata.file_type().is_file() {
            GcFileType::File
        } else if metadata.file_type().is_dir() {
            GcFileType::Directory
        } else {
            GcFileType::Other
        };
        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs_f64());
        Ok(GcMetadata {
            file_type,
            len: metadata.len(),
            modified_secs,
        })
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        self.observe(StorePathOperation::ReadDir, path);
        std::fs::read_dir(path)
            .map_err(GcFileSystemError::from)?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(GcFileSystemError::from)
            })
            .collect()
    }

    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        self.observe(StorePathOperation::Open, path);
        self.observe(StorePathOperation::Read, path);
        std::fs::read_to_string(path).map_err(GcFileSystemError::from)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        let metadata = match self.metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.is_not_found() => return Ok(false),
            Err(error) => return Err(error),
        };
        self.observe(StorePathOperation::Remove, path);
        if metadata.file_type == GcFileType::Directory {
            std::fs::remove_dir_all(path).map_err(GcFileSystemError::from)?;
        } else {
            std::fs::remove_file(path).map_err(GcFileSystemError::from)?;
        }
        Ok(true)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        let metadata = self.metadata(path)?;
        if metadata.file_type != GcFileType::Directory {
            return Ok(metadata.len);
        }
        self.read_dir(path)?
            .into_iter()
            .try_fold(0u64, |size, entry| {
                self.recursive_size(&entry)
                    .map(|entry_size| size.saturating_add(entry_size))
            })
    }
}

pub(crate) fn build_startup_gc_request(
    app_data_dir: PathBuf,
    shared_repo_paths: SharedRepoPaths,
    file_system: &dyn GcFileSystem,
) -> StartupGcRequest {
    let process_records = collect_process_records(&app_data_dir, file_system);
    let live_process_session_ids = process_records
        .iter()
        .filter(|record| record.status == ProcessRecordStatus::Live)
        .filter_map(|record| record.session_id.clone())
        .collect();
    StartupGcRequest {
        app_data_dir: app_data_dir.clone(),
        live_worktrees: resolve_live_worktrees(shared_repo_paths),
        workspace_state_records: collect_workspace_state_records(&app_data_dir, file_system),
        review_comment_records: collect_review_comment_records(&app_data_dir, file_system),
        checkpoint_paths: collect_checkpoint_paths(&app_data_dir, file_system),
        cache_records: collect_cache_records(&app_data_dir, file_system),
        legacy_comment_paths: collect_legacy_comment_paths(&app_data_dir, file_system),
        process_records,
        runtime_protection: RuntimeProtection::incomplete(live_process_session_ids),
        now_secs: crate::other::utils::unix_timestamp_seconds(),
        retention: RetentionPolicy::default(),
    }
}

pub(crate) fn apply_canonical_runtime_owners(
    request: &mut StartupGcRequest,
    owners: CanonicalRuntimeOwners,
) {
    request.runtime_protection = canonical_runtime_protection(owners);
}

pub(crate) fn canonical_runtime_protection(owners: CanonicalRuntimeOwners) -> RuntimeProtection {
    let protected_worktrees =
        LiveWorktreeSet::from_worktrees(owners.protected_worktree_paths.into_iter().map(|path| {
            let name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("worktree")
                .to_string();
            live_worktree(name, path)
        }));
    RuntimeProtection::complete(owners.active_session_ids, protected_worktrees)
}

pub(crate) fn live_process_session_ids(request: &StartupGcRequest) -> HashSet<String> {
    request
        .process_records
        .iter()
        .filter(|record| record.status == ProcessRecordStatus::Live)
        .filter_map(|record| record.session_id.clone())
        .collect()
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
                log::warn!(
                    "app data gc retained workspace-keyed data for unresolved repository {}: {error}",
                    repo_path
                );
                unresolved_workspace_state_key_prefixes
                    .extend(workspace_state_key_prefixes(&repo_path));
                unresolved_repo_paths.push(normalize_path(&repo_path));
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

fn collect_workspace_state_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<WorkspaceStateGcRecord> {
    let directory = app_data_dir.join("workspace_state");
    read_directory_or_empty(file_system, &directory)
        .into_iter()
        .filter_map(|path| {
            let metadata = file_system.metadata(&path).ok()?;
            if metadata.file_type != GcFileType::File
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                return None;
            }
            let key = path.file_stem().and_then(|stem| stem.to_str())?.to_string();
            Some(WorkspaceStateGcRecord { path, key })
        })
        .collect()
}

fn collect_review_comment_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<ReviewCommentGcRecord> {
    let directory = app_data_dir.join("review-comments");
    read_directory_or_empty(file_system, &directory)
        .into_iter()
        .filter_map(|path| {
            let metadata = file_system.metadata(&path).ok()?;
            if metadata.file_type != GcFileType::File {
                return None;
            }
            let name = path.file_name().and_then(|name| name.to_str())?;
            let key = name
                .strip_suffix(".events.json")
                .or_else(|| name.strip_suffix(".events.lock"))?
                .to_string();
            Some(ReviewCommentGcRecord { path, key })
        })
        .collect()
}

fn collect_checkpoint_paths(app_data_dir: &Path, file_system: &dyn GcFileSystem) -> Vec<PathBuf> {
    [
        "agent-worktree-checkpoints",
        "agent-worktree-checkpoint-backups",
    ]
    .into_iter()
    .map(|name| app_data_dir.join(name))
    .filter(|path| file_system.metadata(path).is_ok())
    .collect()
}

fn collect_cache_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<CacheGcRecord> {
    let mut records = Vec::new();
    let lsp = app_data_dir.join("lsp");
    for relative in ["jdtls", "typescript", "jdtls.version"] {
        push_cache_record(&lsp.join(relative), file_system, &mut records);
    }
    let workspaces = lsp.join("jdtls-workspaces");
    for path in read_directory_or_empty(file_system, &workspaces) {
        push_cache_record(&path, file_system, &mut records);
    }
    records
}

fn push_cache_record(
    path: &Path,
    file_system: &dyn GcFileSystem,
    records: &mut Vec<CacheGcRecord>,
) {
    match latest_mtime_secs(path, file_system) {
        Ok(updated_at) => records.push(CacheGcRecord {
            path: path.to_path_buf(),
            updated_at,
        }),
        Err(error) if error.is_not_found() => {}
        Err(error) => log::warn!(
            "app data gc retained cache entry {} because mtime was unavailable: {error}",
            path.display()
        ),
    }
}

fn latest_mtime_secs(
    path: &Path,
    file_system: &dyn GcFileSystem,
) -> Result<f64, GcFileSystemError> {
    let metadata = file_system.metadata(path)?;
    let mut latest = metadata.modified_secs.ok_or_else(|| {
        GcFileSystemError::other(format!("mtime is unavailable for {}", path.display()))
    })?;
    if metadata.file_type == GcFileType::Directory {
        for entry in file_system.read_dir(path)? {
            latest = latest.max(latest_mtime_secs(&entry, file_system)?);
        }
    }
    Ok(latest)
}

fn collect_legacy_comment_paths(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<PathBuf> {
    ["comments", "diff-comments", "threads"]
        .into_iter()
        .map(|name| app_data_dir.join(name))
        .filter(|path| file_system.metadata(path).is_ok())
        .collect()
}

fn collect_process_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<ProcessRecord> {
    let mut records = collect_agent_process_records(app_data_dir, file_system);
    records.extend(collect_pid_registry_records(app_data_dir, file_system));
    records
}

fn collect_agent_process_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<ProcessRecord> {
    let directory = app_data_dir.join("agent-processes");
    read_directory_or_empty(file_system, &directory)
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .map(|path| {
            let parsed = file_system
                .read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|content| parse_pid_file(&content));
            match parsed {
                Ok(pid_file) => ProcessRecord {
                    path,
                    session_id: Some(pid_file.session_id.clone()),
                    status: process_status_to_gc(recorded_pid_status(&pid_file)),
                },
                Err(error) => {
                    log::warn!(
                        "app data gc retained unreadable process record {}: {error}",
                        path.display()
                    );
                    ProcessRecord {
                        path,
                        session_id: None,
                        status: ProcessRecordStatus::Unknown,
                    }
                }
            }
        })
        .collect()
}

fn collect_pid_registry_records(
    app_data_dir: &Path,
    file_system: &dyn GcFileSystem,
) -> Vec<ProcessRecord> {
    let directory = app_data_dir.join("pids");
    read_directory_or_empty(file_system, &directory)
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("pid"))
        .map(|path| {
            let session_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string);
            let status = file_system
                .read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|content| {
                    content
                        .trim()
                        .parse::<i32>()
                        .map_err(|error| error.to_string())
                })
                .map(process_group_status)
                .map(process_status_to_gc)
                .unwrap_or_else(|error| {
                    log::warn!(
                        "app data gc retained unreadable pid record {}: {error}",
                        path.display()
                    );
                    ProcessRecordStatus::Unknown
                });
            ProcessRecord {
                path,
                session_id,
                status,
            }
        })
        .collect()
}

fn parse_pid_file(content: &str) -> Result<PidFileV1, String> {
    let file = serde_json::from_str::<PidFileV1>(content).map_err(|error| error.to_string())?;
    if file.version != 1 {
        return Err(format!("unsupported pid record version {}", file.version));
    }
    if file.pgid <= 1 {
        return Err(format!("unsafe process group {}", file.pgid));
    }
    Ok(file)
}

fn process_status_to_gc(status: ProcessStatus) -> ProcessRecordStatus {
    match status {
        ProcessStatus::Live => ProcessRecordStatus::Live,
        ProcessStatus::Stale => ProcessRecordStatus::Stale,
        ProcessStatus::Unknown => ProcessRecordStatus::Unknown,
    }
}

fn read_directory_or_empty(file_system: &dyn GcFileSystem, path: &Path) -> Vec<PathBuf> {
    match file_system.read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.is_not_found() => Vec::new(),
        Err(error) => {
            log::warn!(
                "app data gc retained entries under {} because enumeration failed: {error}",
                path.display()
            );
            Vec::new()
        }
    }
}

fn live_worktree(name: String, path: String) -> LiveWorktree {
    let normalized = normalize_path(&path);
    LiveWorktree {
        workspace_state_keys: workspace_state_keys(&name, &path, &normalized),
        review_comment_keys: review_comment_keys(&path, &normalized),
        path: normalized,
    }
}

fn workspace_state_keys(name: &str, path: &str, normalized: &str) -> Vec<String> {
    let mut keys = vec![
        crate::adaptor::gateway::workspace_state::repository_impl::storage_key(name),
        crate::adaptor::gateway::workspace_state::repository_impl::storage_key(path),
        crate::adaptor::gateway::workspace_state::repository_impl::storage_key(normalized),
    ];
    if let Some(file_name) = Path::new(normalized)
        .file_name()
        .and_then(|name| name.to_str())
    {
        keys.push(
            crate::adaptor::gateway::workspace_state::repository_impl::storage_key(file_name),
        );
    }
    keys
}

fn workspace_state_key_prefixes(path: &str) -> HashSet<String> {
    let normalized = normalize_path(path);
    [
        crate::adaptor::gateway::workspace_state::repository_impl::storage_key(path),
        crate::adaptor::gateway::workspace_state::repository_impl::storage_key(&normalized),
    ]
    .into_iter()
    .collect()
}

fn review_comment_keys(path: &str, normalized: &str) -> Vec<String> {
    vec![
        crate::adaptor::gateway::comment::worktree_storage_key(path),
        crate::adaptor::gateway::comment::worktree_storage_key(normalized),
    ]
}

fn normalize_path(path: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::app_data_gc::GcCategory;
    use crate::usecase::app_data_gc::run_startup_gc;
    use parking_lot::RwLock;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    #[derive(Default)]
    struct RecordingObserver {
        operations: Mutex<Vec<(StorePathOperation, PathBuf)>>,
    }

    impl StorePathObserver for RecordingObserver {
        fn observe(&self, operation: StorePathOperation, path: &Path) {
            self.operations
                .lock()
                .expect("recording observer")
                .push((operation, path.to_path_buf()));
        }
    }

    #[test]
    fn b070_background_gc_retention_and_restart_never_access_legacy_session_or_workflow_paths() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = app_data.path();
        let legacy_roots = [
            "sessions",
            "workflow_runs",
            "workflow_logs",
            "workflow_execution_logs",
            "workflow_executions",
            "workflow_event_logs",
        ];
        let sentinel = b"\0B-070 malformed legacy sentinel";
        for name in legacy_roots {
            let directory = root.join(name);
            std::fs::create_dir(&directory).expect("legacy root");
            std::fs::write(directory.join("sentinel.invalid"), sentinel).expect("legacy sentinel");
        }
        std::fs::write(root.join("session_titles.json"), sentinel).expect("title sentinel");
        let legacy_paths = [
            root.join("sessions"),
            root.join("session_titles.json"),
            root.join("workflow_runs"),
            root.join("workflow_logs"),
            root.join("workflow_execution_logs"),
            root.join("workflow_executions"),
            root.join("workflow_event_logs"),
        ];
        let before = legacy_paths
            .iter()
            .map(|path| {
                if path.is_dir() {
                    (
                        path.clone(),
                        std::fs::read(path.join("sentinel.invalid")).expect("before bytes"),
                        std::fs::metadata(path.join("sentinel.invalid"))
                            .expect("before metadata")
                            .modified()
                            .expect("before mtime"),
                    )
                } else {
                    (
                        path.clone(),
                        std::fs::read(path).expect("before bytes"),
                        std::fs::metadata(path)
                            .expect("before metadata")
                            .modified()
                            .expect("before mtime"),
                    )
                }
            })
            .collect::<Vec<_>>();

        let cache = root.join("lsp/typescript");
        std::fs::create_dir_all(&cache).expect("cache root");
        let cache_file = cache.join("cache.bin");
        std::fs::write(&cache_file, b"regenerable").expect("cache bytes");
        let old = filetime::FileTime::from_system_time(
            SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60),
        );
        filetime::set_file_mtime(&cache_file, old).expect("old cache file");
        filetime::set_file_mtime(&cache, old).expect("old cache directory");

        let checkpoints = root.join("agent-worktree-checkpoints");
        std::fs::create_dir_all(&checkpoints).expect("checkpoint root");
        std::fs::write(checkpoints.join("retained.checkpoint"), b"checkpoint").expect("checkpoint");

        let stale_process_dir = root.join("agent-processes");
        std::fs::create_dir_all(&stale_process_dir).expect("process root");
        let stale_process = stale_process_dir.join("stale.codex.999999.json");
        std::fs::write(
            &stale_process,
            serde_json::to_vec(&PidFileV1 {
                version: 1,
                session_id: "stale".to_string(),
                backend_id: "codex".to_string(),
                pid: 999_999,
                pgid: 999_999,
                owner_app_pid: None,
                owner_start_time: None,
                created_at_ms: 1,
            })
            .expect("pid json"),
        )
        .expect("stale process");

        let observer = Arc::new(RecordingObserver::default());
        let file_system = StdGcFileSystem::with_observer(observer.clone());
        let repos: SharedRepoPaths = Arc::new(RwLock::new(Vec::new()));
        for _ in 0..2 {
            let mut request =
                build_startup_gc_request(root.to_path_buf(), repos.clone(), &file_system);
            apply_canonical_runtime_owners(&mut request, CanonicalRuntimeOwners::default());
            let report = run_startup_gc(request, &file_system);
            assert_eq!(report.errors, 0);
        }

        assert!(
            !cache.exists(),
            "expired regenerable cache must be collected"
        );
        assert!(
            !stale_process.exists(),
            "stale process record must be collected"
        );
        assert!(
            checkpoints.join("retained.checkpoint").exists(),
            "unmapped checkpoint must be retained conservatively"
        );
        let observed = observer.operations.lock().expect("observed operations");
        assert!(
            observed.iter().all(|(operation, path)| {
                !(*operation == StorePathOperation::ReadDir && path == root)
                    && legacy_paths
                        .iter()
                        .all(|legacy| path != legacy && !path.starts_with(legacy))
            }),
            "GC accessed a B-070 legacy source or enumerated app-data root: {observed:?}"
        );
        drop(observed);
        for (path, bytes, modified) in before {
            let sentinel_path = if path.is_dir() {
                path.join("sentinel.invalid")
            } else {
                path
            };
            assert_eq!(std::fs::read(&sentinel_path).expect("after bytes"), bytes);
            assert_eq!(
                std::fs::metadata(&sentinel_path)
                    .expect("after metadata")
                    .modified()
                    .expect("after mtime"),
                modified
            );
        }
    }

    #[test]
    fn issue_1372_nonlegacy_gc_keeps_live_workspace_data_and_collects_deleted_workspace_data() {
        let app_data = tempfile::tempdir().expect("app data");
        let repo = tempfile::tempdir().expect("repo");
        git2::Repository::init(repo.path()).expect("init repo");
        let root = app_data.path();
        let live_name = repo
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("repo name");
        let live_workspace_key =
            crate::adaptor::gateway::workspace_state::repository_impl::storage_key(live_name);
        let live_review_key = crate::adaptor::gateway::comment::worktree_storage_key(
            repo.path().to_string_lossy().as_ref(),
        );
        let workspace_state = root.join("workspace_state");
        let review_comments = root.join("review-comments");
        std::fs::create_dir_all(&workspace_state).expect("workspace state");
        std::fs::create_dir_all(&review_comments).expect("review comments");
        let live_workspace = workspace_state.join(format!("{live_workspace_key}.json"));
        let stale_workspace = workspace_state.join("deleted-workspace.json");
        let live_review = review_comments.join(format!("{live_review_key}.events.json"));
        let stale_review = review_comments.join("deleted-workspace.events.json");
        for path in [
            &live_workspace,
            &stale_workspace,
            &live_review,
            &stale_review,
        ] {
            std::fs::write(path, b"{}").expect("fixture");
        }

        let file_system = StdGcFileSystem::default();
        let repos: SharedRepoPaths = Arc::new(RwLock::new(vec![repo
            .path()
            .to_string_lossy()
            .into_owned()]));
        let mut request = build_startup_gc_request(root.to_path_buf(), repos, &file_system);
        apply_canonical_runtime_owners(&mut request, CanonicalRuntimeOwners::default());
        let report = run_startup_gc(request, &file_system);

        assert!(live_workspace.exists());
        assert!(live_review.exists());
        assert!(!stale_workspace.exists());
        assert!(!stale_review.exists());
        assert_eq!(report.categories[&GcCategory::DeletedWorkspace].deleted, 2);
    }

    #[test]
    fn issue_1372_cache_boundary_and_legacy_comment_cleanup_remain_active() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = app_data.path();
        let exact_boundary = root.join("lsp/exact-boundary");
        let expired = root.join("lsp/expired");
        std::fs::create_dir_all(exact_boundary.parent().expect("lsp parent")).expect("lsp");
        std::fs::write(&exact_boundary, b"keep").expect("boundary cache");
        std::fs::write(&expired, b"delete").expect("expired cache");
        for name in ["comments", "diff-comments", "threads"] {
            let path = root.join(name);
            std::fs::create_dir(&path).expect("legacy comment root");
            std::fs::write(path.join("record"), b"legacy").expect("legacy comment");
        }
        let current_review = root.join("review-comments/current.events.json");
        std::fs::create_dir_all(current_review.parent().expect("review parent")).expect("review");
        std::fs::write(&current_review, b"[]").expect("current review");

        let file_system = StdGcFileSystem::default();
        let repos: SharedRepoPaths = Arc::new(RwLock::new(Vec::new()));
        let mut request = build_startup_gc_request(root.to_path_buf(), repos, &file_system);
        request.now_secs = request.retention.cache_secs as f64;
        request.cache_records = vec![
            CacheGcRecord {
                path: exact_boundary.clone(),
                updated_at: 0.0,
            },
            CacheGcRecord {
                path: expired.clone(),
                updated_at: -0.001,
            },
        ];
        let report = run_startup_gc(request, &file_system);

        assert!(exact_boundary.exists(), "exactly seven days is retained");
        assert!(!expired.exists(), "older than seven days is collected");
        for name in ["comments", "diff-comments", "threads"] {
            assert!(!root.join(name).exists());
        }
        assert!(current_review.exists());
        assert_eq!(report.categories[&GcCategory::RegenerableCache].deleted, 1);
        assert_eq!(report.categories[&GcCategory::LegacyComments].deleted, 3);
    }

    struct CompositionSendGate {
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAcceptancePort for CompositionSendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _operation_id: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            let allocation = self
                .session_store
                .send_acceptance_allocation("gc-protected-active-session")
                .expect("composition send allocation must be readable");
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: "gc-protected-active-session".to_string(),
                initial_session: None,
                session_projection_guard: allocation.session_projection_guard,
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: allocation.next_turn_id.to_string(),
                },
                input_ref: "b070-production-composition-input".to_string(),
                human_message_id: "b070-production-composition-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "B-070 production composition send".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
            })
        }

        async fn acceptance_state_mutations(
            &self,
            plan: &crate::usecase::agent_session::operation::SendPlan,
            events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
        ) -> Result<
            Vec<crate::domain::local_event::LocalStateMutation>,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.session_store
                .prepare_send_acceptance_mutations(
                    crate::usecase::agent_session::session::SendAcceptanceProjectionInput {
                        session_id: &plan.session_id,
                        initial_session: None,
                        session_projection_guard: plan.session_projection_guard,
                        human_message_id: &plan.human_message_id,
                        prompt: &plan.prompt,
                        disposition: &plan.disposition,
                        reserved_turn_id: plan.reserved_turn_id.as_deref(),
                        input_ref: &plan.input_ref,
                        events,
                    },
                )
                .map_err(|error| {
                    crate::domain::local_event::SafeOperationFailure::new(
                        crate::domain::local_event::SessionOperationFailureKind::PersistFailure,
                        false,
                        &error,
                        "b070-production-composition-send",
                    )
                })
        }

        async fn canonical_immediate_turn_is_current(
            &self,
            _session_id: &str,
            _turn_id: u64,
        ) -> Result<bool, crate::domain::local_event::SafeOperationFailure> {
            Ok(true)
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) -> Result<
            crate::usecase::agent_session::operation::SendEffectDispatch,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::usecase::agent_session::operation::SendEffectDispatch::Scheduled)
        }
    }

    #[tokio::test]
    async fn b070_production_composition_observer_covers_store_gc_shutdown_and_restart() {
        use crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1;
        use crate::domain::local_event::{
            ApplicationShutdownPhase, CommitIdentity, CommitOperationKind, IdempotencyBinding,
            LocalAtomicBatch, LocalEventQuery, LocalEventQueryResult,
            LocalEventTransactionRepository, LocalStateMutation, QuitIntent, Revision,
            RevisionGuard, ShutdownDetailsState, ShutdownLatestPointerMutation, ShutdownPlanKey,
            ShutdownPlanMutation, ShutdownPlanRecord,
        };
        use crate::usecase::agent_session::session::{build_new_session_with_id, SessionStore};

        let app_data = tempfile::tempdir().expect("app data");
        let root = app_data.path();
        let sentinel = b"\0B-070 production composition sentinel";
        let legacy_roots = [
            "sessions",
            "workflow_runs",
            "workflow_logs",
            "workflow_execution_logs",
            "workflow_executions",
            "workflow_event_logs",
        ];
        for name in legacy_roots {
            let directory = root.join(name);
            std::fs::create_dir(&directory).expect("legacy root");
            std::fs::write(directory.join("sentinel.invalid"), sentinel).expect("sentinel");
        }
        std::fs::write(root.join("session_titles.json"), sentinel).expect("title sentinel");
        let legacy_paths = [
            root.join("sessions"),
            root.join("session_titles.json"),
            root.join("workflow_runs"),
            root.join("workflow_logs"),
            root.join("workflow_execution_logs"),
            root.join("workflow_executions"),
            root.join("workflow_event_logs"),
        ];
        let before = legacy_paths
            .iter()
            .map(|path| {
                let metadata = std::fs::metadata(path).expect("legacy path metadata");
                let mut entries = if path.is_dir() {
                    std::fs::read_dir(path)
                        .expect("legacy entries")
                        .map(|entry| {
                            entry
                                .expect("legacy entry")
                                .file_name()
                                .to_string_lossy()
                                .into_owned()
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                entries.sort();
                let sentinel_path = if path.is_dir() {
                    path.join("sentinel.invalid")
                } else {
                    path.clone()
                };
                let sentinel_metadata =
                    std::fs::metadata(&sentinel_path).expect("legacy sentinel metadata");
                (
                    path.clone(),
                    metadata.len(),
                    metadata.modified().expect("legacy path mtime"),
                    entries,
                    std::fs::read(&sentinel_path).expect("legacy sentinel bytes"),
                    sentinel_metadata.len(),
                    sentinel_metadata.modified().expect("legacy sentinel mtime"),
                )
            })
            .collect::<Vec<_>>();

        let observer = Arc::new(RecordingObserver::default());
        let composition =
            crate::adaptor::controller::app_data_composition::ProductionAppDataComposition::with_observer(
                root.to_path_buf(),
                observer.clone(),
            );
        let registration = composition
            .register_process("observer-session", "observer-backend", 42)
            .expect("observed process registration");
        registration.remove();
        let malformed_process = root.join("agent-processes/malformed.json");
        std::fs::write(&malformed_process, b"{").expect("malformed process record");
        let process_cleanup = composition.cleanup_orphan_processes();
        assert_eq!(process_cleanup.failures, 0);
        let protected_worktree = "/removed-but-active-worktree";
        let protected_key = crate::adaptor::gateway::workspace_state::repository_impl::storage_key(
            protected_worktree,
        );
        let workspace_state = root.join("workspace_state");
        std::fs::create_dir_all(&workspace_state).expect("workspace state");
        let protected_state = workspace_state.join(format!("{protected_key}.json"));
        let stale_state = workspace_state.join("definitely-deleted.json");
        std::fs::write(&protected_state, b"{}").expect("protected state");
        std::fs::write(&stale_state, b"{}").expect("stale state");
        let expired_cache = root.join("lsp/typescript");
        std::fs::create_dir_all(&expired_cache).expect("cache directory");
        let expired_cache_file = expired_cache.join("cache.bin");
        std::fs::write(&expired_cache_file, b"regenerable cache").expect("cache fixture");
        let expired = filetime::FileTime::from_system_time(
            SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60),
        );
        filetime::set_file_mtime(&expired_cache_file, expired).expect("cache file mtime");
        filetime::set_file_mtime(&expired_cache, expired).expect("cache directory mtime");

        let live_repo = tempfile::tempdir().expect("live repo");
        git2::Repository::init(live_repo.path()).expect("init live repo");
        let repos: SharedRepoPaths = Arc::new(RwLock::new(vec![live_repo
            .path()
            .to_string_lossy()
            .into_owned()]));

        let store = composition
            .open_local_event_store()
            .expect("production composition cold startup");
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let session_store = Arc::new(SessionStore::new_canonical(
            repository.clone(),
            store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        ));
        let session = build_new_session_with_id(
            "gc-protected-active-session".to_string(),
            protected_worktree,
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session_store
            .save_full_session_from_user(root, &session)
            .expect("canonical active session");
        assert_eq!(
            session_store
                .get_session_meta(root, "gc-protected-active-session")
                .expect("production canonical session query")
                .expect("stored production session")
                .id,
            "gc-protected-active-session"
        );

        let send_gate = Arc::new(CompositionSendGate {
            session_store: session_store.clone(),
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let send_usecase = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
            store.clone(),
            store.clone(),
            send_gate.clone(),
            store.installation_id().to_string(),
        );
        let send = send_usecase
            .send(
                crate::usecase::agent_session::operation::SendOperationRequest {
                    principal: "desktop".to_string(),
                    operation_id: "b070-production-composition-send".to_string(),
                    canonical_payload: "{\"content\":\"B-070 production composition send\"}"
                        .to_string(),
                },
            )
            .await
            .expect("production composition normal send");
        assert!(matches!(
            send,
            crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(_)
        ));
        assert_eq!(
            send_usecase
                .get_operation("desktop", "b070-production-composition-send")
                .await
                .expect("production composition normal send query")
                .receipt
                .operation_id,
            "b070-production-composition-send"
        );
        assert_eq!(
            send_gate.effects.load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        let feedback_maintenance =
            crate::usecase::agent_session::feedback::SessionFeedbackUsecase::new(
                store.clone(),
                store.installation_id().to_string(),
            );
        assert_eq!(
            feedback_maintenance
                .recover_abandoned_reservations()
                .await
                .expect("production composition idle maintenance"),
            0
        );

        let report = composition
            .run_startup_gc_pass(repos.clone(), repository.clone())
            .await
            .expect("production composition background GC/retention");
        assert_eq!(report.errors, 0);
        assert!(
            protected_state.exists(),
            "canonical active owner must protect nonlegacy workspace state"
        );
        assert!(
            !stale_state.exists(),
            "unowned nonlegacy workspace state must still be collected"
        );
        assert!(!expired_cache.exists(), "expired cache must be collected");
        assert!(
            malformed_process.exists(),
            "unparseable process record must be retained fail closed"
        );

        let shutdown_key = ShutdownPlanKey {
            shutdown_id: "b070-production-composition-shutdown".to_string(),
        };
        store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse("b070-composition-shutdown-commit")
                    .expect("shutdown commit identity"),
                idempotency: IdempotencyBinding {
                    installation_id: store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::ApplicationQuit,
                    idempotency_key: "b070-composition-shutdown-key".to_string(),
                    payload_hash: [70; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![
                    LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                        key: shutdown_key.clone(),
                        phase: ApplicationShutdownPhase::Prepared,
                        summary: ShutdownPlanRecord {
                            operation_id: "b070-production-composition-quit".to_string(),
                            intent: QuitIntent::Exit { code: 0 },
                            t0_ms: 0,
                            preparation_cutoff_ms: None,
                            deadline_ms: 15_000,
                            target_count: None,
                            prepared_count: None,
                            effect_reserved_count: None,
                            terminal_count: None,
                            completed_count: None,
                            unresolved_count: None,
                            recovery_snapshot_count: None,
                            recovery_snapshot_id: None,
                            process_instance_id: "b070-composition-process".to_string(),
                            outcome: None,
                            failure: None,
                            shutdown_effect_count: None,
                            admission_open: None,
                            retry_quit_same_boot: None,
                        },
                        details_state: ShutdownDetailsState::Available,
                        expected: RevisionGuard::Absent,
                        revision: Revision::new(0).expect("shutdown revision"),
                    }),
                    LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                        expected: None,
                        new: Some(shutdown_key.clone()),
                    }),
                ],
            })
            .await
            .expect("production composition graceful shutdown persistence");
        assert_eq!(
            composition
                .run_startup_gc_pass(repos.clone(), repository.clone())
                .await
                .expect("cooperative shutdown maintenance")
                .errors,
            0
        );
        drop(feedback_maintenance);
        drop(send_usecase);
        drop(send_gate);
        drop(session_store);
        drop(repository);
        drop(store);

        let reopened = composition
            .open_local_event_store()
            .expect("production composition restart");
        let reopened_repository: Arc<dyn LocalEventTransactionRepository> = reopened.clone();
        assert!(matches!(
            reopened.query(LocalEventQuery::CurrentShutdown).await,
            Ok(LocalEventQueryResult::CurrentShutdown(Some(ref view)))
                if view.plan == shutdown_key
        ));
        let reopened_owners = crate::usecase::app_data_gc::load_canonical_runtime_owners(
            reopened_repository,
            HashSet::new(),
        )
        .await
        .expect("restart canonical runtime protection");
        assert!(reopened_owners
            .protected_worktree_paths
            .contains(protected_worktree));
        drop(reopened);

        let observed = observer.operations.lock().expect("observed operations");
        for required in [
            StorePathOperation::Open,
            StorePathOperation::Metadata,
            StorePathOperation::ReadDir,
            StorePathOperation::Read,
            StorePathOperation::Write,
            StorePathOperation::Rename,
            StorePathOperation::Remove,
        ] {
            assert!(
                observed.iter().any(|(operation, _)| *operation == required),
                "production composition observer did not cover {required:?}: {observed:?}"
            );
        }
        assert!(
            observed
                .iter()
                .any(|(_, path)| path.starts_with(root.join("agent-processes"))),
            "production composition did not observe the process-registry collaborator"
        );
        assert!(
            observed.iter().any(|(_, path)| {
                path.starts_with(root.join("workspace_state")) || path.starts_with(root.join("lsp"))
            }),
            "production composition did not observe the GC/retention collaborator"
        );
        assert!(
            observed
                .iter()
                .any(|(_, path)| path == &root.join("local-event-store.sqlite3")),
            "production composition did not observe the fixed SQLite collaborator"
        );
        assert!(
            observed.iter().all(|(operation, path)| {
                !(*operation == StorePathOperation::ReadDir && path == root)
                    && legacy_paths
                        .iter()
                        .all(|legacy| path != legacy && !path.starts_with(legacy))
            }),
            "production composition accessed a legacy path: {observed:?}"
        );
        drop(observed);
        for (path, path_len, path_modified, entries, bytes, sentinel_len, sentinel_modified) in
            before
        {
            let metadata = std::fs::metadata(&path).expect("unchanged legacy path metadata");
            assert_eq!(metadata.len(), path_len);
            assert_eq!(
                metadata.modified().expect("unchanged legacy path mtime"),
                path_modified
            );
            let mut after_entries = if path.is_dir() {
                std::fs::read_dir(&path)
                    .expect("unchanged legacy entries")
                    .map(|entry| {
                        entry
                            .expect("unchanged legacy entry")
                            .file_name()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            after_entries.sort();
            assert_eq!(after_entries, entries);
            let sentinel_path = if path.is_dir() {
                path.join("sentinel.invalid")
            } else {
                path
            };
            let metadata = std::fs::metadata(&sentinel_path).expect("unchanged sentinel metadata");
            assert_eq!(metadata.len(), sentinel_len);
            assert_eq!(
                metadata.modified().expect("unchanged sentinel mtime"),
                sentinel_modified
            );
            assert_eq!(
                std::fs::read(sentinel_path).expect("unchanged sentinel"),
                bytes
            );
        }
    }

    #[test]
    fn malformed_process_record_is_retained_fail_closed() {
        let app_data = tempfile::tempdir().expect("app data");
        let directory = app_data.path().join("agent-processes");
        std::fs::create_dir_all(&directory).expect("process directory");
        let malformed = directory.join("malformed.json");
        std::fs::write(&malformed, b"{").expect("malformed record");

        let records = collect_process_records(app_data.path(), &StdGcFileSystem::default());

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ProcessRecordStatus::Unknown);
        assert!(malformed.exists());
    }

    fn assert_test_only(source: &str, needle: &str, label: &str) {
        let offset = source
            .find(needle)
            .unwrap_or_else(|| panic!("{label} source contract target is missing: {needle}"));
        let prefix = &source[..offset];
        let nearest_cfg = prefix
            .rfind("#[cfg(")
            .unwrap_or_else(|| panic!("{label} is not guarded by cfg(test): {needle}"));
        let guard_to_item = &prefix[nearest_cfg..];
        assert!(
            guard_to_item.starts_with("#[cfg(test)]")
                && guard_to_item["#[cfg(test)]".len()..].trim().is_empty(),
            "{label} is not guarded by cfg(test): {needle}"
        );
    }

    #[test]
    fn b070_production_sources_exclude_every_legacy_store_adapter() {
        let gc_source = include_str!("mod.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production GC source");
        let composition_source = include_str!("../../controller/app_data_composition.rs");
        let production_root_source = include_str!("../../../lib.rs");
        // This module has small test-only wrappers interleaved with production
        // helpers, so scan the full source. Its tests also must never name a
        // forbidden Session/Workflow root.
        let process_source = include_str!("../../../infrastructure/process/pid_registry.rs");
        let workflow_module_source = include_str!("../workflow/mod.rs");
        let execution_store_source = include_str!("../workflow/execution_store.rs");
        let event_repository_source = include_str!("../workflow/event_repository.rs");
        let workflow_log_source = include_str!("../workflow/log.rs");
        let session_storage_source = include_str!("../agent_session/session_storage.rs");
        for legacy in [
            "sessions",
            "session_titles.json",
            "workflow_runs",
            "workflow_logs",
            "workflow_execution_logs",
            "workflow_executions",
            "workflow_event_logs",
        ] {
            let join = format!("join(\"{legacy}\")");
            assert!(
                !gc_source.contains(&join),
                "production GC reintroduced legacy root {legacy}"
            );
            assert!(
                !process_source.contains(&join),
                "process cleanup reintroduced legacy root {legacy}"
            );
        }
        assert!(
            !gc_source.contains("read_dir(app_data_dir)"),
            "production GC must not enumerate app-data root"
        );
        assert!(
            !process_source.contains("read_dir(data_dir)"),
            "process cleanup must not enumerate app-data root"
        );
        for required in [
            "ProductionAppDataComposition::new",
            "open_local_event_store(&app_data)",
            "spawn_startup_maintenance(app_data.clone()",
            "spawn_startup_app_data_gc(",
        ] {
            assert!(
                production_root_source.contains(required),
                "production application root bypassed the B-070 composition: {required}"
            );
        }
        for required in [
            "config.path_observer = self.observer.clone()",
            "cleanup_orphan_processes_with_observer",
            "StdGcFileSystem::with_observer(self.observer.clone())",
            "run_startup_gc_pass",
        ] {
            assert!(
                composition_source.contains(required),
                "production app-data collaborator bypassed the shared path observer: {required}"
            );
        }

        for (source, needle, label) in [
            (
                execution_store_source,
                "const EXECUTIONS_SUBDIR",
                "workflow execution file root",
            ),
            (
                execution_store_source,
                "async fn persist_metadata(",
                "workflow execution file writer",
            ),
            (
                execution_store_source,
                "pub async fn resolve_worktree_by_execution",
                "workflow execution reverse-lookup fallback",
            ),
            (
                event_repository_source,
                "Legacy(PathBuf)",
                "workflow event legacy repository",
            ),
            (
                workflow_log_source,
                "log_dir: PathBuf",
                "workflow legacy event log",
            ),
            (
                workflow_log_source,
                "pub fn new(data_dir: &Path)",
                "workflow legacy event log constructor",
            ),
            (
                session_storage_source,
                "mod layout;",
                "agent session legacy layout module",
            ),
            (
                session_storage_source,
                "pub struct FileSessionStorage",
                "agent session legacy storage",
            ),
        ] {
            assert_test_only(source, needle, label);
        }
        assert!(
            !workflow_module_source.contains("mod execution_repository;"),
            "workflow execution file repository must stay removed"
        );

        assert!(
            execution_store_source.contains("pub(crate) fn new_canonical("),
            "production workflow execution store must require canonical construction"
        );
        assert!(
            event_repository_source.contains("pub(crate) fn with_authority("),
            "production workflow event repository must require canonical authority"
        );
        assert!(
            workflow_log_source.contains("pub(crate) fn with_authority("),
            "production workflow event log must require canonical authority"
        );
    }
}
