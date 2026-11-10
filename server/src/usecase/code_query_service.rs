//! code ドメインの読み取りクエリサービス（read model 生成・純粋算出の orchestration）。
//!
//! **QueryService は Usecase ではない。** ファイル内容参照・diff 閲覧・hunk/patch/range
//! 算出・diff_tree・language・mention 候補列挙といった読み取り／純粋計算をまとめる協力者で
//! あり、業務手順（オーケストレーション）は持たない。diff バッファ計算（git2 依存）は
//! `DiffComputer` gateway に委譲し、hunk 区切り・range 算出・patch 生成・tree 構築・language
//! 判定はドメインサービス（純粋関数）に委譲する。

use std::sync::Arc;

use crate::domain::code::{
    BranchBaseResolver, CodeError, DiffComputer, FileContentRepository, Hunk, ReviewSideBytes,
    ReviewSideMetadata,
};

use super::code_dto::{DiffHunksResultDto, HunkDto};
use super::code_error::CodeUsecaseError;

/// read model 構築・純粋算出を担う読み取りクエリサービス。
#[derive(Clone)]
pub struct CodeQueryService {
    file_content: Arc<dyn FileContentRepository>,
    diff_computer: Arc<dyn DiffComputer>,
    branch_base: Arc<dyn BranchBaseResolver>,
}

impl CodeQueryService {
    pub fn new(
        file_content: Arc<dyn FileContentRepository>,
        diff_computer: Arc<dyn DiffComputer>,
        branch_base: Arc<dyn BranchBaseResolver>,
    ) -> Self {
        Self {
            file_content,
            diff_computer,
            branch_base,
        }
    }

    // ── ファイル内容参照 ──

    /// 現在ブランチの base 名を解決し、その base コミット OID(hex) を返す。base 名は
    /// 解決できるが ref が実在しない場合は、移行前の gateway（`find_merge_base_commit` /
    /// `find_base_commit`）と等価に `base branch '{name}' not found` を返す。detached /
    /// base 未設定（名前が `None`）は HEAD フォールバック用に `None` を返す。
    fn resolve_base_commit_oid_for(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, CodeUsecaseError> {
        match self.branch_base.resolve_base_branch_name(path_hint)? {
            Some(name) => match self.branch_base.resolve_base_commit_oid(path_hint, &name)? {
                Some(oid) => Ok(Some(oid)),
                None => Err(CodeError::Rule(format!("base branch '{name}' not found")).into()),
            },
            None => Ok(None),
        }
    }

    pub fn review_file_metadata_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideMetadata, CodeUsecaseError> {
        Ok(self
            .file_content
            .review_file_metadata_at_ref(file_path, git_ref)?)
    }

    pub fn review_file_bytes_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        Ok(self
            .file_content
            .review_file_bytes_at_ref(file_path, git_ref)?)
    }

    pub fn review_file_metadata_at_branch_base_tip(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideMetadata, CodeUsecaseError> {
        let base_oid = self.resolve_base_commit_oid_for(file_path)?;
        Ok(self
            .file_content
            .review_file_metadata_at_ref(file_path, base_oid.as_deref().unwrap_or("HEAD"))?)
    }

    pub fn review_file_bytes_at_branch_base_tip(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        let base_oid = self.resolve_base_commit_oid_for(file_path)?;
        Ok(self
            .file_content
            .review_file_bytes_at_ref(file_path, base_oid.as_deref().unwrap_or("HEAD"))?)
    }

    pub fn review_working_tree_metadata(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideMetadata, CodeUsecaseError> {
        Ok(self.file_content.review_working_tree_metadata(file_path)?)
    }

    pub fn review_working_tree_bytes(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        Ok(self.file_content.review_working_tree_bytes(file_path)?)
    }

    pub fn review_working_tree_matches_source(
        &self,
        file_path: &str,
        source: crate::domain::code::value_objects::review::ReviewContentSource,
    ) -> Result<bool, CodeUsecaseError> {
        use crate::domain::code::value_objects::review::ReviewContentSource;
        let git_ref = match source {
            ReviewContentSource::Head => "HEAD".to_owned(),
            ReviewContentSource::BranchBaseTip => self
                .resolve_base_commit_oid_for(file_path)?
                .unwrap_or_else(|| "HEAD".into()),
            _ => {
                return Err(crate::domain::code::CodeError::Rule(
                    "file comparison requires HEAD or branch base tip".into(),
                )
                .into())
            }
        };
        Ok(self
            .file_content
            .review_working_tree_matches_ref(file_path, &git_ref)?)
    }

    pub fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError> {
        Ok(self.file_content.review_binary_by_attributes(file_path)?)
    }

    // ── branch diff ──

    // ── diff tree（純粋） ──

    // ── hunk / patch / range ──

    pub fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        let hunks = self
            .diff_computer
            .diff_buffers(original, modified, file_path)?;
        Ok(DiffHunksResultDto {
            hunks: hunks.iter().map(hunk_to_dto).collect(),
        })
    }

    // ── language（純粋） ──
}

// ── 既存 domain VO → DTO 変換（hunk/range などの共通 read model） ──

fn hunk_to_dto(h: &Hunk) -> HunkDto {
    HunkDto {
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h.lines.clone(),
    }
}

#[cfg(test)]
#[path = "code_query_service_test.rs"]
mod code_query_service_tests;
