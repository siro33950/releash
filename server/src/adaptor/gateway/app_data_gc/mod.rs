use std::collections::{HashMap, HashSet};
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
pub struct StdGcFileSystem {
    observer: Arc<dyn StorePathObserver>,
}

impl Default for StdGcFileSystem {
    fn default() -> Self {
        Self::with_observer(Arc::new(NoopStorePathObserver))
    }
}

impl StdGcFileSystem {
    pub fn with_observer(observer: Arc<dyn StorePathObserver>) -> Self {
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

pub fn build_startup_gc_request(
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
        now_secs: crate::infrastructure::utils::unix_timestamp_seconds(),
        retention: RetentionPolicy::default(),
    }
}

pub fn apply_canonical_runtime_owners(
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
    let mut worktree_repositories = HashMap::new();
    let mut unresolved_repo_paths = Vec::new();
    let mut unresolved_workspace_state_key_prefixes = HashSet::new();
    for repo_path in &repo_paths {
        let worktrees =
            match crate::adaptor::gateway::repository::worktree::registered_worktree_paths(
                repo_path,
            ) {
                Ok(worktrees) => worktrees,
                Err(error) => {
                    log::warn!(
                    "app data gc retained workspace-keyed data for unresolved repository {}: {error}",
                    repo_path
                );
                    unresolved_workspace_state_key_prefixes
                        .extend(workspace_state_key_prefixes(repo_path));
                    unresolved_repo_paths.push(normalize_path(repo_path));
                    continue;
                }
            };
        for (name, path) in worktrees {
            worktree_repositories.insert(normalize_path(&path), normalize_path(repo_path));
            live_worktrees.push(live_worktree(name, path));
        }
    }
    Some(
        LiveWorktreeResolution::new(
            LiveWorktreeSet::from_worktrees(live_worktrees),
            unresolved_repo_paths,
            unresolved_workspace_state_key_prefixes,
        )
        .with_repository_paths(repo_paths)
        .with_worktree_repositories(worktree_repositories),
    )
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
        path.replace(['/', '\\'], "_"),
        normalized.replace(['/', '\\'], "_"),
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
        crate::adaptor::gateway::workspace_state::repository_impl::ABSOLUTE_STATE_KEY_PREFIX
            .to_string(),
        path.replace(['/', '\\'], "_"),
        normalized.replace(['/', '\\'], "_"),
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
