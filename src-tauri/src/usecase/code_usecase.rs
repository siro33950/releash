//! code ドメインの Command 側ユースケース（差分 Approve = staging）と、読み取りの集約入口。
//!
//! controller はこの Usecase だけを入口とする。書き込み（staging）に加え、ファイル内容
//! 参照・diff 閲覧・hunk/patch/range 算出・diff_tree・language・mention 候補列挙といった
//! 読み取り／純粋計算も、読み取りクエリサービス（協力者）へ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::code::{
    ChangeGroup, DiffFileEntry, DiffTreeNode, Hunk, MentionReference, ReviewBase, ReviewBlobSide,
    ReviewBlobUrlParams, ReviewBlobUrlProvider, ReviewSection, ReviewSideBytes, ReviewSideMetadata,
    StagingRepository,
};

use super::code_dto::{
    BranchDiffSummaryDto, DiffHunksResultDto, DiffTreeNodeDto, FileNavigationResultDto,
    HiddenRangeDto, VisibleBlockDto,
};
use super::code_error::CodeUsecaseError;
use super::code_query_service::CodeQueryService;

#[derive(Clone)]
pub struct CodeUsecase {
    staging: Arc<dyn StagingRepository>,
    query: CodeQueryService,
    blob_urls: Arc<dyn ReviewBlobUrlProvider>,
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
    pub fn new(
        staging: Arc<dyn StagingRepository>,
        query: CodeQueryService,
        blob_urls: Arc<dyn ReviewBlobUrlProvider>,
    ) -> Self {
        Self {
            staging,
            query,
            blob_urls,
        }
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

    pub fn git_stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        self.staging.stage_hunk(repo_path, patch)?;
        Ok(())
    }

    pub fn git_unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeUsecaseError> {
        self.staging.unstage_hunk(repo_path, patch)?;
        Ok(())
    }

    // ── ファイル内容参照（読み取り → QueryService へ委譲） ──

    pub fn get_file_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<String, CodeUsecaseError> {
        self.query.get_file_at_ref(file_path, git_ref)
    }

    pub fn get_binary_file_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<String, CodeUsecaseError> {
        self.query.get_binary_file_at_ref(file_path, git_ref)
    }

    pub fn get_file_at_branch_base(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        self.query.get_file_at_branch_base(file_path)
    }

    pub fn get_binary_file_at_branch_base(
        &self,
        file_path: &str,
    ) -> Result<String, CodeUsecaseError> {
        self.query.get_binary_file_at_branch_base(file_path)
    }

    pub fn get_staged_content(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        self.query.get_staged_content(file_path)
    }

    pub fn get_binary_staged_content(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        self.query.get_binary_staged_content(file_path)
    }

    // ── branch diff ──

