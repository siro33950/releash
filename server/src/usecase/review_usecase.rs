//! Review read model command の orchestration。
//!
//! controller と code usecase が repository state の具体 snapshot を共有しないよう、snapshot 取得、
//! version 整合、ReviewSnapshot / ReviewFileView の read model 構成をこの usecase に集約する。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::domain::code::services::hunk::{
    assign_stable_group_ids_for_side, assign_stable_hunk_ids_for_side, StableGroupIdSide,
};
use crate::domain::code::{
    ChangeGroup, CodeError, DiffFileEntry, Hunk, ReviewBase, ReviewBlobContentType, ReviewBlobSide,
    ReviewLimitReason, ReviewSection, ReviewSideBytes, ReviewSideMetadata, ReviewThresholds,
};

use super::code_dto::{
    BranchDiffSummaryDto, ChangeGroupDto, DiffHunksResultDto, DiffTreeNodeDto, HunkDto,
    ReviewBinaryDto, ReviewFallbackDto, ReviewFileEntryDto, ReviewFileViewDto, ReviewImageDto,
    ReviewLimitReasonDto, ReviewSnapshotDto, ReviewTextDiffDto, ReviewTextSource,
};
use super::code_error::CodeUsecaseError;
use super::code_usecase::{CodeUsecase, ReviewContentSource, SelectedReviewSide};
use super::repository_dto::{FileDiffStatDto, FileStatusDto};
use super::repository_state::snapshot::RepositorySnapshot;
use super::repository_state::status_membership::split_staged_changed_statuses;
use super::repository_state::{RepositoryStateError, RepositoryStateService};

trait ReviewSnapshotProvider: Send + Sync {
    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError>;
}

impl ReviewSnapshotProvider for RepositoryStateService {
    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
        RepositoryStateService::get_snapshot(self, worktree_path).map_err(repository_state_error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewTextSides {
    original: String,
    modified: String,
    source: ReviewTextSource,
}

#[derive(Debug, Clone, Copy)]
struct StableDiffSources<'a> {
    original: &'a str,
    modified: &'a str,
    line_offset: u32,
}

#[derive(Debug, Clone, Copy)]
struct ReviewBlobViewContext<'a> {
    relative_path: &'a str,
    file_path: &'a str,
    version: u64,
}

#[async_trait::async_trait]

trait ReviewCodePort: Send + Sync {
    fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError>;

    fn build_diff_file_tree(&self, entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto>;

    fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError>;

    fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError>;

    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError>;

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError>;

    fn generate_group_patch(&self, file_path: &str, hunk: &Hunk, group: &ChangeGroup) -> String;

    async fn git_stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError>;

    async fn git_unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError>;
}

#[async_trait::async_trait]

impl ReviewCodePort for CodeUsecase {
    fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
        CodeUsecase::get_branch_diff_summary(self, repo_path, base_branch)
    }

    fn build_diff_file_tree(&self, entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto> {
        CodeUsecase::build_diff_file_tree(self, entries)
    }

    fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError> {
        CodeUsecase::select_review_side_source(self, file_path, side, section, base)
    }

    fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        CodeUsecase::read_review_source_bytes(self, file_path, source)
    }

    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError> {
        CodeUsecase::review_binary_by_attributes(self, file_path)
    }

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        CodeUsecase::compute_diff_hunks(self, original, modified, file_path)
    }

    fn generate_group_patch(&self, file_path: &str, hunk: &Hunk, group: &ChangeGroup) -> String {
        CodeUsecase::generate_group_patch(self, file_path, hunk, group)
    }

    async fn git_stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        CodeUsecase::git_stage_hunk(self, repo_path, patch).await
    }

    async fn git_unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        CodeUsecase::git_unstage_hunk(self, repo_path, patch).await
    }
}

#[derive(Clone)]
pub struct ReviewUsecase {
    repository_state: Arc<dyn ReviewSnapshotProvider>,
    code: Arc<dyn ReviewCodePort>,
}

impl ReviewUsecase {
    pub fn new(repository_state: Arc<RepositoryStateService>, code: Arc<CodeUsecase>) -> Self {
        let repository_state: Arc<dyn ReviewSnapshotProvider> = repository_state;
        let code: Arc<dyn ReviewCodePort> = code;
        Self {
            repository_state,
            code,
        }
    }

