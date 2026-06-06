//! code ドメインの Command 側ユースケース（差分 Approve = staging）と、読み取りの集約入口。
//!
//! controller はこの Usecase だけを入口とする。書き込み（staging）に加え、ファイル内容
//! 参照・diff 閲覧・hunk/patch/range 算出・diff_tree・language・mention 候補列挙といった
//! 読み取り／純粋計算も、読み取りクエリサービス（協力者）へ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::code::{
    ChangeGroup, DiffFileEntry, DiffTreeNode, Hunk, MentionReference, StagingRepository,
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

    pub fn get_file_in_worktree(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        self.query.get_file_in_worktree(file_path)
    }

    // ── branch diff ──

    pub fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
        self.query.get_branch_diff_summary(repo_path, base_branch)
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
    use crate::usecase::code_query_service::BranchDiffQuery;
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
        fn file_in_worktree(&self, _f: &str) -> Result<String, CodeError> {
            Ok("c".to_string())
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

    fn usecase(staging: Arc<RecordingStaging>) -> CodeUsecase {
        let query = CodeQueryService::new(
            Arc::new(StubFileContent),
            Arc::new(StubDiffComputer),
            Arc::new(StubBranchDiff),
            Arc::new(StubMention),
            Arc::new(StubBranchBase),
        );
        CodeUsecase::new(staging, query)
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
        );
        CodeUsecase::new(
            Arc::new(RecordingStaging {
                calls: Mutex::new(Vec::new()),
            }),
            query,
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
