use git2::{Repository, Status, StatusOptions};

use crate::usecase::agent_session::session::{AgentPromptGitStatusGateway, GitSuggestionContext};

pub(crate) struct GitAgentPromptSuggestionGateway;

impl AgentPromptGitStatusGateway for GitAgentPromptSuggestionGateway {
    fn suggestion_context(&self, worktree_path: &str) -> Option<GitSuggestionContext> {
        let repo = Repository::discover(worktree_path).ok()?;
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);
        let statuses = repo.statuses(Some(&mut options)).ok()?;
        let mut context = GitSuggestionContext::default();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.contains(Status::INDEX_NEW)
                || status.contains(Status::INDEX_MODIFIED)
                || status.contains(Status::INDEX_DELETED)
                || status.contains(Status::INDEX_RENAMED)
                || status.contains(Status::INDEX_TYPECHANGE)
            {
                context.staged_count += 1;
            }
            if status.contains(Status::WT_MODIFIED)
                || status.contains(Status::WT_DELETED)
                || status.contains(Status::WT_RENAMED)
                || status.contains(Status::WT_TYPECHANGE)
            {
                context.unstaged_count += 1;
            }
            if status.contains(Status::WT_NEW) {
                context.untracked_count += 1;
            }
        }
        Some(context)
    }
}