    #[cfg(test)]
    fn new_with_ports(
        repository_state: Arc<dyn ReviewSnapshotProvider>,
        code: Arc<dyn ReviewCodePort>,
    ) -> Self {
        Self {
            repository_state,
            code,
        }
    }

    pub fn get_review_snapshot(
        &self,
        worktree_path: &str,
        base: &str,
    ) -> Result<ReviewSnapshotDto, CodeUsecaseError> {
        let base = ReviewBase::parse(base)?;
        let snapshot = self.snapshot(worktree_path)?;
        self.review_snapshot_with_branch_recheck(worktree_path, base, snapshot.as_ref())
    }

    pub fn get_review_file_view(
        &self,
        worktree_path: &str,
        path: &str,
        section: &str,
        base: &str,
    ) -> Result<ReviewFileViewDto, CodeUsecaseError> {
        let section = ReviewSection::parse(section)?;
        let base = ReviewBase::parse(base)?;
        let snapshot = self.snapshot(worktree_path)?;
        let review_snapshot =
            self.review_snapshot_with_branch_recheck(worktree_path, base, snapshot.as_ref())?;
        let relative_path = resolve_review_target(worktree_path, path)?;
        ensure_review_target_in_snapshot(&review_snapshot, &relative_path, section, base)?;
        let version = review_snapshot.version;
        let stale = review_snapshot.stale;
        let file_path = absolute_review_file_path(worktree_path, &relative_path)?;
        let original_source = self.code.select_review_side_source(
            &file_path,
            ReviewBlobSide::Original,
            section,
            base,
        )?;
        let modified_source = self.code.select_review_side_source(
            &file_path,
            ReviewBlobSide::Modified,
            section,
            base,
        )?;
        let original_metadata = original_source.metadata;
        let modified_metadata = modified_source.metadata;

        if original_metadata.is_missing() && modified_metadata.is_missing() {
            return Err(
                CodeError::Rule(format!("review target not found: {relative_path}")).into(),
            );
        }
        let blob_context = ReviewBlobViewContext {
            relative_path: &relative_path,
            file_path: &file_path,
            version,
        };

        let thresholds = ReviewThresholds::default();
        let max_size = [original_metadata, modified_metadata]
            .iter()
            .filter_map(|metadata| metadata.size_bytes())
            .max()
            .unwrap_or(0);
        if let Some(reason) = thresholds.file_size_limit(max_size) {
            return Ok(fallback_view(
                &relative_path,
                version,
                stale,
                reason,
                None,
                Some(max_size),
                None,
            ));
        }

        if ReviewBlobContentType::image_from_path(&relative_path).is_some() {
            let mime = review_blob_mime_for_path(&relative_path);
            return Ok(ReviewFileViewDto::Image(ReviewImageDto {
                version,
                stale,
                file_id: relative_path.clone(),
                path: relative_path.clone(),
                original_url: self.blob_data_url_if_present(original_source, blob_context, mime)?,
                modified_url: self.blob_data_url_if_present(modified_source, blob_context, mime)?,
                mime: mime.to_string(),
            }));
        }

        if self.code.review_binary_by_attributes(&file_path)? {
            return Ok(binary_view(
                blob_context,
                stale,
                original_metadata,
                modified_metadata,
            ));
        }

        let original_bytes = self
            .code
            .read_review_source_bytes(&file_path, original_source.source)?;
        let modified_bytes = self
            .code
            .read_review_source_bytes(&file_path, modified_source.source)?;

        if side_bytes_look_binary(&original_bytes) || side_bytes_look_binary(&modified_bytes) {
            return Ok(binary_view(
                blob_context,
                stale,
                original_metadata,
                modified_metadata,
            ));
        }

        let source = text_source(&original_bytes, &modified_bytes);
        let original = decode_side_text(&original_bytes)?;
        let modified = decode_side_text(&modified_bytes)?;
        self.review_text_view(
            &relative_path,
            version,
            stale,
            section,
            ReviewTextSides {
                original,
                modified,
                source,
            },
        )
    }

