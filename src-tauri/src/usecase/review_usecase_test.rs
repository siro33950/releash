pub(crate) mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::super::*;
    use crate::usecase::code_dto::{ChangedFileDto, DiffStatsDto};
    use crate::usecase::repository_state::snapshot::SnapshotFlags;

    struct FakeSnapshotProvider {
        snapshots: Mutex<VecDeque<Arc<RepositorySnapshot>>>,
    }

    impl FakeSnapshotProvider {
        fn new(snapshots: Vec<RepositorySnapshot>) -> Self {
            Self {
                snapshots: Mutex::new(snapshots.into_iter().map(Arc::new).collect()),
            }
        }
    }

    impl ReviewSnapshotProvider for FakeSnapshotProvider {
        fn snapshot(
            &self,
            _worktree_path: &str,
        ) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
            self.snapshots.lock().unwrap().pop_front().ok_or_else(|| {
                CodeUsecaseError::from(CodeError::Rule("snapshot queue exhausted".to_string()))
            })
        }
    }

    struct FakeReviewCode {
        calls: Mutex<Vec<String>>,
        branch_summary: BranchDiffSummaryDto,
        source_metadata: HashMap<(String, &'static str), ReviewSideMetadata>,
        source_bytes: HashMap<(String, &'static str), ReviewSideBytes>,
        source_byte_sequences: Mutex<HashMap<(String, &'static str), VecDeque<ReviewSideBytes>>>,
        binary_attributes: HashMap<String, bool>,
        hunk_count_by_path: HashMap<String, usize>,
        hunk_indexes_by_path: HashMap<String, Vec<u32>>,
        change_groups_by_path: HashMap<String, Vec<ChangeGroupDto>>,
        real_diff: bool,
    }

    impl FakeReviewCode {
        fn new() -> Self {
            Self::with_branch_files(Vec::new())
        }

        fn with_branch_files(changed_files: Vec<ChangedFileDto>) -> Self {
            let additions = changed_files.iter().map(|file| file.stats.additions).sum();
            let deletions = changed_files.iter().map(|file| file.stats.deletions).sum();
            Self {
                calls: Mutex::new(Vec::new()),
                branch_summary: BranchDiffSummaryDto {
                    base_branch: "main".to_string(),
                    changed_files,
                    stats: DiffStatsDto {
                        additions,
                        deletions,
                    },
                },
                source_metadata: HashMap::new(),
                source_bytes: HashMap::new(),
                source_byte_sequences: Mutex::new(HashMap::new()),
                binary_attributes: HashMap::new(),
                hunk_count_by_path: HashMap::new(),
                hunk_indexes_by_path: HashMap::new(),
                change_groups_by_path: HashMap::new(),
                real_diff: false,
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn with_source_bytes(
            mut self,
            file_path: &str,
            source: ReviewContentSource,
            bytes: ReviewSideBytes,
        ) -> Self {
            self.source_bytes
                .insert((file_path.to_string(), review_source_key(source)), bytes);
            self
        }

        fn with_source_byte_sequence(
            mut self,
            file_path: &str,
            source: ReviewContentSource,
            bytes: Vec<ReviewSideBytes>,
        ) -> Self {
            self.source_byte_sequences.get_mut().unwrap().insert(
                (file_path.to_string(), review_source_key(source)),
                bytes.into(),
            );
            self
        }

        fn with_source_metadata(
            mut self,
            file_path: &str,
            source: ReviewContentSource,
            metadata: ReviewSideMetadata,
        ) -> Self {
            self.source_metadata
                .insert((file_path.to_string(), review_source_key(source)), metadata);
            self
        }

        fn with_binary_attribute(mut self, file_path: &str, binary: bool) -> Self {
            self.binary_attributes.insert(file_path.to_string(), binary);
            self
        }

        fn with_hunk_count(mut self, file_path: &str, hunk_count: usize) -> Self {
            self.hunk_count_by_path
                .insert(file_path.to_string(), hunk_count);
            self
        }

        fn with_real_diff(mut self) -> Self {
            self.real_diff = true;
            self
        }

        fn with_change_group(mut self, file_path: &str, group_index: u32, hunk_index: u32) -> Self {
            self.hunk_indexes_by_path
                .insert(file_path.to_string(), vec![hunk_index]);
            self.change_groups_by_path.insert(
                file_path.to_string(),
                vec![change_group(group_index, hunk_index)],
            );
            self
        }

        fn with_change_group_missing_hunk(
            mut self,
            file_path: &str,
            group_index: u32,
            hunk_index: u32,
        ) -> Self {
            self.hunk_indexes_by_path
                .insert(file_path.to_string(), Vec::new());
            self.change_groups_by_path.insert(
                file_path.to_string(),
                vec![change_group(group_index, hunk_index)],
            );
            self
        }

        fn metadata_for(&self, file_path: &str, source: ReviewContentSource) -> ReviewSideMetadata {
            let key = (file_path.to_string(), review_source_key(source));
            if let Some(bytes) = self
                .source_byte_sequences
                .lock()
                .unwrap()
                .get(&key)
                .and_then(|sequence| sequence.front())
            {
                return match bytes {
                    ReviewSideBytes::Present(bytes) => ReviewSideMetadata::Present {
                        size_bytes: bytes.len() as u64,
                    },
                    ReviewSideBytes::Missing => ReviewSideMetadata::Missing,
                };
            }
            self.source_metadata.get(&key).copied().unwrap_or_else(|| {
                match self.source_bytes.get(&key) {
                    Some(ReviewSideBytes::Present(bytes)) => ReviewSideMetadata::Present {
                        size_bytes: bytes.len() as u64,
                    },
                    _ => ReviewSideMetadata::Missing,
                }
            })
        }
    }

    #[async_trait::async_trait]

    impl ReviewCodePort for FakeReviewCode {
        fn get_branch_diff_summary(
            &self,
            repo_path: &str,
            _base_branch: Option<&str>,
        ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("branch-diff:{repo_path}"));
            Ok(self.branch_summary.clone())
        }

        fn build_diff_file_tree(&self, entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto> {
            entries
                .into_iter()
                .map(|entry| DiffTreeNodeDto {
                    id: entry.path.clone(),
                    name: entry
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.path)
                        .to_string(),
                    path: entry.path,
                    node_type: "file".to_string(),
                    status: Some(entry.status),
                    additions: Some(entry.additions),
                    deletions: Some(entry.deletions),
                    children: Vec::new(),
                })
                .collect()
        }

        fn select_review_side_source(
            &self,
            file_path: &str,
            side: ReviewBlobSide,
            section: ReviewSection,
            base: ReviewBase,
        ) -> Result<SelectedReviewSide, CodeUsecaseError> {
            let source = review_content_source_for(side, section, base);
            Ok(SelectedReviewSide {
                source,
                metadata: self.metadata_for(file_path, source),
            })
        }

        fn read_review_source_bytes(
            &self,
            file_path: &str,
            source: ReviewContentSource,
        ) -> Result<ReviewSideBytes, CodeUsecaseError> {
            let key = (file_path.to_string(), review_source_key(source));
            {
                let mut sequences = self.source_byte_sequences.lock().unwrap();
                if let Some(sequence) = sequences.get_mut(&key) {
                    if sequence.len() > 1 {
                        return Ok(sequence.pop_front().unwrap());
                    }
                    if let Some(bytes) = sequence.front() {
                        return Ok(bytes.clone());
                    }
                }
            }
            Ok(self
                .source_bytes
                .get(&key)
                .cloned()
                .unwrap_or(ReviewSideBytes::Missing))
        }

        fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError> {
            Ok(self
                .binary_attributes
                .get(file_path)
                .copied()
                .unwrap_or(false))
        }

        fn compute_diff_hunks(
            &self,
            original: &str,
            modified: &str,
            file_path: Option<&str>,
        ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
            let has_fixed_diff = file_path
                .map(|path| {
                    self.hunk_indexes_by_path.contains_key(path)
                        || self.hunk_count_by_path.contains_key(path)
                        || self.change_groups_by_path.contains_key(path)
                })
                .unwrap_or(false);
            if self.real_diff && !has_fixed_diff {
                return real_diff_hunks_result(original, modified, file_path);
            }
            let hunk_indexes = file_path
                .and_then(|path| self.hunk_indexes_by_path.get(path).cloned())
                .unwrap_or_else(|| {
                    let hunk_count = file_path
                        .and_then(|path| self.hunk_count_by_path.get(path).copied())
                        .unwrap_or_else(|| usize::from(original != modified));
                    (0..hunk_count as u32).collect()
                });
            let change_groups = file_path
                .and_then(|path| self.change_groups_by_path.get(path).cloned())
                .unwrap_or_default();
            Ok(DiffHunksResultDto {
                hunks: hunk_indexes
                    .into_iter()
                    .map(|index| HunkDto {
                        index,
                        hunk_id: format!("h:{index}"),
                        old_start: 1,
                        old_lines: 1,
                        new_start: 1,
                        new_lines: 1,
                        lines: vec!["@@".to_string()],
                    })
                    .collect(),
                change_groups,
            })
        }

        fn generate_group_patch(
            &self,
            file_path: &str,
            hunk: &Hunk,
            group: &ChangeGroup,
        ) -> String {
            self.calls.lock().unwrap().push(format!(
                "generate-patch:{file_path}:{}:{}",
                hunk.index, group.group_index
            ));
            "patch".to_string()
        }

        async fn git_stage_hunk(
            &self,
            repo_path: &str,
            _patch: &str,
        ) -> Result<(), CodeUsecaseError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("stage-hunk:{repo_path}"));
            Ok(())
        }

        async fn git_unstage_hunk(
            &self,
            repo_path: &str,
            _patch: &str,
        ) -> Result<(), CodeUsecaseError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("unstage-hunk:{repo_path}"));
            Ok(())
        }
    }

    fn repository_snapshot(version: u64, stale: bool) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            flags: SnapshotFlags {
                stale,
                loading: false,
            },
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        }
    }

    fn repository_snapshot_with_parts(
        version: u64,
        flags: SnapshotFlags,
        status: Vec<FileStatusDto>,
        diff_stats: Vec<FileDiffStatDto>,
        staged_tree: Vec<DiffTreeNodeDto>,
        changes_tree: Vec<DiffTreeNodeDto>,
    ) -> RepositorySnapshot {
        let diff_file_tree = staged_tree
            .iter()
            .cloned()
            .chain(changes_tree.iter().cloned())
            .collect();
        RepositorySnapshot {
            version,
            flags,
            status,
            diff_stats,
            dirty_count: 0,
            diff_file_tree,
            staged_diff_file_tree: staged_tree,
            changes_diff_file_tree: changes_tree,
        }
    }

    fn file_status(path: &str, index_status: &str, worktree_status: &str) -> FileStatusDto {
        FileStatusDto {
            path: path.to_string(),
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        }
    }

    fn diff_stat(
        path: &str,
        index_additions: u32,
        index_deletions: u32,
        wt_additions: u32,
        wt_deletions: u32,
    ) -> FileDiffStatDto {
        FileDiffStatDto {
            path: path.to_string(),
            index_additions,
            index_deletions,
            wt_additions,
            wt_deletions,
        }
    }

    fn tree_node(path: &str, status: &str, additions: u32, deletions: u32) -> DiffTreeNodeDto {
        DiffTreeNodeDto {
            id: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            node_type: "file".to_string(),
            status: Some(status.to_string()),
            additions: Some(additions),
            deletions: Some(deletions),
            children: Vec::new(),
        }
    }

    fn change_group(group_index: u32, hunk_index: u32) -> ChangeGroupDto {
        ChangeGroupDto {
            group_index,
            group_id: format!("g:{group_index}"),
            hunk_index,
            new_start: 1,
            new_end: 1,
            line_offset_start: 0,
            line_offset_end: 0,
            is_staged: None,
        }
    }

    fn real_diff_hunks_result(
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        let raw_hunks = crate::adaptor::gateway::code::diff_compute::diff_buffers(
            original, modified, file_path,
        )?;
        let hunks = crate::domain::code::services::hunk::assign_hunk_ids(&raw_hunks);
        let change_groups = crate::domain::code::services::hunk::compute_change_groups(&hunks);
        Ok(DiffHunksResultDto {
            hunks: hunks
                .iter()
                .map(|hunk| HunkDto {
                    index: hunk.index,
                    hunk_id: hunk.hunk_id.clone(),
                    old_start: hunk.old_start,
                    old_lines: hunk.old_lines,
                    new_start: hunk.new_start,
                    new_lines: hunk.new_lines,
                    lines: hunk.lines.clone(),
                })
                .collect(),
            change_groups: change_groups
                .iter()
                .map(|group| ChangeGroupDto {
                    group_index: group.group_index,
                    group_id: group.group_id.clone(),
                    hunk_index: group.hunk_index,
                    new_start: group.new_start,
                    new_end: group.new_end,
                    line_offset_start: group.line_offset_start,
                    line_offset_end: group.line_offset_end,
                    is_staged: group.is_staged,
                })
                .collect(),
        })
    }

    fn review_content_source_for(
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
    ) -> ReviewContentSource {
        if base.is_branch_base() {
            return match side {
                ReviewBlobSide::Original => ReviewContentSource::BranchBase,
                ReviewBlobSide::Modified => ReviewContentSource::WorkingTree,
            };
        }
        if section.is_staged() {
            return match side {
                ReviewBlobSide::Original => ReviewContentSource::Head,
                ReviewBlobSide::Modified => ReviewContentSource::Staged,
            };
        }
        match side {
            ReviewBlobSide::Original => ReviewContentSource::Staged,
            ReviewBlobSide::Modified => ReviewContentSource::WorkingTree,
        }
    }

    fn review_source_key(source: ReviewContentSource) -> &'static str {
        match source {
            ReviewContentSource::BranchBase => "branch-base",
            ReviewContentSource::Head => "head",
            ReviewContentSource::Staged => "staged",
            ReviewContentSource::WorkingTree => "working-tree",
        }
    }

    fn present_text(content: &str) -> ReviewSideBytes {
        ReviewSideBytes::Present(content.as_bytes().to_vec())
    }

    fn present_metadata(size_bytes: u64) -> ReviewSideMetadata {
        ReviewSideMetadata::Present { size_bytes }
    }

    fn snapshot_with_single_status(
        version: u64,
        path: &str,
        index_status: &str,
        worktree_status: &str,
    ) -> RepositorySnapshot {
        let status = vec![file_status(path, index_status, worktree_status)];
        let (staged_files, changed_files) = split_staged_changed_statuses(&status);
        let staged_tree = staged_files
            .iter()
            .map(|entry| tree_node(&entry.path, &entry.index_status, 1, 0))
            .collect();
        let changes_tree = changed_files
            .iter()
            .map(|entry| tree_node(&entry.path, &entry.worktree_status, 1, 0))
            .collect();

        repository_snapshot_with_parts(
            version,
            SnapshotFlags {
                stale: false,
                loading: false,
            },
            status,
            vec![diff_stat(path, 1, 0, 1, 0)],
            staged_tree,
            changes_tree,
        )
    }

    fn group_action_section(action: &str) -> &'static str {
        match action {
            "stage" => "changes",
            "unstage" => "staged",
            _ => unreachable!("unknown group action"),
        }
    }

    fn group_action_sources(
        action: &str,
    ) -> (
        ReviewContentSource,
        ReviewContentSource,
        &'static str,
        &'static str,
    ) {
        match action {
            "stage" => (
                ReviewContentSource::Staged,
                ReviewContentSource::WorkingTree,
                "none",
                "modified",
            ),
            "unstage" => (
                ReviewContentSource::Head,
                ReviewContentSource::Staged,
                "modified",
                "none",
            ),
            _ => unreachable!("unknown group action"),
        }
    }

    fn snapshot_for_group_action(version: u64, path: &str, action: &str) -> RepositorySnapshot {
        let (_, _, index_status, worktree_status) = group_action_sources(action);
        snapshot_with_single_status(version, path, index_status, worktree_status)
    }

    fn fake_code_for_group_action_content(
        action: &str,
        file_path: &str,
        original: &str,
        modified: &str,
    ) -> FakeReviewCode {
        let (original_source, modified_source, _, _) = group_action_sources(action);
        FakeReviewCode::new()
            .with_real_diff()
            .with_source_bytes(file_path, original_source, present_text(original))
            .with_source_bytes(file_path, modified_source, present_text(modified))
    }

    fn fake_code_for_group_action_content_sequence(
        action: &str,
        file_path: &str,
        original: &str,
        modified_reads: Vec<&str>,
    ) -> FakeReviewCode {
        let (original_source, modified_source, _, _) = group_action_sources(action);
        FakeReviewCode::new()
            .with_real_diff()
            .with_source_bytes(file_path, original_source, present_text(original))
            .with_source_byte_sequence(
                file_path,
                modified_source,
                modified_reads.into_iter().map(present_text).collect(),
            )
    }

    fn usecase_with_code(
        snapshots: Vec<RepositorySnapshot>,
        code: FakeReviewCode,
    ) -> ReviewUsecase {
        ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(snapshots)),
            Arc::new(code),
        )
    }

    fn text_view(view: ReviewFileViewDto) -> ReviewTextDiffDto {
        match view {
            ReviewFileViewDto::TextDiff(dto) => dto,
            other => panic!("expected text diff view, got {other:?}"),
        }
    }

    fn fallback_view_dto(view: ReviewFileViewDto) -> ReviewFallbackDto {
        match view {
            ReviewFileViewDto::Fallback(dto) => dto,
            other => panic!("expected fallback view, got {other:?}"),
        }
    }

    fn binary_view_dto(view: ReviewFileViewDto) -> ReviewBinaryDto {
        match view {
            ReviewFileViewDto::Binary(dto) => dto,
            other => panic!("expected binary view, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn head_snapshot_is_composed_inside_review_usecase_from_repository_snapshot() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![
            repository_snapshot_with_parts(
                42,
                SnapshotFlags {
                    stale: true,
                    loading: true,
                },
                vec![
                    file_status("src/lib.rs", "modified", "none"),
                    file_status("src/main.rs", "none", "modified"),
                ],
                vec![
                    diff_stat("src/lib.rs", 2, 1, 0, 0),
                    diff_stat("src/main.rs", 0, 0, 3, 4),
                ],
                vec![tree_node("src/lib.rs", "modified", 2, 1)],
                vec![tree_node("src/main.rs", "modified", 3, 4)],
            ),
        ]));
        let code = Arc::new(FakeReviewCode::new());
        let usecase = ReviewUsecase::new_with_ports(provider, code.clone());

        let dto = usecase.get_review_snapshot("/repo", "head").unwrap();

        assert_eq!(dto.version, 42);
        assert!(dto.stale);
        assert!(dto.loading);
        assert!(serde_json::to_value(&dto).unwrap().get("limited").is_none());
        assert_eq!(dto.files.len(), 2);
        assert_eq!(dto.files[0].file_id, "src/lib.rs");
        assert_eq!(dto.files[0].additions, 2);
        assert_eq!(dto.files[1].file_id, "src/main.rs");
        assert_eq!(dto.files[1].deletions, 4);
        assert_eq!(dto.staged_files.len(), 1);
        assert_eq!(dto.staged_files[0].path, "src/lib.rs");
        assert_eq!(dto.changed_files.len(), 1);
        assert_eq!(dto.changed_files[0].path, "src/main.rs");
        assert_eq!(dto.staged_file_count, 1);
        assert_eq!(dto.changes_file_count, 1);
        assert!(code.calls().is_empty());
    }

    #[tokio::test]
    async fn head_snapshot_read_model_splits_staged_and_changed_status_in_rust() {
        let snapshot = repository_snapshot_with_parts(
            7,
            SnapshotFlags {
                stale: false,
                loading: false,
            },
            vec![
                file_status("staged.rs", "modified", "none"),
                file_status("changed.rs", "none", "modified"),
                file_status("both.rs", "new", "deleted"),
                file_status("ignored.rs", "none", "ignored"),
                file_status("clean.rs", "none", "none"),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let dto = head_review_snapshot(ReviewBase::Head, &snapshot);

        let staged_paths = dto
            .staged_files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        let changed_paths = dto
            .changed_files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(staged_paths, vec!["staged.rs", "both.rs"]);
        assert_eq!(changed_paths, vec!["changed.rs", "both.rs"]);
        assert_eq!(dto.staged_file_count, dto.staged_files.len());
        assert_eq!(dto.changes_file_count, dto.changed_files.len());
    }

    #[tokio::test]
    async fn branch_base_snapshot_is_composed_inside_review_usecase_with_snapshot_flags() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![
            repository_snapshot_with_parts(
                55,
                SnapshotFlags {
                    stale: true,
                    loading: false,
                },
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            repository_snapshot(55, true),
        ]));
        let code = Arc::new(FakeReviewCode::with_branch_files(vec![
            ChangedFileDto {
                path: "src/feature.rs".to_string(),
                old_path: None,
                status: "modified".to_string(),
                binary: false,
                stats: DiffStatsDto {
                    additions: 5,
                    deletions: 2,
                },
            },
            ChangedFileDto {
                path: "README.md".to_string(),
                old_path: None,
                status: "added".to_string(),
                binary: false,
                stats: DiffStatsDto {
                    additions: 9,
                    deletions: 0,
                },
            },
        ]));
        let usecase = ReviewUsecase::new_with_ports(provider, code.clone());

        let dto = usecase.get_review_snapshot("/repo", "branch-base").unwrap();

        assert_eq!(dto.version, 55);
        assert!(dto.stale);
        assert!(!dto.loading);
        assert!(serde_json::to_value(&dto).unwrap().get("limited").is_none());
        assert_eq!(dto.base, "branch-base");
        assert_eq!(dto.files.len(), 2);
        assert_eq!(dto.files[0].file_id, "src/feature.rs");
        assert_eq!(dto.files[0].index_status, "none");
        assert_eq!(dto.files[0].worktree_status, "modified");
        assert_eq!(dto.files[1].worktree_status, "added");
        assert_eq!(dto.diff_stats[0].wt_additions, 5);
        assert!(dto.staged_tree.is_empty());
        assert_eq!(dto.changes_tree.len(), 2);
        assert_eq!(dto.tree.len(), 2);
        assert!(dto.staged_files.is_empty());
        assert_eq!(
            dto.changed_files
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/feature.rs", "README.md"]
        );
        assert_eq!(dto.changed_files[1].worktree_status, "new");
        assert_eq!(dto.staged_file_count, 0);
        assert_eq!(dto.changes_file_count, 2);
        assert_eq!(code.calls(), vec!["branch-diff:/repo"]);
    }

    #[tokio::test]
    async fn branch_diff_statuses_are_normalized_for_git_file_status_contract() {
        let cases = [
            ("added", "new"),
            ("copied", "new"),
            ("renamed", "modified"),
            ("modified", "modified"),
            ("deleted", "deleted"),
        ];

        for (branch_diff_status, git_worktree_status) in cases {
            assert_eq!(
                normalize_branch_diff_worktree_status(branch_diff_status).unwrap(),
                git_worktree_status
            );
        }

        let err = normalize_branch_diff_worktree_status("ignored")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported branch diff status"));
    }

    #[tokio::test]
    async fn branch_base_snapshot_version_change_marks_result_stale_with_current_version() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![
            repository_snapshot(10, false),
            repository_snapshot(11, false),
        ]));
        let code = Arc::new(FakeReviewCode::new());
        let usecase = ReviewUsecase::new_with_ports(provider, code.clone());

        let dto = usecase.get_review_snapshot("/repo", "branch-base").unwrap();

        assert_eq!(dto.version, 11);
        assert!(dto.stale);
        assert_eq!(code.calls(), vec!["branch-diff:/repo"]);
    }

    #[tokio::test]
    async fn review_file_view_resolves_path_and_switches_head_sections() {
        let path = "/repo/src/app.rs";
        let code = FakeReviewCode::new()
            .with_source_bytes(path, ReviewContentSource::Head, present_text("head\n"))
            .with_source_bytes(path, ReviewContentSource::Staged, present_text("staged\n"))
            .with_source_bytes(
                path,
                ReviewContentSource::WorkingTree,
                present_text("working\n"),
            );
        let usecase = usecase_with_code(
            vec![
                snapshot_with_single_status(7, "src/app.rs", "modified", "modified"),
                snapshot_with_single_status(7, "src/app.rs", "modified", "modified"),
            ],
            code,
        );

        let changes = text_view(
            usecase
                .get_review_file_view("/repo", "src/app.rs", "changes", "head")
                .unwrap(),
        );
        let staged = text_view(
            usecase
                .get_review_file_view("/repo", "src/app.rs", "staged", "head")
                .unwrap(),
        );

        assert_eq!(changes.original, "staged\n");
        assert_eq!(changes.modified, "working\n");
        assert_eq!(changes.source, ReviewTextSource::Diff);
        assert!(!changes.limited);
        assert_eq!(staged.original, "head\n");
        assert_eq!(staged.modified, "staged\n");
        assert_eq!(staged.source, ReviewTextSource::Diff);
    }

    #[tokio::test]
    async fn review_file_view_reports_added_deleted_and_unborn_head_as_text_sources() {
        let added_path = "/repo/new.txt";
        let deleted_path = "/repo/deleted.txt";
        let code = FakeReviewCode::new()
            .with_source_bytes(
                added_path,
                ReviewContentSource::Staged,
                present_text("new file\n"),
            )
            .with_source_bytes(
                deleted_path,
                ReviewContentSource::Staged,
                present_text("deleted file\n"),
            );
        let usecase = usecase_with_code(
            vec![
                snapshot_with_single_status(1, "new.txt", "added", "none"),
                snapshot_with_single_status(1, "deleted.txt", "none", "deleted"),
            ],
            code,
        );

        let added = text_view(
            usecase
                .get_review_file_view("/repo", "/repo/new.txt", "staged", "head")
                .unwrap(),
        );
        let deleted = text_view(
            usecase
                .get_review_file_view("/repo", "deleted.txt", "changes", "head")
                .unwrap(),
        );

        assert_eq!(added.original, "");
        assert_eq!(added.modified, "new file\n");
        assert_eq!(added.source, ReviewTextSource::Added);
        assert_eq!(deleted.original, "deleted file\n");
        assert_eq!(deleted.modified, "");
        assert_eq!(deleted.source, ReviewTextSource::Deleted);
    }

    #[tokio::test]
    async fn test_差分表示_画像はdata_urlを埋め込みバイナリはサイズだけを返す() {
        // Given
        let image_path = "/repo/assets/logo.png";
        let binary_path = "/repo/assets/archive.bin";
        let code = FakeReviewCode::new()
            .with_source_metadata(
                image_path,
                ReviewContentSource::Staged,
                present_metadata(12),
            )
            .with_source_metadata(
                image_path,
                ReviewContentSource::WorkingTree,
                present_metadata(13),
            )
            .with_source_bytes(
                image_path,
                ReviewContentSource::Staged,
                ReviewSideBytes::Present(vec![1, 2, 3]),
            )
            .with_source_bytes(
                image_path,
                ReviewContentSource::WorkingTree,
                ReviewSideBytes::Present(vec![4, 5, 6, 7]),
            )
            .with_source_metadata(
                binary_path,
                ReviewContentSource::Staged,
                present_metadata(21),
            )
            .with_source_metadata(
                binary_path,
                ReviewContentSource::WorkingTree,
                present_metadata(34),
            )
            .with_binary_attribute(binary_path, true);
        let usecase = usecase_with_code(
            vec![
                snapshot_with_single_status(4, "assets/logo.png", "modified", "modified"),
                snapshot_with_single_status(4, "assets/archive.bin", "modified", "modified"),
            ],
            code,
        );

        // When
        let image = usecase
            .get_review_file_view("/repo", "assets/logo.png", "changes", "head")
            .unwrap();
        let binary = usecase
            .get_review_file_view("/repo", "assets/archive.bin", "changes", "head")
            .unwrap();

        // Then
        let ReviewFileViewDto::Image(image) = image else {
            panic!("expected image view");
        };
        assert_eq!(image.mime, "image/png");
        assert_eq!(
            image.original_url.as_deref(),
            Some("data:image/png;base64,AQID")
        );
        assert_eq!(
            image.modified_url.as_deref(),
            Some("data:image/png;base64,BAUGBw==")
        );

        let ReviewFileViewDto::Binary(binary) = binary else {
            panic!("expected binary view");
        };
        assert_eq!(binary.original_size, Some(21));
        assert_eq!(binary.modified_size, Some(34));
    }

    #[tokio::test]
    async fn review_file_view_returns_binary_for_nul_bytes_without_binary_attribute() {
        let path = "/repo/assets/data.bin";
        let code = FakeReviewCode::new().with_source_bytes(
            path,
            ReviewContentSource::WorkingTree,
            ReviewSideBytes::Present(vec![b'a', 0, b'b']),
        );
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                6,
                "assets/data.bin",
                "none",
                "modified",
            )],
            code,
        );

        let binary = binary_view_dto(
            usecase
                .get_review_file_view("/repo", "assets/data.bin", "changes", "head")
                .unwrap(),
        );

        assert_eq!(binary.path, "assets/data.bin");
    }

    #[tokio::test]
    async fn review_file_view_returns_binary_for_non_utf8_bytes_without_binary_attribute() {
        let path = "/repo/assets/non-utf8.dat";
        let code = FakeReviewCode::new().with_source_bytes(
            path,
            ReviewContentSource::WorkingTree,
            ReviewSideBytes::Present(vec![0xff, 0xfe, b'a']),
        );
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                6,
                "assets/non-utf8.dat",
                "none",
                "modified",
            )],
            code,
        );

        let binary = binary_view_dto(
            usecase
                .get_review_file_view("/repo", "assets/non-utf8.dat", "changes", "head")
                .unwrap(),
        );

        assert_eq!(binary.path, "assets/non-utf8.dat");
    }

    #[tokio::test]
    async fn review_file_view_displays_deleted_file_under_removed_parent_directory() {
        let path = "/repo/src/nested/file.txt";
        let code = FakeReviewCode::new().with_source_bytes(
            path,
            ReviewContentSource::Staged,
            present_text("deleted content\n"),
        );
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                9,
                "src/nested/file.txt",
                "none",
                "deleted",
            )],
            code,
        );

        let view = text_view(
            usecase
                .get_review_file_view("/repo", "src/nested/file.txt", "changes", "head")
                .unwrap(),
        );

        assert_eq!(view.original, "deleted content\n");
        assert_eq!(view.modified, "");
        assert_eq!(view.source, ReviewTextSource::Deleted);
    }

    #[tokio::test]
    async fn review_file_view_applies_threshold_boundaries_in_usecase() {
        let exact_size_path = "/repo/exact-size.txt";
        let above_size_path = "/repo/above-size.txt";
        let exact_lines_path = "/repo/exact-lines.txt";
        let above_lines_path = "/repo/above-lines.txt";
        let exact_hunks_path = "/repo/exact-hunks.txt";
        let above_hunks_path = "/repo/above-hunks.txt";
        let exact_token_path = "/repo/exact-token.txt";
        let above_token_path = "/repo/above-token.txt";
        let exact_lines = "x\n".repeat(5_000);
        let above_lines = "x\n".repeat(5_001);
        let exact_token = "a".repeat(100_000);
        let above_token = "a".repeat(100_001);
        let code = FakeReviewCode::new()
            .with_source_metadata(
                exact_size_path,
                ReviewContentSource::WorkingTree,
                present_metadata(1_048_576),
            )
            .with_source_bytes(
                exact_size_path,
                ReviewContentSource::WorkingTree,
                present_text("inside size limit\n"),
            )
            .with_source_metadata(
                above_size_path,
                ReviewContentSource::WorkingTree,
                present_metadata(1_048_577),
            )
            .with_source_bytes(
                exact_lines_path,
                ReviewContentSource::WorkingTree,
                present_text(&exact_lines),
            )
            .with_source_bytes(
                above_lines_path,
                ReviewContentSource::WorkingTree,
                present_text(&above_lines),
            )
            .with_source_bytes(
                exact_hunks_path,
                ReviewContentSource::Staged,
                present_text("old\n"),
            )
            .with_source_bytes(
                exact_hunks_path,
                ReviewContentSource::WorkingTree,
                present_text("new\n"),
            )
            .with_hunk_count("exact-hunks.txt", 300)
            .with_source_bytes(
                above_hunks_path,
                ReviewContentSource::Staged,
                present_text("old\n"),
            )
            .with_source_bytes(
                above_hunks_path,
                ReviewContentSource::WorkingTree,
                present_text("new\n"),
            )
            .with_hunk_count("above-hunks.txt", 301)
            .with_source_bytes(
                exact_token_path,
                ReviewContentSource::WorkingTree,
                present_text(&exact_token),
            )
            .with_source_bytes(
                above_token_path,
                ReviewContentSource::WorkingTree,
                present_text(&above_token),
            );
        let usecase = usecase_with_code(
            vec![
                snapshot_with_single_status(5, "exact-size.txt", "none", "modified"),
                snapshot_with_single_status(5, "above-size.txt", "none", "modified"),
                snapshot_with_single_status(5, "exact-lines.txt", "none", "modified"),
                snapshot_with_single_status(5, "above-lines.txt", "none", "modified"),
                snapshot_with_single_status(5, "exact-hunks.txt", "modified", "modified"),
                snapshot_with_single_status(5, "above-hunks.txt", "modified", "modified"),
                snapshot_with_single_status(5, "exact-token.txt", "none", "modified"),
                snapshot_with_single_status(5, "above-token.txt", "none", "modified"),
            ],
            code,
        );

        let exact_size = text_view(
            usecase
                .get_review_file_view("/repo", "exact-size.txt", "changes", "head")
                .unwrap(),
        );
        let above_size = fallback_view_dto(
            usecase
                .get_review_file_view("/repo", "above-size.txt", "changes", "head")
                .unwrap(),
        );
        let exact_lines = text_view(
            usecase
                .get_review_file_view("/repo", "exact-lines.txt", "changes", "head")
                .unwrap(),
        );
        let above_lines = fallback_view_dto(
            usecase
                .get_review_file_view("/repo", "above-lines.txt", "changes", "head")
                .unwrap(),
        );
        let exact_hunks = text_view(
            usecase
                .get_review_file_view("/repo", "exact-hunks.txt", "changes", "head")
                .unwrap(),
        );
        let above_hunks = fallback_view_dto(
            usecase
                .get_review_file_view("/repo", "above-hunks.txt", "changes", "head")
                .unwrap(),
        );
        let exact_token = text_view(
            usecase
                .get_review_file_view("/repo", "exact-token.txt", "changes", "head")
                .unwrap(),
        );
        let above_token = fallback_view_dto(
            usecase
                .get_review_file_view("/repo", "above-token.txt", "changes", "head")
                .unwrap(),
        );

        assert_eq!(exact_size.modified, "inside size limit\n");
        assert_eq!(above_size.reason, ReviewLimitReasonDto::FileSize);
        assert_eq!(above_size.size_bytes, Some(1_048_577));
        assert_eq!(exact_lines.total_lines, 5_000);
        assert_eq!(above_lines.reason, ReviewLimitReasonDto::LineCount);
        assert_eq!(above_lines.total_lines, Some(5_001));
        assert_eq!(exact_hunks.hunks.len(), 300);
        assert_eq!(above_hunks.reason, ReviewLimitReasonDto::HunkCount);
        assert_eq!(above_hunks.hunk_count, Some(301));
        assert_eq!(exact_token.modified.len(), 100_000);
        assert_eq!(above_token.reason, ReviewLimitReasonDto::Tokenization);
        assert!(above_token.limited);
    }

    #[tokio::test]
    async fn review_stage_group_generates_patch_and_delegates_for_head_diff() {
        let path = "/repo/file.txt";
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_bytes(path, ReviewContentSource::Staged, present_text("old\n"))
                .with_source_bytes(
                    path,
                    ReviewContentSource::WorkingTree,
                    present_text("new\n"),
                )
                .with_change_group("file.txt", 0, 0),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_with_single_status(1, "file.txt", "modified", "modified"),
                snapshot_with_single_status(2, "file.txt", "modified", "modified"),
            ])),
            code.clone(),
        );
        let view = text_view(
            usecase
                .get_review_file_view("/repo", "file.txt", "changes", "head")
                .unwrap(),
        );
        let group_id = view.change_groups[0].group_id.clone();

        usecase
            .git_stage_review_group("/repo", "file.txt", "changes", "head", &group_id)
            .await
            .unwrap();

        assert_eq!(
            code.calls(),
            vec!["generate-patch:file.txt:0:0", "stage-hunk:/repo"]
        );
    }

    #[tokio::test]
    async fn review_unstage_group_generates_patch_and_delegates_for_head_diff() {
        let path = "/repo/file.txt";
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_bytes(path, ReviewContentSource::Head, present_text("old\n"))
                .with_source_bytes(path, ReviewContentSource::Staged, present_text("new\n"))
                .with_change_group("file.txt", 0, 0),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_with_single_status(1, "file.txt", "modified", "none"),
                snapshot_with_single_status(2, "file.txt", "modified", "none"),
            ])),
            code.clone(),
        );
        let view = text_view(
            usecase
                .get_review_file_view("/repo", "file.txt", "staged", "head")
                .unwrap(),
        );
        let group_id = view.change_groups[0].group_id.clone();

        usecase
            .git_unstage_review_group("/repo", "file.txt", "staged", "head", &group_id)
            .await
            .unwrap();

        assert_eq!(
            code.calls(),
            vec!["generate-patch:file.txt:0:0", "unstage-hunk:/repo"]
        );
    }

    #[tokio::test]
    async fn review_group_actions_reject_branch_base_for_stage_and_unstage() {
        for action in ["stage", "unstage"] {
            let code = Arc::new(FakeReviewCode::new());
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(
                    1, false,
                )])),
                code.clone(),
            );

            let err = match action {
                "stage" => {
                    usecase
                        .git_stage_review_group(
                            "/repo",
                            "file.txt",
                            "changes",
                            "branch-base",
                            "g:0",
                        )
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group(
                            "/repo",
                            "file.txt",
                            "changes",
                            "branch-base",
                            "g:0",
                        )
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap_err()
            .to_string();

            assert!(
                err.contains("review group actions are not available for branch-base diffs"),
                "unexpected {action} error: {err}"
            );
            assert!(code.calls().is_empty());
        }
    }

    #[tokio::test]
    async fn review_group_actions_report_missing_group_for_stage_and_unstage() {
        for action in ["stage", "unstage"] {
            let path = "/repo/file.txt";
            let code = Arc::new(
                FakeReviewCode::new()
                    .with_source_bytes(path, ReviewContentSource::Staged, present_text("old\n"))
                    .with_source_bytes(
                        path,
                        ReviewContentSource::WorkingTree,
                        present_text("new\n"),
                    ),
            );
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_with_single_status(1, "file.txt", "modified", "modified"),
                ])),
                code,
            );

            let err = match action {
                "stage" => {
                    usecase
                        .git_stage_review_group(
                            "/repo",
                            "file.txt",
                            "changes",
                            "head",
                            "missing-group",
                        )
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group(
                            "/repo",
                            "file.txt",
                            "changes",
                            "head",
                            "missing-group",
                        )
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap_err()
            .to_string();

            assert!(
                err.contains("review group target stale: missing-group"),
                "unexpected {action} error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn review_group_action_missing_group_uses_typed_stale_target_error() {
        let path = "/repo/file.txt";
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_bytes(path, ReviewContentSource::Staged, present_text("old\n"))
                .with_source_bytes(
                    path,
                    ReviewContentSource::WorkingTree,
                    present_text("new\n"),
                ),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_with_single_status(1, "file.txt", "modified", "modified"),
            ])),
            code,
        );

        let err = usecase
            .git_stage_review_group("/repo", "file.txt", "changes", "head", "missing-group")
            .await
            .unwrap_err();

        match err {
            CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { group_id }) => {
                assert_eq!(group_id, "missing-group");
            }
            other => panic!("expected stale review group target, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn review_group_actions_report_missing_snapshot_target_as_typed_stale_target_error() {
        for action in ["stage", "unstage"] {
            let path = "/repo/file.txt";
            let relative_path = "file.txt";
            let code = Arc::new(fake_code_for_group_action_content(
                action,
                path,
                "old\nsame\n",
                "new\nsame\n",
            ));
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_for_group_action(1, relative_path, action),
                    repository_snapshot(2, false),
                ])),
                code.clone(),
            );
            let section = group_action_section(action);
            let view = text_view(
                usecase
                    .get_review_file_view("/repo", relative_path, section, "head")
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            let err = match action {
                "stage" => {
                    usecase
                        .git_stage_review_group("/repo", relative_path, section, "head", &group_id)
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group(
                            "/repo",
                            relative_path,
                            section,
                            "head",
                            &group_id,
                        )
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap_err();

            match err {
                CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { group_id: stale }) => {
                    assert_eq!(stale, group_id);
                }
                other => panic!("expected stale review group target, got {other:?}"),
            }
            assert!(
                code.calls().is_empty(),
                "{action} should stop before patch generation and index mutation"
            );
        }
    }

    #[tokio::test]
    async fn review_group_actions_accept_previous_group_id_after_snapshot_refresh_when_content_matches(
    ) {
        for action in ["stage", "unstage"] {
            let path = "/repo/file.txt";
            let relative_path = "file.txt";
            let code = Arc::new(fake_code_for_group_action_content(
                action,
                path,
                "old\nsame\n",
                "new\nsame\n",
            ));
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_for_group_action(1, relative_path, action),
                    snapshot_for_group_action(2, relative_path, action),
                ])),
                code.clone(),
            );
            let section = group_action_section(action);
            let view = text_view(
                usecase
                    .get_review_file_view("/repo", relative_path, section, "head")
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            match action {
                "stage" => {
                    usecase
                        .git_stage_review_group("/repo", relative_path, section, "head", &group_id)
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group(
                            "/repo",
                            relative_path,
                            section,
                            "head",
                            &group_id,
                        )
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap();

            let apply_call = match action {
                "stage" => "stage-hunk:/repo",
                "unstage" => "unstage-hunk:/repo",
                _ => unreachable!(),
            };
            assert_eq!(
                code.calls(),
                vec![
                    "generate-patch:file.txt:0:0".to_string(),
                    apply_call.to_string(),
                ],
                "unexpected calls for {action}"
            );
        }
    }

    #[tokio::test]
    async fn review_group_action_accepts_later_duplicate_group_id_after_earlier_duplicate_disappears(
    ) {
        let path = "/repo/file.txt";
        let relative_path = "file.txt";
        let original = "x\na\ny\nx\na\ny\n";
        let staged_after_first = "x\nA\ny\nx\na\ny\n";
        let working = "x\nA\ny\nx\nA\ny\n";
        let code = Arc::new(
            FakeReviewCode::new()
                .with_real_diff()
                .with_source_byte_sequence(
                    path,
                    ReviewContentSource::Staged,
                    vec![present_text(original), present_text(staged_after_first)],
                )
                .with_source_bytes(
                    path,
                    ReviewContentSource::WorkingTree,
                    present_text(working),
                ),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_for_group_action(1, relative_path, "stage"),
                snapshot_for_group_action(2, relative_path, "stage"),
            ])),
            code.clone(),
        );
        let view = text_view(
            usecase
                .get_review_file_view("/repo", relative_path, "changes", "head")
                .unwrap(),
        );
        assert_eq!(view.change_groups.len(), 2);
        let later_group_id = view.change_groups[1].group_id.clone();

        usecase
            .git_stage_review_group("/repo", relative_path, "changes", "head", &later_group_id)
            .await
            .unwrap();

        assert_eq!(
            code.calls(),
            vec![
                "generate-patch:file.txt:0:0".to_string(),
                "stage-hunk:/repo".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn review_file_view_keeps_later_duplicate_hunk_id_after_earlier_duplicate_disappears() {
        let path = "/repo/file.txt";
        let relative_path = "file.txt";
        let original = concat!(
            "c1\n", "c2\n", "c3\n", "a\n", "c4\n", "c5\n", "c6\n", "gap1\n", "gap2\n", "gap3\n",
            "gap4\n", "gap5\n", "gap6\n", "gap7\n", "c1\n", "c2\n", "c3\n", "a\n", "c4\n", "c5\n",
            "c6\n",
        );
        let staged_after_first = concat!(
            "c1\n", "c2\n", "c3\n", "A\n", "c4\n", "c5\n", "c6\n", "gap1\n", "gap2\n", "gap3\n",
            "gap4\n", "gap5\n", "gap6\n", "gap7\n", "c1\n", "c2\n", "c3\n", "a\n", "c4\n", "c5\n",
            "c6\n",
        );
        let working = concat!(
            "c1\n", "c2\n", "c3\n", "A\n", "c4\n", "c5\n", "c6\n", "gap1\n", "gap2\n", "gap3\n",
            "gap4\n", "gap5\n", "gap6\n", "gap7\n", "c1\n", "c2\n", "c3\n", "A\n", "c4\n", "c5\n",
            "c6\n",
        );
        let code = Arc::new(
            FakeReviewCode::new()
                .with_real_diff()
                .with_source_byte_sequence(
                    path,
                    ReviewContentSource::Staged,
                    vec![present_text(original), present_text(staged_after_first)],
                )
                .with_source_bytes(
                    path,
                    ReviewContentSource::WorkingTree,
                    present_text(working),
                ),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_for_group_action(1, relative_path, "stage"),
                snapshot_for_group_action(2, relative_path, "stage"),
            ])),
            code,
        );

        let initial_view = text_view(
            usecase
                .get_review_file_view("/repo", relative_path, "changes", "head")
                .unwrap(),
        );
        assert_eq!(initial_view.hunks.len(), 2);
        let later_hunk_id = initial_view.hunks[1].hunk_id.clone();
        let refreshed_view = text_view(
            usecase
                .get_review_file_view("/repo", relative_path, "changes", "head")
                .unwrap(),
        );

        assert_eq!(refreshed_view.hunks.len(), 1);
        assert_eq!(later_hunk_id, refreshed_view.hunks[0].hunk_id);
    }

    #[tokio::test]
    async fn review_group_actions_reject_previous_group_id_after_snapshot_refresh_when_target_disappears(
    ) {
        for action in ["stage", "unstage"] {
            let path = "/repo/file.txt";
            let relative_path = "file.txt";
            let code = Arc::new(fake_code_for_group_action_content_sequence(
                action,
                path,
                "old\nsame\n",
                vec!["new\nsame\n", "old\nsame\n"],
            ));
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_for_group_action(1, relative_path, action),
                    snapshot_for_group_action(2, relative_path, action),
                ])),
                code.clone(),
            );
            let section = group_action_section(action);
            let view = text_view(
                usecase
                    .get_review_file_view("/repo", relative_path, section, "head")
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            let err = match action {
                "stage" => {
                    usecase
                        .git_stage_review_group("/repo", relative_path, section, "head", &group_id)
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group(
                            "/repo",
                            relative_path,
                            section,
                            "head",
                            &group_id,
                        )
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap_err();

            match err {
                CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { group_id: stale }) => {
                    assert_eq!(stale, group_id);
                }
                other => panic!("expected stale review group target, got {other:?}"),
            }
            assert!(
                code.calls().is_empty(),
                "{action} should stop before patch generation and index mutation"
            );
        }
    }

    #[tokio::test]
    async fn review_group_actions_report_missing_hunk_for_stage_and_unstage() {
        for action in ["stage", "unstage"] {
            let path = "/repo/file.txt";
            let code = Arc::new(
                FakeReviewCode::new()
                    .with_source_bytes(path, ReviewContentSource::Staged, present_text("old\n"))
                    .with_source_bytes(
                        path,
                        ReviewContentSource::WorkingTree,
                        present_text("new\n"),
                    )
                    .with_change_group_missing_hunk("file.txt", 0, 99),
            );
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_with_single_status(1, "file.txt", "modified", "modified"),
                ])),
                code,
            );

            let err = match action {
                "stage" => {
                    usecase
                        .git_stage_review_group("/repo", "file.txt", "changes", "head", "g:0")
                        .await
                }
                "unstage" => {
                    usecase
                        .git_unstage_review_group("/repo", "file.txt", "changes", "head", "g:0")
                        .await
                }
                _ => unreachable!(),
            }
            .unwrap_err()
            .to_string();

            assert!(
                err.contains("review hunk not found: 99"),
                "unexpected {action} error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn review_target_rejects_invalid_and_empty_paths() {
        for raw in ["../secret.txt", "src/../secret.txt", "/etc/passwd", "/"] {
            let err = resolve_review_target("/repo", raw).unwrap_err().to_string();
            assert!(
                err.contains("invalid review target path"),
                "unexpected error for {raw}: {err}"
            );
        }

        let err = resolve_review_target("/repo", "").unwrap_err().to_string();
        assert!(err.contains("empty review target path"));
    }

    #[tokio::test]
    async fn review_target_must_belong_to_snapshot() {
        let snapshot = head_review_snapshot(
            ReviewBase::Head,
            &snapshot_with_single_status(1, "tracked.txt", "none", "modified"),
        );

        let err = ensure_review_target_in_snapshot(
            &snapshot,
            "missing.txt",
            ReviewSection::Changes,
            ReviewBase::Head,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("review target is not in snapshot"));
    }

    #[tokio::test]
    async fn review_snapshot_contains_target_rejects_disallowed_sections_and_statuses() {
        let branch_snapshot = ReviewSnapshotDto {
            version: 1,
            stale: false,
            loading: false,
            base: "branch-base".to_string(),
            files: vec![ReviewFileEntryDto {
                file_id: "src/app.rs".to_string(),
                path: "src/app.rs".to_string(),
                index_status: "none".to_string(),
                worktree_status: "modified".to_string(),
                additions: 1,
                deletions: 0,
            }],
            staged_files: Vec::new(),
            changed_files: vec![FileStatusDto {
                path: "src/app.rs".to_string(),
                index_status: "none".to_string(),
                worktree_status: "modified".to_string(),
            }],
            diff_stats: Vec::new(),
            tree: Vec::new(),
            staged_tree: Vec::new(),
            changes_tree: Vec::new(),
            staged_file_count: 0,
            changes_file_count: 1,
        };
        assert!(!review_snapshot_contains_target(
            &branch_snapshot,
            "src/app.rs",
            ReviewSection::Staged,
            ReviewBase::BranchBase
        ));

        let head_snapshot = head_review_snapshot(
            ReviewBase::Head,
            &repository_snapshot_with_parts(
                1,
                SnapshotFlags {
                    stale: false,
                    loading: false,
                },
                vec![
                    file_status("ignored.txt", "none", "ignored"),
                    file_status("clean.txt", "none", "none"),
                ],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        assert!(!review_snapshot_contains_target(
            &head_snapshot,
            "ignored.txt",
            ReviewSection::Changes,
            ReviewBase::Head
        ));
        assert!(!review_snapshot_contains_target(
            &head_snapshot,
            "clean.txt",
            ReviewSection::Changes,
            ReviewBase::Head
        ));
    }

    #[tokio::test]
    async fn review_blob_mime_mapping_lives_in_usecase() {
        assert_eq!(review_blob_mime_for_path("assets/LOGO.PNG"), "image/png");
        assert_eq!(review_blob_mime_for_path("photo.jpg"), "image/jpeg");
        assert_eq!(review_blob_mime_for_path("photo.jpeg"), "image/jpeg");
        assert_eq!(review_blob_mime_for_path("icons/app.svg"), "image/svg+xml");
        assert_eq!(review_blob_mime_for_path("pic.webp"), "image/webp");
        assert_eq!(
            review_blob_mime_for_path("archive.bin"),
            "application/octet-stream"
        );
    }
}
