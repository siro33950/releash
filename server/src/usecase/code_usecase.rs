//! code ドメインの Command 側ユースケース（差分 Approve = staging）と、読み取りの集約入口。
//!
//! controller はこの Usecase だけを入口とする。書き込み（staging）に加え、ファイル内容
//! 参照・diff 閲覧・hunk/patch/range 算出・diff_tree・language・mention 候補列挙といった
//! 読み取り／純粋計算も、読み取りクエリサービス（協力者）へ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::code::services::directory::GitStatus;
use crate::domain::code::services::filtered_files::FilteredFilesCollector;
use crate::domain::code::value_objects::file_list::{FileFilter, FilteredFiles};
use crate::domain::code::{
    ReviewBase, ReviewBlobSide, ReviewSideBytes, ReviewSideMetadata, StagingRepository,
};

use super::code_dto::DiffHunksResultDto;
use super::code_error::CodeUsecaseError;
use super::code_query_service::CodeQueryService;

#[derive(Clone)]
pub struct CodeUsecase {
    staging: Arc<dyn StagingRepository>,
    query: CodeQueryService,
    directory: Arc<dyn super::directory_query::DirectoryQuery>,
}

pub(super) use crate::domain::code::value_objects::review::ReviewContentSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelectedReviewSide {
    pub source: ReviewContentSource,
    pub metadata: ReviewSideMetadata,
}

fn parse_statuses(
    statuses: &[super::repository_dto::FileStatusDto],
) -> Result<Vec<(&str, GitStatus, GitStatus)>, crate::domain::code::CodeError> {
    statuses
        .iter()
        .map(|entry| {
            Ok((
                entry.path.as_str(),
                GitStatus::parse(&entry.index_status)?,
                GitStatus::parse(&entry.worktree_status)?,
            ))
        })
        .collect()
}

impl CodeUsecase {
    pub fn new(
        staging: Arc<dyn StagingRepository>,
        query: CodeQueryService,
        directory: Arc<dyn super::directory_query::DirectoryQuery>,
    ) -> Self {
        Self {
            staging,
            query,
            directory,
        }
    }

    pub(super) fn file_target(
        &self,
        worktree: &str,
        file: &str,
    ) -> Result<String, CodeUsecaseError> {
        self.directory.file_target(worktree, file)
    }

    pub fn directory_entries(
        &self,
        worktree: &str,
        directory: &str,
        statuses: &[super::repository_dto::FileStatusDto],
    ) -> Result<Vec<super::directory_query::DirectoryEntryDto>, CodeUsecaseError> {
        use crate::domain::code::services::directory::{entry_order, entry_status};
        let statuses = parse_statuses(statuses)?;
        let mut entries: Vec<_> = self
            .directory
            .entries(worktree, directory)?
            .into_iter()
            .map(|entry| super::directory_query::DirectoryEntryDto {
                status: entry_status(&entry.path, entry.directory, statuses.iter().copied()),
                path: entry.path,
                name: entry.name,
                directory: entry.directory,
                ignored: entry.ignored,
            })
            .collect();
        entries.sort_by(|a, b| entry_order(a.directory, &a.name, b.directory, &b.name));
        Ok(entries)
    }

    /// worktree の項目を列挙し、条件で絞り込んだファイル一覧を返す。
    pub fn filtered_files(
        &self,
        worktree: &str,
        filter: &FileFilter,
        statuses: &[super::repository_dto::FileStatusDto],
    ) -> Result<FilteredFiles, CodeUsecaseError> {
        let statuses = parse_statuses(statuses)?;
        let mut collector = FilteredFilesCollector::new(filter.clone(), statuses.iter().copied());
        self.directory
            .walk(worktree, &mut |entry| collector.offer(entry))?;
        Ok(collector.finish())
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

    // ── ファイル内容参照（読み取り → QueryService へ委譲） ──

    // ── branch diff ──

    // ── review primitives（snapshot/version orchestration は ReviewUsecase が持つ） ──

    pub(super) fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError> {
        self.select_review_source(file_path, ReviewContentSource::select(side, base))
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
            ReviewContentSource::BranchBaseTip => self
                .query
                .review_file_metadata_at_branch_base_tip(file_path),
            ReviewContentSource::Head => self.query.review_file_metadata_at_ref(file_path, "HEAD"),
            ReviewContentSource::WorkingTree => self.query.review_working_tree_metadata(file_path),
        }
    }

    pub(super) fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        match source {
            ReviewContentSource::BranchBaseTip => {
                self.query.review_file_bytes_at_branch_base_tip(file_path)
            }
            ReviewContentSource::Head => self.query.review_file_bytes_at_ref(file_path, "HEAD"),
            ReviewContentSource::WorkingTree => self.query.review_working_tree_bytes(file_path),
        }
    }

    pub(super) fn review_working_tree_matches_source(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<bool, CodeUsecaseError> {
        self.query
            .review_working_tree_matches_source(file_path, source)
    }

    pub(super) fn review_binary_by_attributes(
        &self,
        file_path: &str,
    ) -> Result<bool, CodeUsecaseError> {
        self.query.review_binary_by_attributes(file_path)
    }

    // ── diff tree ──

    // ── hunk / patch / range ──

    pub fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        self.query.compute_diff_hunks(original, modified, file_path)
    }

    // ── language ──
}