    pub fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
        self.query.get_branch_diff_summary(repo_path, base_branch)
    }

    // ── review primitives（snapshot/version orchestration は ReviewUsecase が持つ） ──

    pub(super) fn review_blob_url(
        &self,
        worktree_path: &str,
        path: &str,
        side: ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
        version: u64,
    ) -> String {
        self.blob_urls.url(&ReviewBlobUrlParams {
            worktree_path: worktree_path.to_string(),
            path: path.to_string(),
            side,
            section: section.as_str().to_string(),
            base: base.as_str().to_string(),
            version,
        })
    }

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
    ) -> DiffHunksResultDto {
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
    ) -> Vec<HiddenRangeDto> {
        self.query
            .compute_hidden_ranges_from_content(original, modified, context_lines)
    }

    pub fn compute_visible_markdown_blocks(
        &self,
        original: &str,
        modified: &str,
        context_lines: u32,
    ) -> Vec<VisibleBlockDto> {
        self.query
            .compute_visible_markdown_blocks(original, modified, context_lines)
    }

    // ── language ──

    pub fn get_language_from_path(&self, file_path: &str) -> String {
        self.query.get_language_from_path(file_path)
    }

    pub fn get_relative_path(&self, root_path: &str, file_path: &str) -> Option<String> {
        self.query.get_relative_path(root_path, file_path)
    }

    // ── branch base 名解決（外部入口向け） ──

    /// 現在ブランチの実効 base 名（ref 実在検証あり、未解決は `None`）。agent bridge が
    /// gateway 実装へ直接依存せずに base 名を得るための入口。
    pub fn resolve_effective_base_branch_name(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, CodeUsecaseError> {
        self.query.resolve_effective_base_branch_name(path_hint)
    }

    // ── file mention（候補列挙・参照解決） ──

    pub fn list_mentionable_files(
        &self,
        worktree_path: &str,
        query: &str,
    ) -> Result<Vec<String>, CodeUsecaseError> {
        self.query.list_mentionable_files(worktree_path, query)
    }

    pub async fn read_codex_mentionable_files(
        &self,
        worktree_path: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, CodeUsecaseError> {
        self.query
            .read_codex_mentionable_files(worktree_path, query, limit)
            .await
    }

    /// 構造化メンション参照を解決し file_context を本文先頭へ前置する。メンションが空、
    /// または解決失敗時は警告ログを出して本文をそのまま返す（移行前のフォールバック挙動を維持）。
    pub fn resolve_mentions_or_fallback(
        &self,
        worktree_path: &str,
        content: &str,
        mentions: &[MentionReference],
    ) -> String {
        if mentions.is_empty() {
            return content.to_string();
        }
        self.query
            .resolve_mentions(worktree_path, content, mentions)
            .unwrap_or_else(|e| {
                log::warn!("Failed to resolve mentions: {e}");
                content.to_string()
            })
    }
}

#[cfg(test)]
mod code_usecase_tests {
    use super::*;
    use crate::domain::code::{
        BranchBaseResolver, CodeError, DiffComputer, FileContentRepository, MentionRepository,
    };
    use crate::usecase::code_query_service::{BranchDiffQuery, CodexFuzzyFileSearchGateway};
    use std::sync::Mutex;

    struct RecordingStaging {
        calls: Mutex<Vec<String>>,
    }
    impl StagingRepository for RecordingStaging {
        fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("stage:{repo_path}:{}", paths.join(",")));
            Ok(())
        }
        fn unstage(&self, repo_path: &str, _paths: Vec<String>) -> Result<(), CodeError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("unstage:{repo_path}"));
            Ok(())
        }
        fn stage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("stage_hunk:{repo_path}"));
            Ok(())
        }
        fn unstage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("unstage_hunk:{repo_path}"));
            Ok(())
        }
    }

    struct StubFileContent;
    impl FileContentRepository for StubFileContent {
        fn file_at_ref(&self, _f: &str, _r: &str) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn binary_file_at_ref(&self, _f: &str, _r: &str) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn file_at_branch_base(&self, _f: &str, _b: Option<&str>) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn binary_file_at_branch_base(
            &self,
            _f: &str,
            _b: Option<&str>,
        ) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn staged_content(&self, _f: &str) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn binary_staged_content(&self, _f: &str) -> Result<String, CodeError> {
            Ok("c".to_string())
        }
        fn review_file_metadata_at_ref(
            &self,
            _f: &str,
            _r: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_file_bytes_at_ref(
            &self,
            _f: &str,
            _r: &str,
        ) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(b"c".to_vec()))
        }
        fn review_file_metadata_at_branch_base(
            &self,
            _f: &str,
            _b: Option<&str>,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_file_bytes_at_branch_base(
            &self,
            _f: &str,
            _b: Option<&str>,
        ) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(b"c".to_vec()))
        }
        fn review_staged_metadata(&self, _f: &str) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Missing)
        }
        fn review_staged_bytes(&self, _f: &str) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Missing)
        }
        fn review_working_tree_metadata(
            &self,
            file_path: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            match std::fs::metadata(file_path) {
                Ok(metadata) => Ok(ReviewSideMetadata::Present {
                    size_bytes: metadata.len(),
                }),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(ReviewSideMetadata::Missing)
                }
                Err(e) => Err(CodeError::from(e)),
            }
        }
        fn review_working_tree_bytes(&self, file_path: &str) -> Result<ReviewSideBytes, CodeError> {
            match std::fs::read(file_path) {
                Ok(bytes) => Ok(ReviewSideBytes::Present(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReviewSideBytes::Missing),
                Err(e) => Err(CodeError::from(e)),
            }
        }
        fn review_binary_by_attributes(&self, _f: &str) -> Result<bool, CodeError> {
            Ok(false)
        }
    }

    #[derive(Default)]
    struct RecordingFileContent {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingFileContent {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl FileContentRepository for RecordingFileContent {
        fn file_at_ref(&self, f: &str, r: &str) -> Result<String, CodeError> {
            self.record(format!("text:ref:{r}:{f}"));
            Ok("c".to_string())
        }
        fn binary_file_at_ref(&self, f: &str, r: &str) -> Result<String, CodeError> {
            self.record(format!("binary:ref:{r}:{f}"));
            Ok("c".to_string())
        }
        fn file_at_branch_base(&self, f: &str, b: Option<&str>) -> Result<String, CodeError> {
            self.record(format!("text:branch-base:{}:{f}", b.unwrap_or("none")));
            Ok("c".to_string())
        }
        fn binary_file_at_branch_base(
            &self,
            f: &str,
            b: Option<&str>,
        ) -> Result<String, CodeError> {
            self.record(format!("binary:branch-base:{}:{f}", b.unwrap_or("none")));
            Ok("c".to_string())
        }
        fn staged_content(&self, f: &str) -> Result<String, CodeError> {
            self.record(format!("text:staged:{f}"));
            Ok("c".to_string())
        }
        fn binary_staged_content(&self, f: &str) -> Result<String, CodeError> {
            self.record(format!("binary:staged:{f}"));
            Ok("c".to_string())
        }
        fn review_file_metadata_at_ref(
            &self,
            f: &str,
            r: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            self.record(format!("metadata:ref:{r}:{f}"));
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_file_bytes_at_ref(&self, f: &str, r: &str) -> Result<ReviewSideBytes, CodeError> {
            self.record(format!("bytes:ref:{r}:{f}"));
            Ok(ReviewSideBytes::Present(b"head".to_vec()))
        }
        fn review_file_metadata_at_branch_base(
            &self,
            f: &str,
            b: Option<&str>,
        ) -> Result<ReviewSideMetadata, CodeError> {
            self.record(format!("metadata:branch-base:{}:{f}", b.unwrap_or("none")));
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_file_bytes_at_branch_base(
            &self,
            f: &str,
            b: Option<&str>,
        ) -> Result<ReviewSideBytes, CodeError> {
            self.record(format!("bytes:branch-base:{}:{f}", b.unwrap_or("none")));
            Ok(ReviewSideBytes::Present(b"base".to_vec()))
        }
        fn review_staged_metadata(&self, f: &str) -> Result<ReviewSideMetadata, CodeError> {
            self.record(format!("metadata:staged:{f}"));
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_staged_bytes(&self, f: &str) -> Result<ReviewSideBytes, CodeError> {
            self.record(format!("bytes:staged:{f}"));
            Ok(ReviewSideBytes::Present(b"staged".to_vec()))
        }
        fn review_working_tree_metadata(&self, f: &str) -> Result<ReviewSideMetadata, CodeError> {
            self.record(format!("metadata:working-tree:{f}"));
            Ok(ReviewSideMetadata::Present { size_bytes: 1 })
        }
        fn review_working_tree_bytes(&self, f: &str) -> Result<ReviewSideBytes, CodeError> {
            self.record(format!("bytes:working-tree:{f}"));
            Ok(ReviewSideBytes::Present(b"working".to_vec()))
        }
        fn review_binary_by_attributes(&self, _f: &str) -> Result<bool, CodeError> {
            Ok(false)
        }
    }

    struct StubBlobUrls;
    impl ReviewBlobUrlProvider for StubBlobUrls {
        fn url(&self, params: &ReviewBlobUrlParams) -> String {
            let side = match params.side {
                ReviewBlobSide::Original => "original",
                ReviewBlobSide::Modified => "modified",
            };
            format!(
                "review-blob://localhost/blob?side={side}&version={}",
                params.version
            )
        }
    }

    struct StubDiffComputer;
    impl DiffComputer for StubDiffComputer {
        fn diff_buffers(&self, _o: &str, _m: &str, _f: Option<&str>) -> Vec<Hunk> {
            Vec::new()
        }
    }

    struct StubBranchDiff;
    impl BranchDiffQuery for StubBranchDiff {
        fn summary(
            &self,
            _repo_path: &str,
            _base_name: Option<&str>,
            _base_commit_oid: Option<&str>,
        ) -> Result<BranchDiffSummaryDto, CodeError> {
            Ok(BranchDiffSummaryDto {
                base_branch: String::new(),
                changed_files: vec![],
                stats: crate::usecase::code_dto::DiffStatsDto {
                    additions: 0,
                    deletions: 0,
                },
            })
        }
    }

    struct StubMention;
    impl MentionRepository for StubMention {
        fn list_mentionable_files(&self, _w: &str, _q: &str) -> Result<Vec<String>, CodeError> {
            Ok(vec![])
        }
        fn resolve_mentions(
            &self,
            _w: &str,
            content: &str,
            _m: &[MentionReference],
        ) -> Result<String, CodeError> {
            Ok(content.to_string())
        }
    }

    /// メンション解決が成功し file_context を前置するケースを表す stub。
    /// 実際のファイル読み込み・抜粋・不在スキップ等の解決ロジックは mention gateway の
    /// テスト（`mention_gateway_tests`）が担保するため、usecase 層テストは解決成功時の
    /// オーケストレーション（結果をそのまま返す）のみを stub で担保する。
    struct ResolvingMention;
    impl MentionRepository for ResolvingMention {
        fn list_mentionable_files(&self, _w: &str, _q: &str) -> Result<Vec<String>, CodeError> {
            Ok(vec![])
        }
        fn resolve_mentions(
            &self,
            _w: &str,
            content: &str,
            _m: &[MentionReference],
        ) -> Result<String, CodeError> {
            Ok(format!("<file_context>\n</file_context>\n\n{content}"))
        }
    }

    /// メンション解決が失敗するケースを表す stub（usecase のフォールバック方針を担保）。
    struct FailingMention;
    impl MentionRepository for FailingMention {
        fn list_mentionable_files(&self, _w: &str, _q: &str) -> Result<Vec<String>, CodeError> {
            Ok(vec![])
        }
        fn resolve_mentions(
            &self,
            _w: &str,
            _content: &str,
            _m: &[MentionReference],
        ) -> Result<String, CodeError> {
            Err(CodeError::Rule("mention resolution failed".to_string()))
        }
    }

    struct StubBranchBase;
    impl BranchBaseResolver for StubBranchBase {
        fn resolve_base_branch_name(&self, _p: &str) -> Result<Option<String>, CodeError> {
            Ok(None)
        }
        fn resolve_effective_base_branch_name(
            &self,
            _p: &str,
        ) -> Result<Option<String>, CodeError> {
            Ok(None)
        }
        fn resolve_base_commit_oid(
            &self,
            _path_hint: &str,
            _base_name: &str,
        ) -> Result<Option<String>, CodeError> {
            Ok(None)
        }
    }

    struct StubCodexFuzzyFileSearch;
    #[async_trait::async_trait]
    impl CodexFuzzyFileSearchGateway for StubCodexFuzzyFileSearch {
        async fn search_files(
            &self,
            _worktree_path: &str,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<String>, CodeError> {
            Ok(vec!["src/main.rs".to_string()])
        }
    }

    fn usecase(staging: Arc<RecordingStaging>) -> CodeUsecase {
        let query = CodeQueryService::new(
            Arc::new(StubFileContent),
            Arc::new(StubDiffComputer),
            Arc::new(StubBranchDiff),
            Arc::new(StubMention),
            Arc::new(StubBranchBase),
            Arc::new(StubCodexFuzzyFileSearch),
        );
        CodeUsecase::new(staging, query, Arc::new(StubBlobUrls))
    }

    fn usecase_with_file_content(file_content: Arc<dyn FileContentRepository>) -> CodeUsecase {
        let query = CodeQueryService::new(
            file_content,
            Arc::new(StubDiffComputer),
            Arc::new(StubBranchDiff),
            Arc::new(StubMention),
            Arc::new(StubBranchBase),
            Arc::new(StubCodexFuzzyFileSearch),
        );
        CodeUsecase::new(
            Arc::new(RecordingStaging {
                calls: Mutex::new(Vec::new()),
            }),
            query,
            Arc::new(StubBlobUrls),
        )
    }

    /// mention 解決のフォールバック挙動テスト用に、指定の `MentionRepository` 実装で
    /// usecase を組み立てる。adaptor 層（composition root）へ逆依存せず、stub のみで
    /// usecase を直接構築する（依存方向は usecase → domain trait のみ）。
    fn usecase_with_mention(mention: Arc<dyn MentionRepository>) -> CodeUsecase {
        let query = CodeQueryService::new(
            Arc::new(StubFileContent),
            Arc::new(StubDiffComputer),
            Arc::new(StubBranchDiff),
            mention,
            Arc::new(StubBranchBase),
            Arc::new(StubCodexFuzzyFileSearch),
        );
        CodeUsecase::new(
            Arc::new(RecordingStaging {
                calls: Mutex::new(Vec::new()),
            }),
            query,
            Arc::new(StubBlobUrls),
        )
    }

    #[test]
    fn test_stageはstagingリポジトリへ委譲する() {
        let staging = Arc::new(RecordingStaging {
            calls: Mutex::new(Vec::new()),
        });
        let uc = usecase(staging.clone());

        uc.git_stage("/repo", vec!["a.rs".to_string()]).unwrap();
        uc.git_unstage_hunk("/repo", "patch").unwrap();

        let calls = staging.calls.lock().unwrap();
        assert_eq!(calls[0], "stage:/repo:a.rs");
        assert_eq!(calls[1], "unstage_hunk:/repo");
    }

    #[test]
    fn test_読み取りはquery_serviceへ委譲する() {
        let staging = Arc::new(RecordingStaging {
            calls: Mutex::new(Vec::new()),
        });
        let uc = usecase(staging);
        // QueryService 経由でファイル内容参照が返ることを確認（委譲経路の担保）。
        assert_eq!(uc.get_file_at_ref("f.rs", "HEAD").unwrap(), "c");
        assert_eq!(uc.get_language_from_path("a.rs"), "rust");
    }

    #[test]
    fn review_side_source_mapping_uses_production_metadata_and_bytes_methods() {
        let file_content = Arc::new(RecordingFileContent::default());
        let uc = usecase_with_file_content(file_content.clone());
        let file_path = "/repo/file.txt";
        let cases = [
            (
                ReviewSection::Changes,
                ReviewBase::Head,
                ReviewBlobSide::Original,
                ReviewContentSource::Staged,
                format!("metadata:staged:{file_path}"),
                format!("bytes:staged:{file_path}"),
            ),
            (
                ReviewSection::Changes,
                ReviewBase::Head,
                ReviewBlobSide::Modified,
                ReviewContentSource::WorkingTree,
                format!("metadata:working-tree:{file_path}"),
                format!("bytes:working-tree:{file_path}"),
            ),
            (
                ReviewSection::Staged,
                ReviewBase::Head,
                ReviewBlobSide::Original,
                ReviewContentSource::Head,
                format!("metadata:ref:HEAD:{file_path}"),
                format!("bytes:ref:HEAD:{file_path}"),
            ),
            (
                ReviewSection::Staged,
                ReviewBase::Head,
                ReviewBlobSide::Modified,
                ReviewContentSource::Staged,
                format!("metadata:staged:{file_path}"),
                format!("bytes:staged:{file_path}"),
            ),
            (
                ReviewSection::Changes,
                ReviewBase::BranchBase,
                ReviewBlobSide::Original,
                ReviewContentSource::BranchBase,
                format!("metadata:branch-base:none:{file_path}"),
                format!("bytes:branch-base:none:{file_path}"),
            ),
            (
                ReviewSection::Changes,
                ReviewBase::BranchBase,
                ReviewBlobSide::Modified,
                ReviewContentSource::WorkingTree,
                format!("metadata:working-tree:{file_path}"),
                format!("bytes:working-tree:{file_path}"),
            ),
        ];

        let mut expected_calls = Vec::new();
        for (section, base, side, expected_source, metadata_call, bytes_call) in cases {
            let selected = uc
                .select_review_side_source(file_path, side, section, base)
                .unwrap();
            assert_eq!(selected.source, expected_source);
            uc.read_review_source_bytes(file_path, selected.source)
                .unwrap();
            expected_calls.push(metadata_call);
            expected_calls.push(bytes_call);
        }

        assert_eq!(file_content.calls(), expected_calls);
    }

    #[tokio::test]
    async fn read_codex_mentionable_files_delegates_to_fuzzy_gateway() {
        let staging = Arc::new(RecordingStaging {
            calls: Mutex::new(Vec::new()),
        });
        let uc = usecase(staging);

        let files = uc
            .read_codex_mentionable_files("/repo", "main", 50)
            .await
            .unwrap();

        assert_eq!(files, vec!["src/main.rs"]);
    }

    // ── mention 参照解決のフォールバック挙動（usecase 層の責務）──
    // usecase 層テストは adaptor（composition root）へ逆依存せず stub のみで構成する。
    // 実際のファイル読み込み・抜粋・不在スキップ等の解決ロジックは mention gateway の
    // テスト（`adaptor/gateway/code/mention.rs` の `mention_gateway_tests`）が担保する。

    #[test]
    fn test_mention解決_空メンションは内容不変() {
        let uc = usecase_with_mention(Arc::new(StubMention));
        assert_eq!(
            uc.resolve_mentions_or_fallback("/tmp", "Hello world", &[]),
            "Hello world"
        );
    }

    #[test]
    fn test_mention解決_解決成功時は前置結果をそのまま返す() {
        let uc = usecase_with_mention(Arc::new(ResolvingMention));
        let mentions = vec![MentionReference {
            file_path: "test.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result = uc.resolve_mentions_or_fallback("/wt", "Check", &mentions);
        assert!(result.contains("<file_context>"));
        assert!(result.contains("Check"));
    }

    #[test]
    fn test_mention解決_解決失敗時は本文へフォールバックする() {
        let uc = usecase_with_mention(Arc::new(FailingMention));
        let mentions = vec![MentionReference {
            file_path: "nonexistent.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result = uc.resolve_mentions_or_fallback("/wt", "Check", &mentions);
        assert_eq!(result, "Check");
    }
}
