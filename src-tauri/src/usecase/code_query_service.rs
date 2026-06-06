//! code ドメインの読み取りクエリサービス（read model 生成・純粋算出の orchestration）。
//!
//! **QueryService は Usecase ではない。** ファイル内容参照・diff 閲覧・hunk/patch/range
//! 算出・diff_tree・language・mention 候補列挙といった読み取り／純粋計算をまとめる協力者で
//! あり、業務手順（オーケストレーション）は持たない。diff バッファ計算（git2 依存）は
//! `DiffComputer` gateway に委譲し、hunk 区切り・range 算出・patch 生成・tree 構築・language
//! 判定はドメインサービス（純粋関数）に委譲する。

use std::sync::Arc;

use crate::domain::code::services;
use crate::domain::code::{
    BranchBaseResolver, ChangeGroup, CodeError, DiffComputer, DiffFileEntry, DiffTreeNode,
    FileContentRepository, HiddenRange, Hunk, MentionReference, MentionRepository, VisibleBlock,
};

use super::code_dto::{
    BranchDiffSummaryDto, ChangeGroupDto, DiffHunksResultDto, DiffTreeNodeDto,
    FileNavigationResultDto, HiddenRangeDto, HunkDto, VisibleBlockDto,
};
use super::code_error::CodeUsecaseError;

/// branch diff サマリの読み取りポート（Query 側）。
///
/// gateway 実装がデータソース（git2）から read model（`BranchDiffSummaryDto`）を
/// 直接組み立てる。
pub trait BranchDiffQuery: Send + Sync {
    /// `base_name` は表示用の base 名タグ（`None` は detached / 未設定で "HEAD" 表示）。
    /// `base_commit_oid` は usecase が解決済みの base コミット OID(hex)（`None` は HEAD
    /// フォールバック）。ref 解決は repository ドメインが所有するため本 port は受け取らない。
    fn summary(
        &self,
        repo_path: &str,
        base_name: Option<&str>,
        base_commit_oid: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, crate::domain::code::CodeError>;
}

/// read model 構築・純粋算出を担う読み取りクエリサービス。
#[derive(Clone)]
pub struct CodeQueryService {
    file_content: Arc<dyn FileContentRepository>,
    diff_computer: Arc<dyn DiffComputer>,
    branch_diff: Arc<dyn BranchDiffQuery>,
    mention: Arc<dyn MentionRepository>,
    branch_base: Arc<dyn BranchBaseResolver>,
}

impl CodeQueryService {
    pub fn new(
        file_content: Arc<dyn FileContentRepository>,
        diff_computer: Arc<dyn DiffComputer>,
        branch_diff: Arc<dyn BranchDiffQuery>,
        mention: Arc<dyn MentionRepository>,
        branch_base: Arc<dyn BranchBaseResolver>,
    ) -> Self {
        Self {
            file_content,
            diff_computer,
            branch_diff,
            mention,
            branch_base,
        }
    }

    // ── ファイル内容参照 ──

    pub fn get_file_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<String, CodeUsecaseError> {
        Ok(self.file_content.file_at_ref(file_path, git_ref)?)
    }

    pub fn get_binary_file_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<String, CodeUsecaseError> {
        Ok(self.file_content.binary_file_at_ref(file_path, git_ref)?)
    }

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

    pub fn get_file_at_branch_base(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        let base_oid = self.resolve_base_commit_oid_for(file_path)?;
        Ok(self
            .file_content
            .file_at_branch_base(file_path, base_oid.as_deref())?)
    }

    pub fn get_binary_file_at_branch_base(
        &self,
        file_path: &str,
    ) -> Result<String, CodeUsecaseError> {
        let base_oid = self.resolve_base_commit_oid_for(file_path)?;
        Ok(self
            .file_content
            .binary_file_at_branch_base(file_path, base_oid.as_deref())?)
    }

