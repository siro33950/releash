use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::domain::agent_session::{
    dedup_instructions, normalize_path_components, InstructionOrigin, ResolvedInstruction,
};

use super::stable_content_fingerprint;

pub(crate) const PROJECT_INSTRUCTION_FILENAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
const INSTRUCTION_RESOLUTION_CACHE_LIMIT: usize = 64;
const INSTRUCTION_FILE_READ_CACHE_LIMIT: usize = 4096;
const FILE_SYSTEM_INSTRUCTION_CACHE_KEY_PREFIX: &str = "file-system";

pub(crate) trait InstructionSourcePort {
    fn read_instruction_file(
        &self,
        path: &Path,
        worktree_root: &Path,
    ) -> Result<Option<String>, String>;

    fn instruction_cache_key(&self, _worktree_root: &Path) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstructionResolutionRequest {
    pub worktree_root: PathBuf,
    pub repo_context_dir: Option<PathBuf>,
    pub read_file_paths: Vec<PathBuf>,
    pub workflow_instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstructionResolution {
    pub instructions: Vec<ResolvedInstruction>,
    pub skipped_read_errors: usize,
}

impl InstructionResolution {
    pub fn payload(&self) -> Option<String> {
        let chunks = self
            .instructions
            .iter()
            .map(|instruction| instruction.content.trim())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();
        (!chunks.is_empty()).then(|| chunks.join("\n\n"))
    }
}

pub(crate) struct InstructionResolver<'a> {
    source: &'a dyn InstructionSourcePort,
}

impl<'a> InstructionResolver<'a> {
    pub fn new(source: &'a dyn InstructionSourcePort) -> Self {
        Self { source }
    }

    pub fn resolve(&self, request: &InstructionResolutionRequest) -> InstructionResolution {
        let plan = InstructionResolutionPlan::from_request(request);
        let source_cache_key = self.source.instruction_cache_key(&plan.worktree_root);
        if let Some(cached) = cached_instruction_resolution(request, source_cache_key.as_deref()) {
            return cached;
        }
        let file_reads = self.read_candidate_files(&plan, source_cache_key.as_deref());
        let mut collected = Vec::new();
        let mut skipped_read_errors = 0;

        for candidate in &plan.candidates {
            match file_reads.by_path.get(&candidate.path) {
                Some(InstructionFileRead::Present {
                    canonical_content,
                    fingerprint,
                }) => collected.push(ResolvedInstruction::new(
                    candidate.origin,
                    Some(candidate.path.clone()),
                    canonical_content.clone(),
                    fingerprint.clone(),
                    candidate.scope_depth,
                )),
                Some(InstructionFileRead::Failed) => skipped_read_errors += 1,
                Some(InstructionFileRead::Missing) | None => {}
            }
        }

        for (index, instruction) in request.workflow_instructions.iter().enumerate() {
            let canonical_content = instruction.trim();
            if canonical_content.is_empty() {
                continue;
            }
            collected.push(ResolvedInstruction::new(
                InstructionOrigin::WorkflowFacet,
                None,
                canonical_content.to_string(),
                stable_content_fingerprint(canonical_content),
                2000 + index,
            ));
        }

        let resolution = InstructionResolution {
            instructions: dedup_instructions(collected),
            skipped_read_errors,
        };
        cache_instruction_resolution(request, source_cache_key.as_deref(), &resolution);
        resolution
    }