    pub async fn git_stage_review_group(
        &self,
        worktree_path: &str,
        path: &str,
        section: &str,
        base: &str,
        group_id: &str,
    ) -> Result<(), CodeUsecaseError> {
        let snapshot = self.snapshot(worktree_path)?;
        let patch = self.generate_review_group_patch(
            worktree_path,
            path,
            section,
            base,
            group_id,
            snapshot.as_ref(),
        )?;
        self.code.git_stage_hunk(worktree_path, &patch).await
    }

    pub async fn git_unstage_review_group(
        &self,
        worktree_path: &str,
        path: &str,
        section: &str,
        base: &str,
        group_id: &str,
    ) -> Result<(), CodeUsecaseError> {
        let snapshot = self.snapshot(worktree_path)?;
        let patch = self.generate_review_group_patch(
            worktree_path,
            path,
            section,
            base,
            group_id,
            snapshot.as_ref(),
        )?;
        self.code.git_unstage_hunk(worktree_path, &patch).await
    }

    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
        self.repository_state.snapshot(worktree_path)
    }

    fn review_snapshot_with_branch_recheck(
        &self,
        worktree_path: &str,
        base: ReviewBase,
        snapshot: &RepositorySnapshot,
    ) -> Result<ReviewSnapshotDto, CodeUsecaseError> {
        let initial_version = snapshot.version;
        let mut dto = self.review_snapshot_from_snapshot(worktree_path, base, snapshot)?;
        if base.is_branch_base() {
            let current = self.snapshot(worktree_path)?;
            if current.version != initial_version {
                dto.version = current.version;
                dto.stale = true;
            }
        }
        Ok(dto)
    }

    fn review_snapshot_from_snapshot(
        &self,
        worktree_path: &str,
        base: ReviewBase,
        snapshot: &RepositorySnapshot,
    ) -> Result<ReviewSnapshotDto, CodeUsecaseError> {
        if base.is_branch_base() {
            return self.branch_base_review_snapshot(worktree_path, base, snapshot);
        }
        Ok(head_review_snapshot(base, snapshot))
    }

    fn branch_base_review_snapshot(
        &self,
        worktree_path: &str,
        base: ReviewBase,
        snapshot: &RepositorySnapshot,
    ) -> Result<ReviewSnapshotDto, CodeUsecaseError> {
        let summary = self.code.get_branch_diff_summary(worktree_path, None)?;
        let entries: Vec<DiffFileEntry> = summary
            .changed_files
            .iter()
            .map(|file| DiffFileEntry {
                path: file.path.clone(),
                status: file.status.clone(),
                additions: file.stats.additions,
                deletions: file.stats.deletions,
            })
            .collect();
        let tree = self.code.build_diff_file_tree(entries);
        let files = summary
            .changed_files
            .iter()
            .map(|file| ReviewFileEntryDto {
                file_id: file.path.clone(),
                path: file.path.clone(),
                index_status: "none".to_string(),
                worktree_status: file.status.clone(),
                additions: file.stats.additions,
                deletions: file.stats.deletions,
            })
            .collect::<Vec<_>>();
        let status = summary
            .changed_files
            .iter()
            .map(|file| {
                Ok(FileStatusDto {
                    path: file.path.clone(),
                    index_status: "none".to_string(),
                    worktree_status: normalize_branch_diff_worktree_status(&file.status)?
                        .to_string(),
                })
            })
            .collect::<Result<Vec<_>, CodeUsecaseError>>()?;
        let (staged_files, changed_files) = split_staged_changed_statuses(&status);
        let staged_file_count = staged_files.len();
        let changes_file_count = changed_files.len();
        let diff_stats = summary
            .changed_files
            .iter()
            .map(|file| FileDiffStatDto {
                path: file.path.clone(),
                index_additions: 0,
                index_deletions: 0,
                wt_additions: file.stats.additions,
                wt_deletions: file.stats.deletions,
            })
            .collect();

        Ok(ReviewSnapshotDto {
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            base: base.as_str().to_string(),
            files,
            staged_files,
            changed_files,
            diff_stats,
            tree: tree.clone(),
            staged_tree: Vec::new(),
            changes_tree: tree,
            staged_file_count,
            changes_file_count,
        })
    }

    fn review_text_view(
        &self,
        relative_path: &str,
        version: u64,
        stale: bool,
        section: ReviewSection,
        sides: ReviewTextSides,
    ) -> Result<ReviewFileViewDto, CodeUsecaseError> {
        let ReviewTextSides {
            original,
            modified,
            source,
        } = sides;
        let thresholds = ReviewThresholds::default();
        let original_lines = line_count(&original);
        let modified_lines = line_count(&modified);
        let total_lines = original_lines.max(modified_lines);

        if let Some(reason) = thresholds.line_count_limit(total_lines) {
            return Ok(fallback_view(
                relative_path,
                version,
                stale,
                reason,
                Some(total_lines as u32),
                Some(original.len().max(modified.len()) as u64),
                None,
            ));
        }

        let hunks =
            self.compute_review_diff_hunks(&original, &modified, Some(relative_path), section)?;
        if let Some(reason) = thresholds.hunk_count_limit(hunks.hunks.len()) {
            return Ok(fallback_view(
                relative_path,
                version,
                stale,
                reason,
                Some(total_lines as u32),
                Some(original.len().max(modified.len()) as u64),
                Some(hunks.hunks.len() as u32),
            ));
        }
        if let Some(reason) = thresholds.tokenization_limit(
            original.chars().count().max(modified.chars().count()),
            total_lines,
        ) {
            return Ok(fallback_view(
                relative_path,
                version,
                stale,
                reason,
                Some(total_lines as u32),
                Some(original.len().max(modified.len()) as u64),
                Some(hunks.hunks.len() as u32),
            ));
        }

        Ok(ReviewFileViewDto::TextDiff(ReviewTextDiffDto {
            version,
            stale,
            file_id: relative_path.to_string(),
            path: relative_path.to_string(),
            original,
            modified,
            source,
            hunks: hunks.hunks,
            change_groups: hunks.change_groups,
            limited: false,
            total_lines: total_lines as u32,
        }))
    }

    fn blob_data_url_if_present(
        &self,
        side: SelectedReviewSide,
        context: ReviewBlobViewContext<'_>,
        mime: &str,
    ) -> Result<Option<String>, CodeUsecaseError> {
        use base64::Engine;
        if !side.metadata.is_present() {
            return Ok(None);
        }
        match self
            .code
            .read_review_source_bytes(context.file_path, side.source)?
        {
            ReviewSideBytes::Present(bytes) => Ok(Some(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))),
            ReviewSideBytes::Missing => Err(CodeError::Rule(format!(
                "review blob not found: {}",
                context.relative_path
            ))
            .into()),
        }
    }

    fn compute_review_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
        section: ReviewSection,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        self.compute_review_diff_hunks_with_stable_sources(
            original,
            modified,
            StableDiffSources {
                original,
                modified,
                line_offset: 0,
            },
            file_path,
            section,
        )
    }

    fn compute_review_diff_hunks_with_stable_sources(
        &self,
        original: &str,
        modified: &str,
        stable_sources: StableDiffSources<'_>,
        file_path: Option<&str>,
        section: ReviewSection,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        let mut result = self
            .code
            .compute_diff_hunks(original, modified, file_path)?;
        let hunks: Vec<Hunk> = result.hunks.iter().map(hunk_dto_to_domain).collect();
        let groups: Vec<ChangeGroup> = result
            .change_groups
            .iter()
            .map(change_group_dto_to_domain)
            .collect();
        let stable_source_hunks = if stable_sources.line_offset == 0 {
            hunks.clone()
        } else {
            hunks
                .iter()
                .map(|hunk| offset_hunk_coordinates(hunk, stable_sources.line_offset))
                .collect()
        };
        let stable_side = stable_group_id_side(section);
        let stable_hunks = assign_stable_hunk_ids_for_side(
            &stable_source_hunks,
            stable_sources.original,
            stable_sources.modified,
            stable_side,
        );
        let hunk_ids: HashMap<u32, String> = stable_hunks
            .iter()
            .map(|hunk| (hunk.index, hunk.hunk_id.clone()))
            .collect();
        let display_hunks: Vec<Hunk> = hunks
            .iter()
            .map(|hunk| Hunk {
                hunk_id: hunk_ids
                    .get(&hunk.index)
                    .cloned()
                    .unwrap_or_else(|| hunk.hunk_id.clone()),
                ..hunk.clone()
            })
            .collect();
        let stable_groups = assign_stable_group_ids_for_side(
            &stable_source_hunks,
            &groups,
            stable_sources.original,
            stable_sources.modified,
            stable_side,
        );
        result.hunks = display_hunks.iter().map(hunk_domain_to_dto).collect();
        result.change_groups = stable_groups
            .iter()
            .map(change_group_domain_to_dto)
            .collect();
        Ok(result)
    }

    fn generate_review_group_patch(
        &self,
        worktree_path: &str,
        path: &str,
        section: &str,
        base: &str,
        group_id: &str,
        snapshot: &RepositorySnapshot,
    ) -> Result<String, CodeUsecaseError> {
        let section = ReviewSection::parse(section)?;
        let base = ReviewBase::parse(base)?;
        if base.is_branch_base() {
            return Err(CodeError::Rule(
                "review group actions are not available for branch-base diffs".to_string(),
            )
            .into());
        }

        let review_snapshot = self.review_snapshot_from_snapshot(worktree_path, base, snapshot)?;
        let relative_path = resolve_review_target(worktree_path, path)?;
        if !review_snapshot_contains_target(&review_snapshot, &relative_path, section, base) {
            return Err(CodeError::StaleReviewGroupTarget {
                group_id: group_id.to_string(),
            }
            .into());
        }
        let file_path = absolute_review_file_path(worktree_path, &relative_path)?;
        let original_source = self.code.select_review_side_source(
            &file_path,
            ReviewBlobSide::Original,
            section,
            base,
        )?;
        let modified_source = self.code.select_review_side_source(
            &file_path,
            ReviewBlobSide::Modified,
            section,
            base,
        )?;
        let original = decode_side_text(
            &self
                .code
                .read_review_source_bytes(&file_path, original_source.source)?,
        )?;
        let modified = decode_side_text(
            &self
                .code
                .read_review_source_bytes(&file_path, modified_source.source)?,
        )?;
        let result =
            self.compute_review_diff_hunks(&original, &modified, Some(&relative_path), section)?;
        let group = result
            .change_groups
            .iter()
            .find(|group| group.group_id == group_id)
            .ok_or_else(|| CodeError::StaleReviewGroupTarget {
                group_id: group_id.to_string(),
            })?;
        let hunk = result
            .hunks
            .iter()
            .find(|hunk| hunk.index == group.hunk_index)
            .ok_or_else(|| {
                CodeError::Rule(format!("review hunk not found: {}", group.hunk_index))
            })?;

        Ok(self.code.generate_group_patch(
            &relative_path,
            &hunk_dto_to_domain(hunk),
            &change_group_dto_to_domain(group),
        ))
    }
}

