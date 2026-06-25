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
    ReviewLimitReasonDto, ReviewSnapshotDto, ReviewTextDiffDto, ReviewTextSource, ViewportDto,
};
use super::code_error::CodeUsecaseError;
use super::code_usecase::{CodeUsecase, ReviewContentSource, SelectedReviewSide};
use super::repository_dto::{FileDiffStatDto, FileStatusDto};
use super::repository_state::snapshot::RepositorySnapshot;
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
pub enum ReviewTarget {
    FileId(String),
    Path(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewViewport {
    pub start_line: u32,
    pub end_line: u32,
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
    worktree_path: &'a str,
    relative_path: &'a str,
    section: ReviewSection,
    base: ReviewBase,
    version: u64,
}

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

    fn review_blob_url(
        &self,
        worktree_path: &str,
        path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
        version: u64,
    ) -> String;

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> DiffHunksResultDto;

    fn generate_group_patch(&self, file_path: &str, hunk: &Hunk, group: &ChangeGroup) -> String;

    fn git_stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError>;

    fn git_unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError>;
}

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

    fn review_blob_url(
        &self,
        worktree_path: &str,
        path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
        version: u64,
    ) -> String {
        CodeUsecase::review_blob_url(self, worktree_path, path, side, section, base, version)
    }

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> DiffHunksResultDto {
        CodeUsecase::compute_diff_hunks(self, original, modified, file_path)
    }

    fn generate_group_patch(&self, file_path: &str, hunk: &Hunk, group: &ChangeGroup) -> String {
        CodeUsecase::generate_group_patch(self, file_path, hunk, group)
    }

