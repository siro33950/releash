use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::adaptor::gateway::local_event_store::layout::{
    NoopStorePathObserver, StorePathObserver, StorePathOperation,
};
use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
use crate::domain::app_data_gc::RetentionPolicy;
use crate::usecase::app_data_gc::{
    CacheGcRecord, CanonicalRuntimeOwners, GcFileSystem, GcFileSystemError, GcFileType, GcMetadata,
    LiveWorktree, LiveWorktreeResolution, LiveWorktreeSet, ReviewCommentGcRecord,
    RuntimeProtection, StartupGcRequest, WorkspaceStateGcRecord,
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
    StartupGcRequest {
        app_data_dir: app_data_dir.clone(),
        live_worktrees: resolve_live_worktrees(shared_repo_paths),
        workspace_state_records: collect_workspace_state_records(&app_data_dir, file_system),
        review_comment_records: collect_review_comment_records(&app_data_dir, file_system),
        checkpoint_paths: collect_checkpoint_paths(&app_data_dir, file_system),
        cache_records: collect_cache_records(&app_data_dir, file_system),
        legacy_comment_paths: collect_legacy_comment_paths(&app_data_dir, file_system),
        runtime_protection: RuntimeProtection::incomplete(),
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
    RuntimeProtection::complete(protected_worktrees)
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

        let legacy_process_dir = root.join("agent-processes");
        std::fs::create_dir_all(&legacy_process_dir).expect("process root");
        let legacy_process = legacy_process_dir.join("stale.codex.999999.json");
        std::fs::write(&legacy_process, b"legacy process record").expect("legacy process record");

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
            legacy_process.exists(),
            "legacy Agent process data must not be deleted"
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
        let workflow_module_source = include_str!("../workflow/mod.rs");
        let execution_store_source = include_str!("../workflow/execution_store.rs");
        let event_repository_source = include_str!("../workflow/event_repository.rs");
        let workflow_log_source = include_str!("../workflow/log.rs");
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
        }
        assert!(
            !gc_source.contains("read_dir(app_data_dir)"),
            "production GC must not enumerate app-data root"
        );
        for required in [
            "ProductionAppDataComposition::new",
            "open_local_event_store(&app_data)",
            "spawn_startup_app_data_gc(",
        ] {
            assert!(
                production_root_source.contains(required),
                "production application root bypassed the B-070 composition: {required}"
            );
        }
        for required in [
            "config.path_observer = self.observer.clone()",
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