    fn read_candidate_files(
        &self,
        plan: &InstructionResolutionPlan,
        source_cache_key: Option<&str>,
    ) -> InstructionFileReads {
        let mut by_path = BTreeMap::new();
        for path in plan.unique_candidate_paths() {
            if let Some(cached) =
                cached_instruction_file_read(source_cache_key, &plan.worktree_root, &path)
            {
                by_path.insert(path, cached);
                continue;
            }
            match self
                .source
                .read_instruction_file(&path, &plan.worktree_root)
            {
                Ok(Some(content)) if !content.trim().is_empty() => {
                    let canonical_content = content.trim().to_string();
                    let read = InstructionFileRead::Present {
                        fingerprint: stable_content_fingerprint(&canonical_content),
                        canonical_content,
                    };
                    cache_instruction_file_read(
                        source_cache_key,
                        &plan.worktree_root,
                        &path,
                        &read,
                    );
                    by_path.insert(path, read);
                }
                Ok(_) => {
                    let read = InstructionFileRead::Missing;
                    cache_instruction_file_read(
                        source_cache_key,
                        &plan.worktree_root,
                        &path,
                        &read,
                    );
                    by_path.insert(path, read);
                }
                Err(err) => {
                    log::warn!(
                        "Failed to read project instruction file {}: {err}",
                        path.display()
                    );
                    by_path.insert(path, InstructionFileRead::Failed);
                }
            }
        }
        InstructionFileReads { by_path }
    }
}

fn normalize_dir_input(path: &Path, worktree_root: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    };
    normalize_path_components(&path)
}

fn normalize_file_parent(path: &Path, worktree_root: &Path) -> PathBuf {
    let path = normalize_dir_input(path, worktree_root);
    normalize_path_components(path.parent().unwrap_or(&path))
}

fn is_path_within_root(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn path_chain(root: &Path, target: &Path) -> Vec<PathBuf> {
    if !is_path_within_root(target, root) {
        return vec![root.to_path_buf()];
    }
    let mut chain = vec![root.to_path_buf()];
    let Ok(relative) = target.strip_prefix(root) else {
        return chain;
    };
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        chain.push(cursor.clone());
    }
    chain
}

#[derive(Debug, Clone)]
struct InstructionResolutionPlan {
    worktree_root: PathBuf,
    candidates: Vec<InstructionFileCandidate>,
}

impl InstructionResolutionPlan {
    fn from_request(request: &InstructionResolutionRequest) -> Self {
        let worktree_root = normalize_path_components(&request.worktree_root);
        let mut candidates = Vec::new();
        let repo_dir = request
            .repo_context_dir
            .as_ref()
            .map(|path| normalize_dir_input(path, &worktree_root))
            .filter(|path| is_path_within_root(path, &worktree_root))
            .unwrap_or_else(|| worktree_root.clone());

        for (depth, dir) in path_chain(&worktree_root, &repo_dir)
            .into_iter()
            .enumerate()
        {
            push_dir_candidates(
                &mut candidates,
                &dir,
                InstructionOrigin::RepoHierarchy,
                depth,
            );
        }

        let mut neighbor_dirs = BTreeSet::new();
        for file_path in &request.read_file_paths {
            let dir = normalize_file_parent(file_path, &worktree_root);
            if is_path_within_root(&dir, &worktree_root) {
                neighbor_dirs.insert(dir);
            }
        }
        for dir in neighbor_dirs {
            for (depth, chain_dir) in path_chain(&worktree_root, &dir).into_iter().enumerate() {
                push_dir_candidates(
                    &mut candidates,
                    &chain_dir,
                    InstructionOrigin::FileNeighbor,
                    depth + 1000,
                );
            }
        }

        Self {
            worktree_root,
            candidates,
        }
    }

    fn unique_candidate_paths(&self) -> BTreeSet<PathBuf> {
        self.candidates
            .iter()
            .map(|candidate| candidate.path.clone())
            .collect()
    }
}

fn push_dir_candidates(
    candidates: &mut Vec<InstructionFileCandidate>,
    dir: &Path,
    origin: InstructionOrigin,
    scope_depth: usize,
) {
    for filename in PROJECT_INSTRUCTION_FILENAMES {
        candidates.push(InstructionFileCandidate {
            path: dir.join(filename),
            origin,
            scope_depth,
        });
    }
}