    fn git_stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        CodeUsecase::git_stage_hunk(self, repo_path, patch)
    }

    fn git_unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        CodeUsecase::git_unstage_hunk(self, repo_path, patch)
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
        target: ReviewTarget,
        section: &str,
        base: &str,
        viewport: Option<ReviewViewport>,
        snapshot_version: Option<u64>,
    ) -> Result<ReviewFileViewDto, CodeUsecaseError> {
        let section = ReviewSection::parse(section)?;
        let base = ReviewBase::parse(base)?;
        let snapshot = self.snapshot(worktree_path)?;
        let review_snapshot =
            self.review_snapshot_with_branch_recheck(worktree_path, base, snapshot.as_ref())?;
        let relative_path = resolve_review_target(worktree_path, &target)?;
        ensure_review_target_in_snapshot(&review_snapshot, &relative_path, section, base)?;
        let version = review_snapshot.version;
        let stale = review_snapshot.stale
            || snapshot_version
                .map(|expected| expected != snapshot.version)
                .unwrap_or(false);
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
            worktree_path,
            relative_path: &relative_path,
            section,
            base,
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
            return Ok(ReviewFileViewDto::Image(ReviewImageDto {
                version,
                stale,
                file_id: relative_path.clone(),
                path: relative_path.clone(),
                original_url: self.blob_url_if_present(
                    ReviewBlobSide::Original,
                    original_metadata,
                    blob_context,
                ),
                modified_url: self.blob_url_if_present(
                    ReviewBlobSide::Modified,
                    modified_metadata,
                    blob_context,
                ),
                mime: review_blob_mime_for_path(&relative_path).to_string(),
            }));
        }

        if self.code.review_binary_by_attributes(&file_path)? {
            return Ok(self.binary_view(blob_context, stale, original_metadata, modified_metadata));
        }

        let original_bytes = self
            .code
            .read_review_source_bytes(&file_path, original_source.source)?;
        let modified_bytes = self
            .code
            .read_review_source_bytes(&file_path, modified_source.source)?;

        if side_bytes_look_binary(&original_bytes) || side_bytes_look_binary(&modified_bytes) {
            return Ok(self.binary_view(blob_context, stale, original_metadata, modified_metadata));
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
            viewport,
        )
    }

    pub fn git_stage_review_group(
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
        self.code.git_stage_hunk(worktree_path, &patch)
    }

    pub fn git_unstage_review_group(
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
        self.code.git_unstage_hunk(worktree_path, &patch)
    }

    pub fn read_review_blob_bytes(
        &self,
        worktree_path: &str,
        path: &str,
        side: ReviewBlobSide,
        section: &str,
        base: &str,
        version: u64,
    ) -> Result<Vec<u8>, CodeUsecaseError> {
        let snapshot = self.snapshot(worktree_path)?;
        ensure_current_review_blob_version(snapshot.as_ref(), version)?;
        let section = ReviewSection::parse(section)?;
        let base = ReviewBase::parse(base)?;
        let review_snapshot =
            self.review_snapshot_with_blob_version_recheck(worktree_path, base, snapshot.as_ref())?;
        let relative_path =
            resolve_review_target(worktree_path, &ReviewTarget::Path(path.to_string()))?;
        ensure_review_target_in_snapshot(&review_snapshot, &relative_path, section, base)?;
        let file_path = absolute_review_file_path(worktree_path, &relative_path)?;
        let selected = self
            .code
            .select_review_side_source(&file_path, side, section, base)?;
        match self
            .code
            .read_review_source_bytes(&file_path, selected.source)?
        {
            ReviewSideBytes::Present(bytes) => Ok(bytes),
            ReviewSideBytes::Missing => {
                Err(CodeError::Rule(format!("review blob not found: {relative_path}")).into())
            }
        }
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

    fn review_snapshot_with_blob_version_recheck(
        &self,
        worktree_path: &str,
        base: ReviewBase,
        snapshot: &RepositorySnapshot,
    ) -> Result<ReviewSnapshotDto, CodeUsecaseError> {
        let dto = self.review_snapshot_from_snapshot(worktree_path, base, snapshot)?;
        if base.is_branch_base() {
            let current = self.snapshot(worktree_path)?;
            ensure_current_review_blob_version(current.as_ref(), snapshot.version)?;
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
        let status = files
            .iter()
            .map(|file| FileStatusDto {
                path: file.path.clone(),
                index_status: file.index_status.clone(),
                worktree_status: file.worktree_status.clone(),
            })
            .collect();
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
            limited: snapshot.flags.limited,
            base: base.as_str().to_string(),
            files,
            status,
            diff_stats,
            tree: tree.clone(),
            staged_tree: Vec::new(),
            changes_tree: tree,
            staged_file_count: 0,
            changes_file_count: summary.changed_files.len(),
        })
    }

    fn review_text_view(
        &self,
        relative_path: &str,
        version: u64,
        stale: bool,
        section: ReviewSection,
        sides: ReviewTextSides,
        viewport: Option<ReviewViewport>,
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

        if let Some(requested_viewport) = viewport {
            let stable_original = original.clone();
            let stable_modified = modified.clone();
            let stable_line_offset = requested_viewport.start_line.max(1).saturating_sub(1);
            let (original, modified, viewport) =
                apply_viewport(original, modified, Some(requested_viewport));
            let hunks = self.compute_review_diff_hunks_with_stable_sources(
                &original,
                &modified,
                StableDiffSources {
                    original: &stable_original,
                    modified: &stable_modified,
                    line_offset: stable_line_offset,
                },
                Some(relative_path),
                section,
            );
            return Ok(ReviewFileViewDto::TextDiff(ReviewTextDiffDto {
                version,
                stale,
                file_id: relative_path.to_string(),
                path: relative_path.to_string(),
                original,
                modified,
                source,
                hunks: hunks.hunks,
                change_groups: hunks.change_groups,
                limited: true,
                viewport,
                total_lines: total_lines as u32,
            }));
        }

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
            self.compute_review_diff_hunks(&original, &modified, Some(relative_path), section);
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
            viewport: None,
            total_lines: total_lines as u32,
        }))
    }

    fn binary_view(
        &self,
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
            original_url: self.blob_url_if_present(
                ReviewBlobSide::Original,
                original_metadata,
                context,
            ),
            modified_url: self.blob_url_if_present(
                ReviewBlobSide::Modified,
                modified_metadata,
                context,
            ),
            original_size: original_metadata.size_bytes(),
            modified_size: modified_metadata.size_bytes(),
        })
    }

    fn blob_url_if_present(
        &self,
        side: ReviewBlobSide,
        metadata: ReviewSideMetadata,
        context: ReviewBlobViewContext<'_>,
    ) -> Option<String> {
        metadata.is_present().then(|| {
            self.code.review_blob_url(
                context.worktree_path,
                context.relative_path,
                side,
                context.section,
                context.base,
                context.version,
            )
        })
    }

    fn compute_review_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
        section: ReviewSection,
    ) -> DiffHunksResultDto {
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
    ) -> DiffHunksResultDto {
        let mut result = self.code.compute_diff_hunks(original, modified, file_path);
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
        result
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
        let relative_path =
            resolve_review_target(worktree_path, &ReviewTarget::Path(path.to_string()))?;
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
            self.compute_review_diff_hunks(&original, &modified, Some(&relative_path), section);
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

fn ensure_current_review_blob_version(
    snapshot: &RepositorySnapshot,
    version: u64,
) -> Result<(), CodeUsecaseError> {
    if version != snapshot.version {
        return Err(CodeError::StaleReviewBlobVersion {
            requested: version,
            current: snapshot.version,
        }
        .into());
    }
    Ok(())
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
    let staged_file_count = snapshot
        .status
        .iter()
        .filter(|entry| entry.index_status != "none")
        .count();
    let changes_file_count = snapshot
        .status
        .iter()
        .filter(|entry| entry.worktree_status != "none" && entry.worktree_status != "ignored")
        .count();

    ReviewSnapshotDto {
        version: snapshot.version,
        stale: snapshot.flags.stale,
        loading: snapshot.flags.loading,
        limited: snapshot.flags.limited,
        base: base.as_str().to_string(),
        files,
        status: snapshot.status.clone(),
        diff_stats: snapshot.diff_stats.clone(),
        tree: snapshot.diff_file_tree.clone(),
        staged_tree: snapshot.staged_diff_file_tree.clone(),
        changes_tree: snapshot.changes_diff_file_tree.clone(),
        staged_file_count,
        changes_file_count,
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
            .status
            .iter()
            .any(|entry| entry.path == relative_path && entry.index_status != "none");
    }

    snapshot.status.iter().any(|entry| {
        entry.path == relative_path
            && entry.worktree_status != "none"
            && entry.worktree_status != "ignored"
    })
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

fn resolve_review_target(
    worktree_path: &str,
    target: &ReviewTarget,
) -> Result<String, CodeUsecaseError> {
    let raw = match target {
        ReviewTarget::FileId(value) | ReviewTarget::Path(value) => value,
    };
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

fn apply_viewport(
    original: String,
    modified: String,
    viewport: Option<ReviewViewport>,
) -> (String, String, Option<ViewportDto>) {
    let Some(viewport) = viewport else {
        return (original, modified, None);
    };
    let start = viewport.start_line.max(1);
    let end = viewport.end_line;
    if end < start {
        return (
            String::new(),
            String::new(),
            Some(ViewportDto {
                start_line: start,
                end_line: end,
            }),
        );
    }
    (
        slice_lines(&original, start, end),
        slice_lines(&modified, start, end),
        Some(ViewportDto {
            start_line: start,
            end_line: end,
        }),
    )
}

fn slice_lines(content: &str, start_line: u32, end_line: u32) -> String {
    content
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, line)| {
            let line_no = index as u32 + 1;
            (line_no >= start_line && line_no <= end_line).then_some(line)
        })
        .collect()
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

pub(crate) fn review_blob_mime_for_path(path: &str) -> &'static str {
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
pub(crate) mod tests_support {
    use super::*;
    use crate::usecase::repository_state::snapshot::SnapshotFlags;

    pub(crate) fn review_usecase_with_snapshot_version(snapshot_version: u64) -> ReviewUsecase {
        ReviewUsecase::new_with_ports(
            Arc::new(StaticSnapshotProvider {
                snapshot: Arc::new(RepositorySnapshot {
                    version: snapshot_version,
                    flags: SnapshotFlags {
                        stale: false,
                        loading: false,
                        limited: false,
                    },
                    status: Vec::new(),
                    diff_stats: Vec::new(),
                    branch_cards: Vec::new(),
                    diff_file_tree: Vec::new(),
                    staged_diff_file_tree: Vec::new(),
                    changes_diff_file_tree: Vec::new(),
                }),
            }),
            Arc::new(PanicReviewCode),
        )
    }

    struct StaticSnapshotProvider {
        snapshot: Arc<RepositorySnapshot>,
    }

    impl ReviewSnapshotProvider for StaticSnapshotProvider {
        fn snapshot(
            &self,
            _worktree_path: &str,
        ) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
            Ok(self.snapshot.clone())
        }
    }

    struct PanicReviewCode;

    impl ReviewCodePort for PanicReviewCode {
        fn get_branch_diff_summary(
            &self,
            _repo_path: &str,
            _base_branch: Option<&str>,
        ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn build_diff_file_tree(&self, _entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn select_review_side_source(
            &self,
            _file_path: &str,
            _side: ReviewBlobSide,
            _section: ReviewSection,
            _base: ReviewBase,
        ) -> Result<SelectedReviewSide, CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn read_review_source_bytes(
            &self,
            _file_path: &str,
            _source: ReviewContentSource,
        ) -> Result<ReviewSideBytes, CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn review_binary_by_attributes(&self, _file_path: &str) -> Result<bool, CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn review_blob_url(
            &self,
            _worktree_path: &str,
            _path: &str,
            _side: ReviewBlobSide,
            _section: ReviewSection,
            _base: ReviewBase,
            _version: u64,
        ) -> String {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn compute_diff_hunks(
            &self,
            _original: &str,
            _modified: &str,
            _file_path: Option<&str>,
        ) -> DiffHunksResultDto {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn generate_group_patch(
            &self,
            _file_path: &str,
            _hunk: &Hunk,
            _group: &ChangeGroup,
        ) -> String {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn git_stage_hunk(&self, _repo_path: &str, _patch: &str) -> Result<(), CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }

        fn git_unstage_hunk(&self, _repo_path: &str, _patch: &str) -> Result<(), CodeUsecaseError> {
            panic!("review code port should not be called for stale review blob versions")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::*;
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

        fn review_blob_url(
            &self,
            _worktree_path: &str,
            path: &str,
            side: ReviewBlobSide,
            section: ReviewSection,
            base: ReviewBase,
            version: u64,
        ) -> String {
            let side = match side {
                ReviewBlobSide::Original => "original",
                ReviewBlobSide::Modified => "modified",
            };
            format!(
                "review-blob://localhost/blob?path={path}&side={side}&section={}&base={}&version={version}",
                section.as_str(),
                base.as_str()
            )
        }

        fn compute_diff_hunks(
            &self,
            original: &str,
            modified: &str,
            file_path: Option<&str>,
        ) -> DiffHunksResultDto {
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
            DiffHunksResultDto {
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
            }
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

        fn git_stage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeUsecaseError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("stage-hunk:{repo_path}"));
            Ok(())
        }

        fn git_unstage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeUsecaseError> {
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
                limited: false,
            },
            status: Vec::new(),
            diff_stats: Vec::new(),
            branch_cards: Vec::new(),
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
            branch_cards: Vec::new(),
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
    ) -> DiffHunksResultDto {
        let raw_hunks = crate::adaptor::gateway::code::diff_compute::diff_buffers(
            original, modified, file_path,
        );
        let hunks = crate::domain::code::services::hunk::assign_hunk_ids(&raw_hunks);
        let change_groups = crate::domain::code::services::hunk::compute_change_groups(&hunks);
        DiffHunksResultDto {
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
        }
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
        repository_snapshot_with_parts(
            version,
            SnapshotFlags {
                stale: false,
                loading: false,
                limited: false,
            },
            vec![file_status(path, index_status, worktree_status)],
            vec![diff_stat(path, 1, 0, 1, 0)],
            (index_status != "none")
                .then(|| tree_node(path, index_status, 1, 0))
                .into_iter()
                .collect(),
            (worktree_status != "none" && worktree_status != "ignored")
                .then(|| tree_node(path, worktree_status, 1, 0))
                .into_iter()
                .collect(),
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

    #[test]
    fn head_snapshot_is_composed_inside_review_usecase_from_repository_snapshot() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![
            repository_snapshot_with_parts(
                42,
                SnapshotFlags {
                    stale: true,
                    loading: true,
                    limited: true,
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
        assert!(dto.limited);
        assert_eq!(dto.files.len(), 2);
        assert_eq!(dto.files[0].file_id, "src/lib.rs");
        assert_eq!(dto.files[0].additions, 2);
        assert_eq!(dto.files[1].file_id, "src/main.rs");
        assert_eq!(dto.files[1].deletions, 4);
        assert_eq!(dto.staged_file_count, 1);
        assert_eq!(dto.changes_file_count, 1);
        assert!(code.calls().is_empty());
    }

    #[test]
    fn branch_base_snapshot_is_composed_inside_review_usecase_with_snapshot_flags() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![
            repository_snapshot_with_parts(
                55,
                SnapshotFlags {
                    stale: true,
                    loading: false,
                    limited: true,
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
        assert!(dto.limited);
        assert_eq!(dto.base, "branch-base");
        assert_eq!(dto.files.len(), 2);
        assert_eq!(dto.files[0].file_id, "src/feature.rs");
        assert_eq!(dto.files[0].index_status, "none");
        assert_eq!(dto.files[0].worktree_status, "modified");
        assert_eq!(dto.diff_stats[0].wt_additions, 5);
        assert!(dto.staged_tree.is_empty());
        assert_eq!(dto.changes_tree.len(), 2);
        assert_eq!(dto.tree.len(), 2);
        assert_eq!(dto.staged_file_count, 0);
        assert_eq!(dto.changes_file_count, 2);
        assert_eq!(code.calls(), vec!["branch-diff:/repo"]);
    }

    #[test]
    fn branch_base_snapshot_version_change_marks_result_stale_with_current_version() {
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

    #[test]
    fn review_file_view_resolves_file_id_and_path_and_switches_head_sections() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::FileId("src/app.rs".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(7),
                )
                .unwrap(),
        );
        let staged = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("src/app.rs".to_string()),
                    "staged",
                    "head",
                    None,
                    Some(7),
                )
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

    #[test]
    fn review_file_view_reports_added_deleted_and_unborn_head_as_text_sources() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("/repo/new.txt".to_string()),
                    "staged",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );
        let deleted = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::FileId("deleted.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );

        assert_eq!(added.original, "");
        assert_eq!(added.modified, "new file\n");
        assert_eq!(added.source, ReviewTextSource::Added);
        assert_eq!(deleted.original, "deleted file\n");
        assert_eq!(deleted.modified, "");
        assert_eq!(deleted.source, ReviewTextSource::Deleted);
    }

    #[test]
    fn review_file_view_applies_viewport_to_text_diff() {
        let path = "/repo/src/view.rs";
        let code = FakeReviewCode::new()
            .with_source_bytes(
                path,
                ReviewContentSource::Staged,
                present_text("old1\nold2\nold3\nold4\n"),
            )
            .with_source_bytes(
                path,
                ReviewContentSource::WorkingTree,
                present_text("new1\nnew2\nnew3\nnew4\n"),
            );
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                3,
                "src/view.rs",
                "modified",
                "modified",
            )],
            code,
        );

        let view = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("src/view.rs".to_string()),
                    "changes",
                    "head",
                    Some(ReviewViewport {
                        start_line: 2,
                        end_line: 3,
                    }),
                    Some(3),
                )
                .unwrap(),
        );

        assert_eq!(view.original, "old2\nold3\n");
        assert_eq!(view.modified, "new2\nnew3\n");
        assert!(view.limited);
        assert_eq!(
            view.viewport,
            Some(ViewportDto {
                start_line: 2,
                end_line: 3,
            })
        );
        assert_eq!(view.total_lines, 4);
    }

    #[test]
    fn review_file_view_viewport_stable_ids_use_full_file_occurrence_for_group_actions() {
        let path = "/repo/file.txt";
        let relative_path = "file.txt";
        let original = concat!(
            "c1\n", "c2\n", "c3\n", "a\n", "c4\n", "c5\n", "c6\n", "gap1\n", "gap2\n", "gap3\n",
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
                .with_source_bytes(path, ReviewContentSource::Staged, present_text(original))
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
                snapshot_for_group_action(3, relative_path, "stage"),
            ])),
            code.clone(),
        );

        let viewport_view = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path(relative_path.to_string()),
                    "changes",
                    "head",
                    Some(ReviewViewport {
                        start_line: 15,
                        end_line: 21,
                    }),
                    Some(1),
                )
                .unwrap(),
        );
        let full_view = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path(relative_path.to_string()),
                    "changes",
                    "head",
                    None,
                    Some(2),
                )
                .unwrap(),
        );

        assert_eq!(viewport_view.hunks.len(), 1);
        assert_eq!(viewport_view.change_groups.len(), 1);
        assert_eq!(full_view.hunks.len(), 2);
        assert_eq!(full_view.change_groups.len(), 2);
        assert_eq!(viewport_view.hunks[0].hunk_id, full_view.hunks[1].hunk_id);
        assert_eq!(
            viewport_view.change_groups[0].group_id,
            full_view.change_groups[1].group_id
        );

        let group_id = viewport_view.change_groups[0].group_id.clone();
        usecase
            .git_stage_review_group("/repo", relative_path, "changes", "head", &group_id)
            .unwrap();

        assert_eq!(
            code.calls(),
            vec![
                "generate-patch:file.txt:1:1".to_string(),
                "stage-hunk:/repo".to_string(),
            ]
        );
    }

    #[test]
    fn review_file_view_returns_image_and_binary_blob_urls_without_data_urls() {
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

        let image = usecase
            .get_review_file_view(
                "/repo",
                ReviewTarget::Path("assets/logo.png".to_string()),
                "changes",
                "head",
                None,
                Some(4),
            )
            .unwrap();
        let binary = usecase
            .get_review_file_view(
                "/repo",
                ReviewTarget::Path("assets/archive.bin".to_string()),
                "changes",
                "head",
                None,
                Some(4),
            )
            .unwrap();

        let ReviewFileViewDto::Image(image) = image else {
            panic!("expected image view");
        };
        assert_eq!(image.mime, "image/png");
        assert!(image
            .original_url
            .as_deref()
            .unwrap()
            .starts_with("review-blob://"));
        assert!(image
            .modified_url
            .as_deref()
            .unwrap()
            .starts_with("review-blob://"));
        assert!(!image.modified_url.as_deref().unwrap().starts_with("data:"));

        let ReviewFileViewDto::Binary(binary) = binary else {
            panic!("expected binary view");
        };
        assert_eq!(binary.original_size, Some(21));
        assert_eq!(binary.modified_size, Some(34));
        assert!(binary
            .original_url
            .as_deref()
            .unwrap()
            .contains("side=original"));
        assert!(binary
            .modified_url
            .as_deref()
            .unwrap()
            .contains("side=modified"));
        assert!(!binary.modified_url.as_deref().unwrap().starts_with("data:"));
    }

    #[test]
    fn review_file_view_returns_binary_for_nul_bytes_without_binary_attribute() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("assets/data.bin".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(6),
                )
                .unwrap(),
        );

        assert_eq!(binary.original_url, None);
        assert!(binary
            .modified_url
            .as_deref()
            .unwrap()
            .starts_with("review-blob://"));
        assert!(!binary.modified_url.as_deref().unwrap().starts_with("data:"));
    }

    #[test]
    fn review_file_view_returns_binary_for_non_utf8_bytes_without_binary_attribute() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("assets/non-utf8.dat".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(6),
                )
                .unwrap(),
        );

        assert_eq!(binary.original_url, None);
        assert!(binary
            .modified_url
            .as_deref()
            .unwrap()
            .starts_with("review-blob://"));
        assert!(!binary.modified_url.as_deref().unwrap().starts_with("data:"));
    }

    #[test]
    fn review_file_view_displays_deleted_file_under_removed_parent_directory() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("src/nested/file.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(9),
                )
                .unwrap(),
        );

        assert_eq!(view.original, "deleted content\n");
        assert_eq!(view.modified, "");
        assert_eq!(view.source, ReviewTextSource::Deleted);
    }

    #[test]
    fn review_file_view_applies_threshold_boundaries_in_usecase() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("exact-size.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let above_size = fallback_view_dto(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("above-size.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let exact_lines = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("exact-lines.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let above_lines = fallback_view_dto(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("above-lines.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let exact_hunks = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("exact-hunks.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let above_hunks = fallback_view_dto(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("above-hunks.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let exact_token = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("exact-token.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
                .unwrap(),
        );
        let above_token = fallback_view_dto(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("above-token.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(5),
                )
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

    #[test]
    fn review_stage_group_generates_patch_and_delegates_for_head_diff() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("file.txt".to_string()),
                    "changes",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );
        let group_id = view.change_groups[0].group_id.clone();

        usecase
            .git_stage_review_group("/repo", "file.txt", "changes", "head", &group_id)
            .unwrap();

        assert_eq!(
            code.calls(),
            vec!["generate-patch:file.txt:0:0", "stage-hunk:/repo"]
        );
    }

    #[test]
    fn review_unstage_group_generates_patch_and_delegates_for_head_diff() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path("file.txt".to_string()),
                    "staged",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );
        let group_id = view.change_groups[0].group_id.clone();

        usecase
            .git_unstage_review_group("/repo", "file.txt", "staged", "head", &group_id)
            .unwrap();

        assert_eq!(
            code.calls(),
            vec!["generate-patch:file.txt:0:0", "unstage-hunk:/repo"]
        );
    }

    #[test]
    fn review_group_actions_reject_branch_base_for_stage_and_unstage() {
        for action in ["stage", "unstage"] {
            let code = Arc::new(FakeReviewCode::new());
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(
                    1, false,
                )])),
                code.clone(),
            );

            let err = match action {
                "stage" => usecase.git_stage_review_group(
                    "/repo",
                    "file.txt",
                    "changes",
                    "branch-base",
                    "g:0",
                ),
                "unstage" => usecase.git_unstage_review_group(
                    "/repo",
                    "file.txt",
                    "changes",
                    "branch-base",
                    "g:0",
                ),
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

    #[test]
    fn review_group_actions_report_missing_group_for_stage_and_unstage() {
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
                "stage" => usecase.git_stage_review_group(
                    "/repo",
                    "file.txt",
                    "changes",
                    "head",
                    "missing-group",
                ),
                "unstage" => usecase.git_unstage_review_group(
                    "/repo",
                    "file.txt",
                    "changes",
                    "head",
                    "missing-group",
                ),
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

    #[test]
    fn review_group_action_missing_group_uses_typed_stale_target_error() {
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
            .unwrap_err();

        match err {
            CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { group_id }) => {
                assert_eq!(group_id, "missing-group");
            }
            other => panic!("expected stale review group target, got {other:?}"),
        }
    }

    #[test]
    fn review_group_actions_report_missing_snapshot_target_as_typed_stale_target_error() {
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
                    .get_review_file_view(
                        "/repo",
                        ReviewTarget::Path(relative_path.to_string()),
                        section,
                        "head",
                        None,
                        Some(1),
                    )
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            let err = match action {
                "stage" => usecase.git_stage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
                "unstage" => usecase.git_unstage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
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

    #[test]
    fn review_group_actions_accept_previous_group_id_after_snapshot_refresh_when_content_matches() {
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
                    .get_review_file_view(
                        "/repo",
                        ReviewTarget::Path(relative_path.to_string()),
                        section,
                        "head",
                        None,
                        Some(1),
                    )
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            match action {
                "stage" => usecase.git_stage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
                "unstage" => usecase.git_unstage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
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

    #[test]
    fn review_group_action_accepts_later_duplicate_group_id_after_earlier_duplicate_disappears() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path(relative_path.to_string()),
                    "changes",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );
        assert_eq!(view.change_groups.len(), 2);
        let later_group_id = view.change_groups[1].group_id.clone();

        usecase
            .git_stage_review_group("/repo", relative_path, "changes", "head", &later_group_id)
            .unwrap();

        assert_eq!(
            code.calls(),
            vec![
                "generate-patch:file.txt:0:0".to_string(),
                "stage-hunk:/repo".to_string(),
            ]
        );
    }

    #[test]
    fn review_file_view_keeps_later_duplicate_hunk_id_after_earlier_duplicate_disappears() {
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
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path(relative_path.to_string()),
                    "changes",
                    "head",
                    None,
                    Some(1),
                )
                .unwrap(),
        );
        assert_eq!(initial_view.hunks.len(), 2);
        let later_hunk_id = initial_view.hunks[1].hunk_id.clone();
        let refreshed_view = text_view(
            usecase
                .get_review_file_view(
                    "/repo",
                    ReviewTarget::Path(relative_path.to_string()),
                    "changes",
                    "head",
                    None,
                    Some(2),
                )
                .unwrap(),
        );

        assert_eq!(refreshed_view.hunks.len(), 1);
        assert_eq!(later_hunk_id, refreshed_view.hunks[0].hunk_id);
    }

    #[test]
    fn review_group_actions_reject_previous_group_id_after_snapshot_refresh_when_target_disappears()
    {
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
                    .get_review_file_view(
                        "/repo",
                        ReviewTarget::Path(relative_path.to_string()),
                        section,
                        "head",
                        None,
                        Some(1),
                    )
                    .unwrap(),
            );
            let group_id = view.change_groups[0].group_id.clone();

            let err = match action {
                "stage" => usecase.git_stage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
                "unstage" => usecase.git_unstage_review_group(
                    "/repo",
                    relative_path,
                    section,
                    "head",
                    &group_id,
                ),
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

    #[test]
    fn review_group_actions_report_missing_hunk_for_stage_and_unstage() {
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

            let err =
                match action {
                    "stage" => usecase
                        .git_stage_review_group("/repo", "file.txt", "changes", "head", "g:0"),
                    "unstage" => usecase
                        .git_unstage_review_group("/repo", "file.txt", "changes", "head", "g:0"),
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

    #[test]
    fn review_blob_rejects_stale_version_before_code_port() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(
            8, false,
        )]));
        let code = Arc::new(FakeReviewCode::new());
        let usecase = ReviewUsecase::new_with_ports(provider, code.clone());

        let err = usecase
            .read_review_blob_bytes(
                "/repo",
                "image.png",
                ReviewBlobSide::Modified,
                "changes",
                "head",
                7,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("stale review blob version: requested 7, current 8"));
        assert!(code.calls().is_empty());
    }

    #[test]
    fn review_blob_rejects_stale_version_with_typed_error() {
        let provider = Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(
            8, false,
        )]));
        let code = Arc::new(FakeReviewCode::new());
        let usecase = ReviewUsecase::new_with_ports(provider, code);

        let err = usecase
            .read_review_blob_bytes(
                "/repo",
                "image.png",
                ReviewBlobSide::Modified,
                "changes",
                "head",
                7,
            )
            .unwrap_err();

        match err {
            CodeUsecaseError::Code(CodeError::StaleReviewBlobVersion { requested, current }) => {
                assert_eq!(requested, 7);
                assert_eq!(current, 8);
            }
            other => panic!("expected stale review blob version, got {other:?}"),
        }
    }

    #[test]
    fn review_blob_returns_present_bytes_for_current_version() {
        let path = "/repo/image.png";
        let bytes = vec![1, 2, 3, 4];
        let code = FakeReviewCode::new().with_source_bytes(
            path,
            ReviewContentSource::WorkingTree,
            ReviewSideBytes::Present(bytes.clone()),
        );
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                8,
                "image.png",
                "none",
                "modified",
            )],
            code,
        );

        let result = usecase
            .read_review_blob_bytes(
                "/repo",
                "image.png",
                ReviewBlobSide::Modified,
                "changes",
                "head",
                8,
            )
            .unwrap();

        assert_eq!(result, bytes);
    }

    #[test]
    fn review_blob_rejects_missing_source_bytes() {
        let usecase = usecase_with_code(
            vec![snapshot_with_single_status(
                8,
                "image.png",
                "none",
                "modified",
            )],
            FakeReviewCode::new(),
        );

        let err = usecase
            .read_review_blob_bytes(
                "/repo",
                "image.png",
                ReviewBlobSide::Modified,
                "changes",
                "head",
                8,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("review blob not found: image.png"));
    }

    #[test]
    fn review_target_rejects_invalid_and_empty_paths() {
        for raw in ["../secret.txt", "src/../secret.txt", "/etc/passwd", "/"] {
            let err = resolve_review_target("/repo", &ReviewTarget::Path(raw.to_string()))
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("invalid review target path"),
                "unexpected error for {raw}: {err}"
            );
        }

        let err = resolve_review_target("/repo", &ReviewTarget::Path(String::new()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty review target path"));
    }

    #[test]
    fn review_target_must_belong_to_snapshot() {
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

    #[test]
    fn review_snapshot_contains_target_rejects_disallowed_sections_and_statuses() {
        let branch_snapshot = ReviewSnapshotDto {
            version: 1,
            stale: false,
            loading: false,
            limited: false,
            base: "branch-base".to_string(),
            files: vec![ReviewFileEntryDto {
                file_id: "src/app.rs".to_string(),
                path: "src/app.rs".to_string(),
                index_status: "none".to_string(),
                worktree_status: "modified".to_string(),
                additions: 1,
                deletions: 0,
            }],
            status: Vec::new(),
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
                    limited: false,
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

    #[test]
    fn review_blob_mime_mapping_lives_in_usecase() {
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