    /// 現在ブランチの実効 base 名（ref 実在検証あり）。agent bridge の env 伝搬向け。
    pub fn resolve_effective_base_branch_name(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, CodeUsecaseError> {
        Ok(self
            .branch_base
            .resolve_effective_base_branch_name(path_hint)?)
    }

    pub fn get_staged_content(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        Ok(self.file_content.staged_content(file_path)?)
    }

    pub fn get_binary_staged_content(&self, file_path: &str) -> Result<String, CodeUsecaseError> {
        Ok(self.file_content.binary_staged_content(file_path)?)
    }

    // ── branch diff ──

    pub fn get_branch_diff_summary(
        &self,
        repo_path: &str,
        base_branch: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeUsecaseError> {
        // Thread 6 / 移行前等価: 明示 base 名が無い通常経路（`useBranchDiffFiles` 等）でも、
        // 旧 `find_base_commit` が内部で行っていた現在ブランチの base 解決
        // （per-branch override → global → default）を usecase で補完してから渡す。
        // 補完できない（detached / 未設定）場合のみ `None` のまま HEAD フォールバックする。
        let base_name = match base_branch {
            Some(name) => Some(name.to_string()),
            None => self.branch_base.resolve_base_branch_name(repo_path)?,
        };
        // ref → base コミット OID の解決は repository ドメインへ委譲。base 名はあるが ref が
        // 実在しない場合は移行前と等価に "base branch not found" を返す。
        let base_oid = match &base_name {
            Some(name) => match self.branch_base.resolve_base_commit_oid(repo_path, name)? {
                Some(oid) => Some(oid),
                None => {
                    return Err(CodeError::Rule(format!("base branch '{name}' not found")).into())
                }
            },
            None => None,
        };
        Ok(self
            .branch_diff
            .summary(repo_path, base_name.as_deref(), base_oid.as_deref())?)
    }

    // ── diff tree（純粋） ──

    pub fn build_diff_file_tree(&self, entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNodeDto> {
        services::diff_tree::build_tree(entries)
            .iter()
            .map(diff_tree_node_to_dto)
            .collect()
    }

    pub fn get_file_navigation(
        &self,
        tree: &[DiffTreeNode],
        current_file: &str,
    ) -> FileNavigationResultDto {
        let nav = services::diff_tree::get_file_navigation(tree, current_file);
        FileNavigationResultDto {
            current_index: nav.current_index,
            total: nav.total,
            prev_file: nav.prev_file,
            next_file: nav.next_file,
        }
    }

    // ── hunk / patch / range ──

    pub fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> DiffHunksResultDto {
        let hunks = self
            .diff_computer
            .diff_buffers(original, modified, file_path);
        let change_groups = services::hunk::compute_change_groups(&hunks);
        DiffHunksResultDto {
            hunks: hunks.iter().map(hunk_to_dto).collect(),
            change_groups: change_groups.iter().map(change_group_to_dto).collect(),
        }
    }

    pub fn generate_group_patch(
        &self,
        file_path: &str,
        hunk: &Hunk,
        group: &ChangeGroup,
    ) -> String {
        services::hunk::generate_group_patch(file_path, hunk, group)
    }

    pub fn compute_hidden_ranges(
        &self,
        hunks: &[Hunk],
        total_lines: u32,
        context_lines: u32,
    ) -> Vec<HiddenRangeDto> {
        services::hunk::compute_hidden_ranges(hunks, total_lines, context_lines)
            .iter()
            .map(hidden_range_to_dto)
            .collect()
    }

    pub fn compute_hidden_ranges_from_content(
        &self,
        original: &str,
        modified: &str,
        context_lines: u32,
    ) -> Vec<HiddenRangeDto> {
        let hunks = self.diff_computer.diff_buffers(original, modified, None);
        let total_lines = modified.lines().count() as u32;
        services::hunk::compute_hidden_ranges(&hunks, total_lines, context_lines)
            .iter()
            .map(hidden_range_to_dto)
            .collect()
    }

    pub fn compute_visible_markdown_blocks(
        &self,
        original: &str,
        modified: &str,
        context_lines: u32,
    ) -> Vec<VisibleBlockDto> {
        let hunks = self.diff_computer.diff_buffers(original, modified, None);
        services::hunk::compute_visible_markdown_blocks(&hunks, original, modified, context_lines)
            .iter()
            .map(visible_block_to_dto)
            .collect()
    }

    // ── language（純粋） ──

    pub fn get_language_from_path(&self, file_path: &str) -> String {
        services::language::get_language_from_path(file_path)
    }

    pub fn get_relative_path(&self, root_path: &str, file_path: &str) -> Option<String> {
        crate::other::utils::relative_path(root_path, file_path)
    }

    // ── file mention（候補列挙・参照解決） ──

    pub fn list_mentionable_files(
        &self,
        worktree_path: &str,
        query: &str,
    ) -> Result<Vec<String>, CodeUsecaseError> {
        Ok(self.mention.list_mentionable_files(worktree_path, query)?)
    }

    /// 構造化メンション参照を解決し file_context を前置する。失敗時のフォールバック方針は
    /// 呼び出し元（`CodeUsecase`）が決めるため、ここでは解決結果／エラーをそのまま返す。
    pub fn resolve_mentions(
        &self,
        worktree_path: &str,
        content: &str,
        mentions: &[MentionReference],
    ) -> Result<String, CodeUsecaseError> {
        Ok(self
            .mention
            .resolve_mentions(worktree_path, content, mentions)?)
    }
}

// ── VO → DTO 変換（QueryService が算出結果を転送表現へ詰め替える） ──

fn hunk_to_dto(h: &Hunk) -> HunkDto {
    HunkDto {
        index: h.index,
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h.lines.clone(),
    }
}

fn change_group_to_dto(g: &ChangeGroup) -> ChangeGroupDto {
    ChangeGroupDto {
        group_index: g.group_index,
        hunk_index: g.hunk_index,
        new_start: g.new_start,
        new_end: g.new_end,
        line_offset_start: g.line_offset_start,
        line_offset_end: g.line_offset_end,
        is_staged: g.is_staged,
    }
}

fn hidden_range_to_dto(r: &HiddenRange) -> HiddenRangeDto {
    HiddenRangeDto {
        start_line: r.start_line,
        end_line: r.end_line,
        hidden_count: r.hidden_count,
    }
}

fn visible_block_to_dto(b: &VisibleBlock) -> VisibleBlockDto {
    VisibleBlockDto {
        start_line: b.start_line,
        end_line: b.end_line,
        content: b.content.clone(),
        deleted_content: b.deleted_content.clone(),
    }
}

fn diff_tree_node_to_dto(n: &DiffTreeNode) -> DiffTreeNodeDto {
    DiffTreeNodeDto {
        id: n.id.clone(),
        name: n.name.clone(),
        path: n.path.clone(),
        node_type: n.node_type.clone(),
        status: n.status.clone(),
        additions: n.additions,
        deletions: n.deletions,
        children: n.children.iter().map(diff_tree_node_to_dto).collect(),
    }
}

#[cfg(test)]
mod code_query_service_tests {
    use super::*;
    use crate::domain::code::CodeError;

    struct FakeFileContent;
    impl FileContentRepository for FakeFileContent {
        fn file_at_ref(&self, file_path: &str, git_ref: &str) -> Result<String, CodeError> {
            Ok(format!("{file_path}@{git_ref}"))
        }
        fn binary_file_at_ref(
            &self,
            _file_path: &str,
            _git_ref: &str,
        ) -> Result<String, CodeError> {
            Ok("YmluYXJ5".to_string())
        }
        fn file_at_branch_base(
            &self,
            _file_path: &str,
            base_commit_oid: Option<&str>,
        ) -> Result<String, CodeError> {
            Ok(format!("base@{}", base_commit_oid.unwrap_or("HEAD")))
        }
        fn binary_file_at_branch_base(
            &self,
            _file_path: &str,
            _base_branch: Option<&str>,
        ) -> Result<String, CodeError> {
            Ok("YmFzZQ==".to_string())
        }
        fn staged_content(&self, _file_path: &str) -> Result<String, CodeError> {
            Ok("staged".to_string())
        }
        fn binary_staged_content(&self, _file_path: &str) -> Result<String, CodeError> {
            Ok("c3RhZ2Vk".to_string())
        }
    }

    struct FakeDiffComputer;
    impl DiffComputer for FakeDiffComputer {
        fn diff_buffers(
            &self,
            _original: &str,
            _modified: &str,
            _file_path: Option<&str>,
        ) -> Vec<Hunk> {
            vec![Hunk {
                index: 0,
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec!["-a".to_string(), "+b".to_string()],
            }]
        }
    }

    struct FakeBranchDiff;
    impl BranchDiffQuery for FakeBranchDiff {
        fn summary(
            &self,
            _repo_path: &str,
            base_name: Option<&str>,
            _base_commit_oid: Option<&str>,
        ) -> Result<BranchDiffSummaryDto, CodeError> {
            Ok(BranchDiffSummaryDto {
                base_branch: base_name.unwrap_or("main").to_string(),
                changed_files: vec![],
                stats: crate::usecase::code_dto::DiffStatsDto {
                    additions: 0,
                    deletions: 0,
                },
            })
        }
    }

    struct FakeMention;
    impl MentionRepository for FakeMention {
        fn list_mentionable_files(
            &self,
            _worktree_path: &str,
            query: &str,
        ) -> Result<Vec<String>, CodeError> {
            if query.is_empty() {
                Ok(vec!["a.rs".to_string(), "b.rs".to_string()])
            } else {
                Ok(vec!["a.rs".to_string()])
            }
        }
        fn resolve_mentions(
            &self,
            _worktree_path: &str,
            content: &str,
            mentions: &[MentionReference],
        ) -> Result<String, CodeError> {
            if mentions.is_empty() {
                Ok(content.to_string())
            } else {
                Ok(format!("<file_context/>\n{content}"))
            }
        }
    }

    struct FakeBranchBase;
    impl BranchBaseResolver for FakeBranchBase {
        fn resolve_base_branch_name(&self, _path_hint: &str) -> Result<Option<String>, CodeError> {
            Ok(Some("main".to_string()))
        }
        fn resolve_effective_base_branch_name(
            &self,
            _path_hint: &str,
        ) -> Result<Option<String>, CodeError> {
            Ok(Some("main".to_string()))
        }
        fn resolve_base_commit_oid(
            &self,
            _path_hint: &str,
            base_name: &str,
        ) -> Result<Option<String>, CodeError> {
            // Fake では base 名をそのまま OID 代わりに返し、resolver → file_content の
            // 配線（base 名で解決した値が下流へ渡る）を担保する。
            Ok(Some(base_name.to_string()))
        }
    }

    fn service() -> CodeQueryService {
        CodeQueryService::new(
            Arc::new(FakeFileContent),
            Arc::new(FakeDiffComputer),
            Arc::new(FakeBranchDiff),
            Arc::new(FakeMention),
            Arc::new(FakeBranchBase),
        )
    }

    #[test]
    fn test_ファイル内容参照を委譲する() {
        let s = service();
        assert_eq!(s.get_file_at_ref("f.rs", "HEAD").unwrap(), "f.rs@HEAD");
        assert_eq!(s.get_staged_content("f.rs").unwrap(), "staged");
    }

    #[test]
    fn test_branch_base参照はresolverで解決したbase名を渡す() {
        // FakeBranchBase が Some("main") を返し、FakeFileContent が受け取った base 名を
        // 反映する（base@main）。base 名解決を resolver 経由で行う配線を担保する。
        let s = service();
        assert_eq!(s.get_file_at_branch_base("f.rs").unwrap(), "base@main");
    }

    #[test]
    fn test_実効base名を委譲する() {
        let s = service();
        assert_eq!(
            s.resolve_effective_base_branch_name("/repo").unwrap(),
            Some("main".to_string())
        );
    }

    #[test]
    fn test_diff_hunks算出はchange_groupを付与する() {
        let s = service();
        // FakeDiffComputer が 1 hunk（-a/+b）を返す → change group が 1 件算出される。
        let result = s.compute_diff_hunks("a\n", "b\n", None);
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(result.change_groups.len(), 1);
        assert_eq!(result.change_groups[0].hunk_index, 0);
    }

    #[test]
    fn test_branch_diffサマリを委譲する() {
        let s = service();
        let summary = s.get_branch_diff_summary("/repo", Some("develop")).unwrap();
        assert_eq!(summary.base_branch, "develop");
    }

    #[test]
    fn test_branch_diff_base未指定は現在ブランチbaseを補完する() {
        // Thread 6: base_branch=None の通常経路でも resolver が解決した base 名
        // （FakeBranchBase → "main"）を補完して gateway へ渡す。HEAD フォールバックに
        // 倒れないことを担保する。
        let s = service();
        let summary = s.get_branch_diff_summary("/repo", None).unwrap();
        assert_eq!(summary.base_branch, "main");
    }

    #[test]
    fn test_mention候補列挙を委譲する() {
        let s = service();
        assert_eq!(s.list_mentionable_files("/wt", "").unwrap().len(), 2);
        assert_eq!(s.list_mentionable_files("/wt", "a").unwrap().len(), 1);
    }

    #[test]
    fn test_language判定を委譲する() {
        let s = service();
        assert_eq!(s.get_language_from_path("main.rs"), "rust");
    }
}
