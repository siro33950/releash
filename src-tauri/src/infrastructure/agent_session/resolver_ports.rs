use crate::domain::code::MentionReference;
use crate::usecase::code_usecase::CodeUsecase;

pub(crate) trait BaseBranchResolverPort: Send + Sync {
    fn resolve_effective_base_branch_name(&self, cwd: &str) -> Option<String>;
}

pub(crate) trait MentionResolverPort: Send + Sync {
    fn resolve_mentions_or_fallback(
        &self,
        worktree_path: &str,
        content: &str,
        mentions: &[MentionReference],
    ) -> String;
}

impl BaseBranchResolverPort for CodeUsecase {
    fn resolve_effective_base_branch_name(&self, cwd: &str) -> Option<String> {
        self.resolve_effective_base_branch_name(cwd).ok().flatten()
    }
}

impl MentionResolverPort for CodeUsecase {
    fn resolve_mentions_or_fallback(
        &self,
        worktree_path: &str,
        content: &str,
        mentions: &[MentionReference],
    ) -> String {
        self.resolve_mentions_or_fallback(worktree_path, content, mentions)
    }
}