#[derive(Debug, Clone)]
struct InstructionFileCandidate {
    path: PathBuf,
    origin: InstructionOrigin,
    scope_depth: usize,
}

#[derive(Debug, Clone)]
struct InstructionFileReads {
    by_path: BTreeMap<PathBuf, InstructionFileRead>,
}

#[derive(Debug, Clone)]
enum InstructionFileRead {
    Present {
        canonical_content: String,
        fingerprint: String,
    },
    Missing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstructionFileReadCacheKey {
    source_cache_key: String,
    worktree_root: PathBuf,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstructionResolutionCacheKey {
    source_cache_key: String,
    worktree_root: PathBuf,
    repo_context_dir: Option<PathBuf>,
    read_file_paths: Vec<PathBuf>,
    workflow_instructions: Vec<String>,
}

impl InstructionResolutionCacheKey {
    fn from_request(request: &InstructionResolutionRequest, source_cache_key: &str) -> Self {
        let worktree_root = normalize_path_components(&request.worktree_root);
        let repo_context_dir = request
            .repo_context_dir
            .as_ref()
            .map(|path| normalize_dir_input(path, &worktree_root));
        let mut read_file_paths = request
            .read_file_paths
            .iter()
            .map(|path| normalize_dir_input(path, &worktree_root))
            .collect::<Vec<_>>();
        read_file_paths.sort();
        read_file_paths.dedup();
        let workflow_instructions = request
            .workflow_instructions
            .iter()
            .map(|instruction| instruction.trim().to_string())
            .filter(|instruction| !instruction.is_empty())
            .collect();
        Self {
            source_cache_key: source_cache_key.to_string(),
            worktree_root,
            repo_context_dir,
            read_file_paths,
            workflow_instructions,
        }
    }
}

#[derive(Default)]
struct InstructionResolutionCache {
    entries: Vec<(InstructionResolutionCacheKey, InstructionResolution)>,
}

impl InstructionResolutionCache {
    fn get(&mut self, key: &InstructionResolutionCacheKey) -> Option<InstructionResolution> {
        let index = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == key)?;
        let (key, value) = self.entries.remove(index);
        let cached = value.clone();
        self.entries.push((key, value));
        Some(cached)
    }

    fn insert(&mut self, key: InstructionResolutionCacheKey, value: InstructionResolution) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == &key)
        {
            self.entries.remove(index);
        }
        self.entries.push((key, value));
        while self.entries.len() > INSTRUCTION_RESOLUTION_CACHE_LIMIT {
            self.entries.remove(0);
        }
    }

    fn remove_source_cache_key_prefix(&mut self, prefix: &str) {
        self.entries
            .retain(|(key, _)| !key.source_cache_key.starts_with(prefix));
    }
}

#[derive(Default)]
struct InstructionFileReadCache {
    entries: Vec<(InstructionFileReadCacheKey, InstructionFileRead)>,
}

impl InstructionFileReadCache {
    fn get(&mut self, key: &InstructionFileReadCacheKey) -> Option<InstructionFileRead> {
        let index = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == key)?;
        let (key, value) = self.entries.remove(index);
        let cached = value.clone();
        self.entries.push((key, value));
        Some(cached)
    }

    fn insert(&mut self, key: InstructionFileReadCacheKey, value: InstructionFileRead) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == &key)
        {
            self.entries.remove(index);
        }
        self.entries.push((key, value));
        while self.entries.len() > INSTRUCTION_FILE_READ_CACHE_LIMIT {
            self.entries.remove(0);
        }
    }

    fn remove_source_cache_key_prefix(&mut self, prefix: &str) {
        self.entries
            .retain(|(key, _)| !key.source_cache_key.starts_with(prefix));
    }
}

static INSTRUCTION_RESOLUTION_CACHE: OnceLock<parking_lot::Mutex<InstructionResolutionCache>> =
    OnceLock::new();
