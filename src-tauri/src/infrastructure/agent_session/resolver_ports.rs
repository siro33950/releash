use crate::domain::code::MentionReference;
use crate::usecase::agent_session::context::{
    file_system_instruction_cache_key, BranchDiffContextChangedFile, BranchDiffContextPort,
    BranchDiffContextStats, BranchDiffContextSummary, InstructionSourcePort,
};
use crate::usecase::code_usecase::CodeUsecase;
use std::path::Path;

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

impl BranchDiffContextPort for CodeUsecase {
    fn get_branch_diff_context(
        &self,
        worktree_path: &str,
    ) -> Result<BranchDiffContextSummary, String> {
        let summary = self
            .get_branch_diff_summary(worktree_path, None)
            .map_err(|err| err.to_string())?;
        Ok(BranchDiffContextSummary {
            base_branch: summary.base_branch,
            changed_files: summary
                .changed_files
                .into_iter()
                .map(|file| BranchDiffContextChangedFile {
                    path: file.path,
                    status: file.status,
                    stats: BranchDiffContextStats {
                        additions: file.stats.additions,
                        deletions: file.stats.deletions,
                    },
                })
                .collect(),
        })
    }
}

pub(crate) struct FileSystemInstructionSourcePort;

impl InstructionSourcePort for FileSystemInstructionSourcePort {
    fn read_instruction_file(
        &self,
        path: &Path,
        worktree_root: &Path,
    ) -> Result<Option<String>, String> {
        let canonical_root = match worktree_root.canonicalize() {
            Ok(root) => root,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(format!(
                    "failed to validate instruction root {}: {err}",
                    worktree_root.display()
                ));
            }
        };
        let canonical_path = match path.canonicalize() {
            Ok(path) => path,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(format!(
                    "failed to validate instruction file {}: {err}",
                    path.display()
                ));
            }
        };
        if canonical_path != canonical_root && !canonical_path.starts_with(&canonical_root) {
            return Ok(None);
        }
        std::fs::read_to_string(&canonical_path)
            .map(Some)
            .map_err(|e| format!("failed to read {}: {e}", canonical_path.display()))
    }

    fn instruction_cache_key(&self, _worktree_root: &Path) -> Option<String> {
        Some(file_system_instruction_cache_key())
    }
}

#[cfg(test)]
mod tests {
    use super::FileSystemInstructionSourcePort;
    use crate::usecase::agent_session::context::{
        invalidate_instruction_resolution_cache_for_path, InstructionResolutionRequest,
        InstructionResolver,
    };

    #[cfg(unix)]
    #[test]
    fn root_external_symlink_instruction_is_not_read() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(temp.path().join("secret.txt"), "secret instruction").unwrap();
        std::os::unix::fs::symlink("../secret.txt", repo.join("AGENTS.md")).unwrap();
        let source = FileSystemInstructionSourcePort;
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo,
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        });

        assert_eq!(result.payload(), None);
    }

    #[cfg(unix)]
    #[test]
    fn root_internal_symlink_instruction_is_read() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("inside.txt"), "inside instruction").unwrap();
        std::os::unix::fs::symlink("inside.txt", repo.join("AGENTS.md")).unwrap();
        let source = FileSystemInstructionSourcePort;
        let resolver = InstructionResolver::new(&source);

        let result = resolver.resolve(&InstructionResolutionRequest {
            worktree_root: repo,
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        });

        assert_eq!(result.payload().as_deref(), Some("inside instruction"));
    }

    #[test]
    fn file_system_instruction_cache_invalidates_for_instruction_file_change() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let instruction_path = repo.join("AGENTS.md");
        std::fs::write(&instruction_path, "root-v1").unwrap();
        let source = FileSystemInstructionSourcePort;
        let resolver = InstructionResolver::new(&source);
        let request = InstructionResolutionRequest {
            worktree_root: repo,
            repo_context_dir: None,
            read_file_paths: Vec::new(),
            workflow_instructions: Vec::new(),
        };

        let first = resolver.resolve(&request);
        std::fs::write(&instruction_path, "root-v2").unwrap();
        invalidate_instruction_resolution_cache_for_path(&instruction_path);
        let second = resolver.resolve(&request);

        assert_eq!(first.payload().as_deref(), Some("root-v1"));
        assert_eq!(second.payload().as_deref(), Some("root-v2"));
    }
}