fn head_review_snapshot(base: ReviewBase, snapshot: &RepositorySnapshot) -> ReviewSnapshotDto {
    let stats = stats_by_path(&snapshot.diff_stats);
    let files = snapshot
        .status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .map(|entry| {
            let stat = stats.get(entry.path.as_str()).copied();
            ReviewFileEntryDto {
                file_id: entry.path.clone(),
                path: entry.path.clone(),
                index_status: entry.index_status.clone(),
                worktree_status: entry.worktree_status.clone(),
                additions: stat
                    .map(|stat| stat.index_additions + stat.wt_additions)
                    .unwrap_or(0),
                deletions: stat
                    .map(|stat| stat.index_deletions + stat.wt_deletions)
                    .unwrap_or(0),
            }
        })
        .collect();
    let (staged_files, changed_files) = split_staged_changed_statuses(&snapshot.status);
    let staged_file_count = staged_files.len();
    let changes_file_count = changed_files.len();

    ReviewSnapshotDto {
        version: snapshot.version,
        stale: snapshot.flags.stale,
        loading: snapshot.flags.loading,
        base: base.as_str().to_string(),
        files,
        staged_files,
        changed_files,
        diff_stats: snapshot.diff_stats.clone(),
        tree: snapshot.diff_file_tree.clone(),
        staged_tree: snapshot.staged_diff_file_tree.clone(),
        changes_tree: snapshot.changes_diff_file_tree.clone(),
        staged_file_count,
        changes_file_count,
    }
}