static INSTRUCTION_FILE_READ_CACHE: OnceLock<parking_lot::Mutex<InstructionFileReadCache>> =
    OnceLock::new();
static FILE_SYSTEM_INSTRUCTION_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn instruction_resolution_cache() -> &'static parking_lot::Mutex<InstructionResolutionCache> {
    INSTRUCTION_RESOLUTION_CACHE.get_or_init(Default::default)
}

fn instruction_file_read_cache() -> &'static parking_lot::Mutex<InstructionFileReadCache> {
    INSTRUCTION_FILE_READ_CACHE.get_or_init(Default::default)
}

pub(crate) fn file_system_instruction_cache_key() -> String {
    format!(
        "{FILE_SYSTEM_INSTRUCTION_CACHE_KEY_PREFIX}:{}",
        FILE_SYSTEM_INSTRUCTION_CACHE_GENERATION.load(Ordering::SeqCst)
    )
}

pub(crate) fn invalidate_instruction_resolution_cache_for_path(path: &Path) {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if !PROJECT_INSTRUCTION_FILENAMES.contains(&filename) {
        return;
    }
    FILE_SYSTEM_INSTRUCTION_CACHE_GENERATION.fetch_add(1, Ordering::SeqCst);
    instruction_resolution_cache()
        .lock()
        .remove_source_cache_key_prefix(FILE_SYSTEM_INSTRUCTION_CACHE_KEY_PREFIX);
    instruction_file_read_cache()
        .lock()
        .remove_source_cache_key_prefix(FILE_SYSTEM_INSTRUCTION_CACHE_KEY_PREFIX);
}

fn cached_instruction_resolution(
    request: &InstructionResolutionRequest,
    source_cache_key: Option<&str>,
) -> Option<InstructionResolution> {
    let source_cache_key = source_cache_key?;
    let key = InstructionResolutionCacheKey::from_request(request, source_cache_key);
    instruction_resolution_cache().lock().get(&key)
}

fn cache_instruction_resolution(
    request: &InstructionResolutionRequest,
    source_cache_key: Option<&str>,
    resolution: &InstructionResolution,
) {
    let Some(source_cache_key) = source_cache_key else {
        return;
    };
    if resolution.skipped_read_errors > 0 {
        return;
    }
    let key = InstructionResolutionCacheKey::from_request(request, source_cache_key);
    instruction_resolution_cache()
        .lock()
        .insert(key, resolution.clone());
}

fn cached_instruction_file_read(
    source_cache_key: Option<&str>,
    worktree_root: &Path,
    path: &Path,
) -> Option<InstructionFileRead> {
    let source_cache_key = source_cache_key?;
    let key = InstructionFileReadCacheKey {
        source_cache_key: source_cache_key.to_string(),
        worktree_root: worktree_root.to_path_buf(),
        path: path.to_path_buf(),
    };
    instruction_file_read_cache().lock().get(&key)
}

fn cache_instruction_file_read(
    source_cache_key: Option<&str>,
    worktree_root: &Path,
    path: &Path,
    read: &InstructionFileRead,
) {
    let Some(source_cache_key) = source_cache_key else {
        return;
    };
    if matches!(read, InstructionFileRead::Failed) {
        return;
    }
    let key = InstructionFileReadCacheKey {
        source_cache_key: source_cache_key.to_string(),
        worktree_root: worktree_root.to_path_buf(),
        path: path.to_path_buf(),
    };
    instruction_file_read_cache()
        .lock()
        .insert(key, read.clone());
}

