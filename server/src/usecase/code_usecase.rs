//! code ドメインの Command 側ユースケース（差分 Approve = staging）と、読み取りの集約入口。
//!
//! controller はこの Usecase だけを入口とする。書き込み（staging）に加え、ファイル内容
//! 参照・diff 閲覧・hunk/patch/range 算出・diff_tree・language・mention 候補列挙といった
//! 読み取り／純粋計算も、読み取りクエリサービス（協力者）へ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::code::{
    ChangeGroup, DiffFileEntry, DiffSide, DiffTreeNode, Hunk, ReviewBase, ReviewBlobSide,
    ReviewSection, ReviewSideBytes, ReviewSideMetadata, StagingRepository,
};

use super::code_dto::{
    BranchDiffSummaryDto, DiffHunksResultDto, DiffRangeDto, DiffTreeNodeDto,
    FileNavigationResultDto, HiddenRangeDto, InlineChunkDto, SplitRowDto, VisibleBlockDto,
};
use super::code_error::CodeUsecaseError;
use super::code_query_service::CodeQueryService;

#[derive(Clone)]
pub struct CodeUsecase {
    staging: Arc<dyn StagingRepository>,
    query: CodeQueryService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewContentSource {
    BranchBase,
    Head,
    Staged,
    WorkingTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelectedReviewSide {
    pub source: ReviewContentSource,
    pub metadata: ReviewSideMetadata,
}

impl CodeUsecase {
    pub fn new(staging: Arc<dyn StagingRepository>, query: CodeQueryService) -> Self {
        Self { staging, query }
    }

    // ── staging（書き込み = 差分 Approve） ──

    pub fn git_stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeUsecaseError> {
        self.staging.stage(repo_path, paths)?;
        Ok(())
    }

    pub fn git_unstage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeUsecaseError> {
        self.staging.unstage(repo_path, paths)?;
        Ok(())
    }

    pub async fn git_stage_hunk(
        &self,
        repo_path: &str,
        patch: &str,
    ) -> Result<(), CodeUsecaseError> {
        self.staging.stage_hunk(repo_path, patch).await?;
        Ok(())
    }

    pub async fn git_unstage_hunk(
        &self,
        repo_path: &str,
        patch: &str,
    ) -> Result<(), CodeUsecaseError> {
        self.staging.unstage_hunk(repo_path, patch).await?;
        Ok(())
    }

    // ── ファイル内容参照（読み取り → QueryService へ委譲） ──

    // ── branch diff ──

    pub fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
        self.query.get_branch_diff_summary(repo_path, base_branch)
    }

    // ── review primitives（snapshot/version orchestration は ReviewUsecase が持つ） ──

    pub(super) fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError> {
        if base.is_branch_base() {
            let source = match side {
                ReviewBlobSide::Original => ReviewContentSource::BranchBase,
                ReviewBlobSide::Modified => ReviewContentSource::WorkingTree,
            };
            return self.select_review_source(file_path, source);
        }
        if section.is_staged() {
            let source = match side {
                ReviewBlobSide::Original => ReviewContentSource::Head,
                ReviewBlobSide::Modified => ReviewContentSource::Staged,
            };
            return self.select_review_source(file_path, source);
        }

        match side {
            ReviewBlobSide::Original => {
                self.select_review_source(file_path, ReviewContentSource::Staged)
            }
            ReviewBlobSide::Modified => {
                self.select_review_source(file_path, ReviewContentSource::WorkingTree)
            }
        }
    }

    fn select_review_source(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<SelectedReviewSide, CodeUsecaseError> {
        Ok(SelectedReviewSide {
            source,
            metadata: self.read_review_source_metadata(file_path, source)?,
        })
    }

    fn read_review_source_metadata(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideMetadata, CodeUsecaseError> {
        match source {
            ReviewContentSource::BranchBase => {
                self.query.review_file_metadata_at_branch_base(file_path)
            }
            ReviewContentSource::Head => self.query.review_file_metadata_at_ref(file_path, "HEAD"),
            ReviewContentSource::Staged => self.query.review_staged_metadata(file_path),
            ReviewContentSource::WorkingTree => self.query.review_working_tree_metadata(file_path),
        }
    }

    pub(super) fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        match source {
            ReviewContentSource::BranchBase => {
                self.query.review_file_bytes_at_branch_base(file_path)
            }
            ReviewContentSource::Head => self.query.review_file_bytes_at_ref(file_path, "HEAD"),
            ReviewContentSource::Staged => self.query.review_staged_bytes(file_path),
            ReviewContentSource::WorkingTree => self.query.review_working_tree_bytes(file_path),
        }
    }

    pub(super) fn review_binary_by_attributes(
        &self,
        file_path: &str,
    ) -> Result<bool, CodeUsecaseError> {
        self.query.review_binary_by_attributes(file_path)
    }

    // ── diff tree ──

    pub fn build_diff_file_tree(&self, entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto> {
        self.query.build_diff_file_tree(entries)
    }

    pub fn get_file_navigation(
        &self,
        tree: &[DiffTreeNode],
        current_file: &str,
    ) -> FileNavigationResultDto {
        self.query.get_file_navigation(tree, current_file)
    }

    // ── hunk / patch / range ──

    pub fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        self.query.compute_diff_hunks(original, modified, file_path)
    }

    pub fn generate_group_patch(
        &self,
        file_path: &str,
        hunk: &Hunk,
        group: &ChangeGroup,
    ) -> String {
        self.query.generate_group_patch(file_path, hunk, group)
    }

    pub fn compute_hidden_ranges(
        &self,
        hunks: &[Hunk],
        total_lines: u32,
        context_lines: u32,
    ) -> Vec<HiddenRangeDto> {
        self.query
            .compute_hidden_ranges(hunks, total_lines, context_lines)
    }

    pub fn compute_hidden_ranges_from_content(
        &self,
        original: &str,
        modified: &str,
        context_lines: u32,
    ) -> Result<Vec<HiddenRangeDto>, CodeUsecaseError> {
        self.query
            .compute_hidden_ranges_from_content(original, modified, context_lines)
    }

    pub fn compute_visible_markdown_blocks(
        &self,
        original: &str,
        modified: &str,
        context_lines: u32,
    ) -> Result<Vec<VisibleBlockDto>, CodeUsecaseError> {
        self.query
            .compute_visible_markdown_blocks(original, modified, context_lines)
    }

    pub fn compute_markdown_diff_ranges(
        &self,
        original: &str,
        modified: &str,
        side: DiffSide,
    ) -> Result<Vec<DiffRangeDto>, CodeUsecaseError> {
        self.query
            .compute_markdown_diff_ranges(original, modified, side)
    }

    pub fn compute_markdown_split_rows(
        &self,
        original: &str,
        modified: &str,
    ) -> Result<Vec<SplitRowDto>, CodeUsecaseError> {
        self.query.compute_markdown_split_rows(original, modified)
    }

    pub fn compute_markdown_inline_chunks(
        &self,
        original: &str,
        modified: &str,
    ) -> Result<Vec<InlineChunkDto>, CodeUsecaseError> {
        self.query
            .compute_markdown_inline_chunks(original, modified)
    }

    // ── language ──

    pub fn get_language_from_path(&self, file_path: &str) -> String {
        self.query.get_language_from_path(file_path)
    }
}

#[cfg(test)]
#[path = "code_usecase_test.rs"]
mod code_usecase_tests;