fn normalize_branch_diff_worktree_status(status: &str) -> Result<&'static str, CodeUsecaseError> {
    match status {
        "added" | "copied" => Ok("new"),
        "deleted" => Ok("deleted"),
        "modified" | "renamed" => Ok("modified"),
        _ => Err(CodeError::Rule(format!(
            "unsupported branch diff status for review snapshot: {status}"
        ))
        .into()),
    }
}

fn ensure_review_target_in_snapshot(
    snapshot: &ReviewSnapshotDto,
    relative_path: &str,
    section: ReviewSection,
    base: ReviewBase,
) -> Result<(), CodeUsecaseError> {
    if review_snapshot_contains_target(snapshot, relative_path, section, base) {
        return Ok(());
    }

    Err(CodeError::Rule(format!(
        "review target is not in snapshot for {}/{}: {relative_path}",
        base.as_str(),
        section.as_str()
    ))
    .into())
}

fn review_snapshot_contains_target(
    snapshot: &ReviewSnapshotDto,
    relative_path: &str,
    section: ReviewSection,
    base: ReviewBase,
) -> bool {
    if base.is_branch_base() {
        return !section.is_staged()
            && snapshot
                .files
                .iter()
                .any(|entry| review_file_entry_matches(entry, relative_path));
    }

    if section.is_staged() {
        return snapshot
            .staged_files
            .iter()
            .any(|entry| entry.path == relative_path);
    }

    snapshot
        .changed_files
        .iter()
        .any(|entry| entry.path == relative_path)
}