#[cfg(test)]
fn clear_instruction_resolution_cache_for_test() {
    instruction_resolution_cache().lock().entries.clear();
    instruction_file_read_cache().lock().entries.clear();
    FILE_SYSTEM_INSTRUCTION_CACHE_GENERATION.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::OnceLock;

    static INSTRUCTION_RESOLUTION_TEST_LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();

    fn reset_instruction_resolution_cache_for_test() -> parking_lot::MutexGuard<'static, ()> {
        let guard = INSTRUCTION_RESOLUTION_TEST_LOCK
            .get_or_init(|| parking_lot::Mutex::new(()))
            .lock();
        clear_instruction_resolution_cache_for_test();
        guard
    }

    #[derive(Default)]
    struct FakeInstructionSource {
        files: HashMap<PathBuf, String>,
        failures: HashSet<PathBuf>,
        read_count: std::sync::atomic::AtomicUsize,
    }

    impl FakeInstructionSource {
        fn with_file_path(mut self, path: &Path, content: &str) -> Self {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
            self.files.insert(path.to_path_buf(), content.to_string());
            self
        }

        fn with_failure_path(mut self, path: &Path) -> Self {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, "unreadable").unwrap();
            self.failures.insert(path.to_path_buf());
            self
        }

        fn read_count(&self) -> usize {
            self.read_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl InstructionSourcePort for FakeInstructionSource {
        fn read_instruction_file(
            &self,
            path: &Path,
            _worktree_root: &Path,
        ) -> Result<Option<String>, String> {
            self.read_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.failures.contains(path) {
                return Err("permission denied".to_string());
            }
            Ok(self.files.get(path).cloned())
        }

        fn instruction_cache_key(&self, _worktree_root: &Path) -> Option<String> {
            Some(format!("fake:{:p}", self))
        }
    }

    #[test]
    fn resolves_repo_hierarchy_and_file_neighbor_inside_worktree_only() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let outside = temp.path().join("outside");
        let source = FakeInstructionSource::default()
            .with_file_path(&repo.join("AGENTS.md"), "root")
            .with_file_path(&repo.join("src/CLAUDE.md"), "src")
            .with_file_path(&repo.join("src/deep/AGENTS.md"), "deep")
            .with_file_path(&outside.join("AGENTS.md"), "outside");
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: Some(repo.join("src")),
            read_file_paths: vec![repo.join("src/deep/file.rs"), outside.join("file.rs")],
            workflow_instructions: Vec::new(),
        });

        assert_eq!(result.skipped_read_errors, 0);
        assert_eq!(
            result
                .instructions
                .iter()
                .map(|instruction| instruction.content.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "src", "deep"]
        );
    }

    #[test]
    fn dedups_repo_file_neighbor_and_workflow_instruction_content() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let source = FakeInstructionSource::default()
            .with_file_path(&repo.join("AGENTS.md"), "same")
            .with_file_path(&repo.join("src/AGENTS.md"), "local");
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: Some(repo.join("src")),
            read_file_paths: vec![repo.join("src/file.rs")],
            workflow_instructions: vec!["local".to_string()],
        });

        assert_eq!(
            result
                .instructions
                .iter()
                .map(|instruction| instruction.content.as_str())
                .collect::<Vec<_>>(),
            vec!["same", "local"]
        );
    }

    #[test]
    fn read_errors_are_skipped_without_dropping_other_instructions() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let source = FakeInstructionSource::default()
            .with_file_path(&repo.join("AGENTS.md"), "root")
            .with_failure_path(&repo.join("CLAUDE.md"))
            .with_file_path(&repo.join("src/AGENTS.md"), "src");
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: Some(repo.join("src")),
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        });

        assert_eq!(result.skipped_read_errors, 1);
        assert_eq!(result.payload().as_deref(), Some("root\n\nsrc"));
    }

    #[test]
    fn dedups_repo_and_workflow_instruction_after_trim_normalization() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let source =
            FakeInstructionSource::default().with_file_path(&repo.join("AGENTS.md"), "Use Rust.\n");
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo,
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: vec!["Use Rust.".to_string()],
        });

        assert_eq!(result.payload().as_deref(), Some("Use Rust."));
        assert_eq!(result.instructions.len(), 1);
    }

    #[test]
    fn repeated_resolution_for_same_read_paths_validates_cache_with_distinct_candidate_paths() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let source = FakeInstructionSource::default()
            .with_file_path(&repo.join("AGENTS.md"), "root")
            .with_file_path(&repo.join("src/AGENTS.md"), "src");
        let resolver = InstructionResolver::new(&source);
        let request = InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: Some(repo.join("src")),
            read_file_paths: vec![repo.join("src/file.rs")],
            workflow_instructions: Vec::new(),
        };

        let first = resolver.resolve(&request);
        let expected_reads_per_resolution = InstructionResolutionPlan::from_request(&request)
            .unique_candidate_paths()
            .len();
        let read_count_after_first = source.read_count();
        let second = resolver.resolve(&request);

        assert_eq!(first, second);
        assert_eq!(read_count_after_first, expected_reads_per_resolution);
        assert_eq!(
            source.read_count() - read_count_after_first,
            0,
            "same read-path set should reuse cached candidate reads without re-reading every file"
        );
    }

    #[test]
    fn growing_read_paths_only_reads_new_candidate_paths() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let source = FakeInstructionSource::default()
            .with_file_path(&repo.join("AGENTS.md"), "root")
            .with_file_path(&repo.join("src/AGENTS.md"), "src")
            .with_file_path(&repo.join("src/a/AGENTS.md"), "a")
            .with_file_path(&repo.join("src/b/AGENTS.md"), "b");
        let resolver = InstructionResolver::new(&source);
        let first_request = InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: None,
            read_file_paths: vec![repo.join("src/a/file.rs")],
            workflow_instructions: Vec::new(),
        };
        let second_request = InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: None,
            read_file_paths: vec![repo.join("src/a/file.rs"), repo.join("src/b/file.rs")],
            workflow_instructions: Vec::new(),
        };

        let first = resolver.resolve(&first_request);
        let read_count_after_first = source.read_count();
        let second = resolver.resolve(&second_request);
        let first_paths =
            InstructionResolutionPlan::from_request(&first_request).unique_candidate_paths();
        let second_paths =
            InstructionResolutionPlan::from_request(&second_request).unique_candidate_paths();
        let newly_needed_reads = second_paths.difference(&first_paths).count();

        assert_eq!(first.payload().as_deref(), Some("root\n\nsrc\n\na"));
        assert_eq!(second.payload().as_deref(), Some("root\n\nsrc\n\na\n\nb"));
        assert_eq!(
            source.read_count() - read_count_after_first,
            newly_needed_reads
        );
        assert_eq!(newly_needed_reads, 2);
    }

    #[test]
    fn cached_resolution_is_invalidated_when_instruction_content_changes() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let request = InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        };
        let source_v1 =
            FakeInstructionSource::default().with_file_path(&repo.join("AGENTS.md"), "root-v1");
        let first = InstructionResolver::new(&source_v1).resolve(&request);

        let source_v2 =
            FakeInstructionSource::default().with_file_path(&repo.join("AGENTS.md"), "root-v2");
        let second = InstructionResolver::new(&source_v2).resolve(&request);

        assert_eq!(first.payload().as_deref(), Some("root-v1"));
        assert_eq!(second.payload().as_deref(), Some("root-v2"));
    }

    #[test]
    fn cached_empty_resolution_is_invalidated_when_instruction_file_appears() {
        let _cache_guard = reset_instruction_resolution_cache_for_test();
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let request = InstructionResolutionRequest {
            worktree_root: repo.clone(),
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        };
        let empty_source = FakeInstructionSource::default();
        let first = InstructionResolver::new(&empty_source).resolve(&request);

        let populated_source =
            FakeInstructionSource::default().with_file_path(&repo.join("AGENTS.md"), "root");
        let second = InstructionResolver::new(&populated_source).resolve(&request);

        assert_eq!(first.payload(), None);
        assert_eq!(second.payload().as_deref(), Some("root"));
    }
}