fn review_file_entry_matches(entry: &ReviewFileEntryDto, relative_path: &str) -> bool {
    entry.file_id == relative_path || entry.path == relative_path
}

fn stats_by_path(diff_stats: &[FileDiffStatDto]) -> HashMap<&str, &FileDiffStatDto> {
    diff_stats
        .iter()
        .map(|stat| (stat.path.as_str(), stat))
        .collect()
}

fn resolve_review_target(worktree_path: &str, raw: &str) -> Result<String, CodeUsecaseError> {
    let worktree = Path::new(worktree_path);
    let path = Path::new(raw);
    let relative = if path.is_absolute() {
        path.strip_prefix(worktree)
            .map_err(|_| CodeError::Rule(format!("invalid review target path: {raw}")))?
    } else {
        path
    };

    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(CodeError::Rule(format!("invalid review target path: {raw}")).into());
    }

    let normalized = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        return Err(CodeError::Rule("empty review target path".to_string()).into());
    }
    Ok(normalized)
}

fn absolute_review_file_path(
    worktree_path: &str,
    relative_path: &str,
) -> Result<String, CodeUsecaseError> {
    let path = PathBuf::from(worktree_path).join(relative_path);
    Ok(path
        .to_str()
        .ok_or_else(|| CodeError::Rule("invalid path encoding".to_string()))?
        .to_string())
}

fn decode_side_text(bytes: &ReviewSideBytes) -> Result<String, CodeUsecaseError> {
    match bytes {
        ReviewSideBytes::Present(bytes) => Ok(std::str::from_utf8(bytes)
            .map_err(CodeError::from)?
            .to_string()),
        ReviewSideBytes::Missing => Ok(String::new()),
    }
}

fn side_bytes_look_binary(bytes: &ReviewSideBytes) -> bool {
    match bytes {
        ReviewSideBytes::Present(bytes) => {
            looks_binary(bytes) || std::str::from_utf8(bytes).is_err()
        }
        ReviewSideBytes::Missing => false,
    }
}

impl ReviewSideMetadata {
    fn is_present(self) -> bool {
        matches!(self, Self::Present { .. })
    }

    fn is_missing(self) -> bool {
        matches!(self, Self::Missing)
    }

    fn size_bytes(self) -> Option<u64> {
        match self {
            Self::Present { size_bytes } => Some(size_bytes),
            Self::Missing => None,
        }
    }
}

fn hunk_dto_to_domain(h: &HunkDto) -> Hunk {
    Hunk {
        index: h.index,
        hunk_id: h.hunk_id.clone(),
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h.lines.clone(),
    }
}

fn hunk_domain_to_dto(h: &Hunk) -> HunkDto {
    HunkDto {
        index: h.index,
        hunk_id: h.hunk_id.clone(),
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h.lines.clone(),
    }
}

fn offset_hunk_coordinates(hunk: &Hunk, line_offset: u32) -> Hunk {
    Hunk {
        old_start: if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start.saturating_add(line_offset)
        },
        new_start: if hunk.new_start == 0 {
            0
        } else {
            hunk.new_start.saturating_add(line_offset)
        },
        ..hunk.clone()
    }
}

fn change_group_dto_to_domain(g: &ChangeGroupDto) -> ChangeGroup {
    ChangeGroup {
        group_index: g.group_index,
        group_id: g.group_id.clone(),
        hunk_index: g.hunk_index,
        new_start: g.new_start,
        new_end: g.new_end,
        line_offset_start: g.line_offset_start,
        line_offset_end: g.line_offset_end,
        is_staged: g.is_staged,
    }
}

fn change_group_domain_to_dto(g: &ChangeGroup) -> ChangeGroupDto {
    ChangeGroupDto {
        group_index: g.group_index,
        group_id: g.group_id.clone(),
        hunk_index: g.hunk_index,
        new_start: g.new_start,
        new_end: g.new_end,
        line_offset_start: g.line_offset_start,
        line_offset_end: g.line_offset_end,
        is_staged: g.is_staged,
    }
}

fn stable_group_id_side(section: ReviewSection) -> StableGroupIdSide {
    if section.is_staged() {
        StableGroupIdSide::Original
    } else {
        StableGroupIdSide::Modified
    }
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

fn text_source(original: &ReviewSideBytes, modified: &ReviewSideBytes) -> ReviewTextSource {
    match (original, modified) {
        (ReviewSideBytes::Missing, ReviewSideBytes::Present(_)) => ReviewTextSource::Added,
        (ReviewSideBytes::Present(_), ReviewSideBytes::Missing) => ReviewTextSource::Deleted,
        _ => ReviewTextSource::Diff,
    }
}

fn fallback_view(
    relative_path: &str,
    version: u64,
    stale: bool,
    reason: ReviewLimitReason,
    total_lines: Option<u32>,
    size_bytes: Option<u64>,
    hunk_count: Option<u32>,
) -> ReviewFileViewDto {
    ReviewFileViewDto::Fallback(ReviewFallbackDto {
        version,
        stale,
        file_id: relative_path.to_string(),
        path: relative_path.to_string(),
        reason: limit_reason_to_dto(reason),
        total_lines,
        size_bytes,
        hunk_count,
        limited: true,
    })
}

fn binary_view(
    context: ReviewBlobViewContext<'_>,
    stale: bool,
    original_metadata: ReviewSideMetadata,
    modified_metadata: ReviewSideMetadata,
) -> ReviewFileViewDto {
    ReviewFileViewDto::Binary(ReviewBinaryDto {
        version: context.version,
        stale,
        file_id: context.relative_path.to_string(),
        path: context.relative_path.to_string(),
        original_size: original_metadata.size_bytes(),
        modified_size: modified_metadata.size_bytes(),
    })
}

fn limit_reason_to_dto(reason: ReviewLimitReason) -> ReviewLimitReasonDto {
    match reason {
        ReviewLimitReason::FileSize => ReviewLimitReasonDto::FileSize,
        ReviewLimitReason::LineCount => ReviewLimitReasonDto::LineCount,
        ReviewLimitReason::HunkCount => ReviewLimitReasonDto::HunkCount,
        ReviewLimitReason::Tokenization => ReviewLimitReasonDto::Tokenization,
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

fn review_blob_mime_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("tiff") | Some("tif") => "image/tiff",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    }
}

fn repository_state_error(error: RepositoryStateError) -> CodeUsecaseError {
    CodeError::External(error.to_string()).into()
}

#[cfg(test)]
#[path = "review_usecase_test.rs"]
mod review_usecase_tests;
