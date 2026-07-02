use std::path::Path;

use crate::usecase::agent_session::context::{
    file_system_instruction_cache_key, InstructionSourcePort,
};

pub(crate) struct FileSystemInstructionSourceGateway;

impl InstructionSourcePort for FileSystemInstructionSourceGateway {
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
    use super::*;

    #[test]
    fn read_instruction_file_reads_root_inner_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        std::fs::write(&path, "instructions").unwrap();

        let content = FileSystemInstructionSourceGateway
            .read_instruction_file(&path, tmp.path())
            .unwrap();

        assert_eq!(content.as_deref(), Some("instructions"));
    }

    #[test]
    fn read_instruction_file_returns_none_for_missing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("missing.md");

        let content = FileSystemInstructionSourceGateway
            .read_instruction_file(&path, tmp.path())
            .unwrap();

        assert!(content.is_none());
    }

    #[test]
    fn read_instruction_file_rejects_root_outside_path() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("AGENTS.md");
        std::fs::write(&path, "outside").unwrap();

        let content = FileSystemInstructionSourceGateway
            .read_instruction_file(&path, root.path())
            .unwrap();

        assert!(content.is_none());
    }

    #[test]
    fn read_instruction_file_allows_path_equal_to_root() {
        let root_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(root_file.path(), "root file").unwrap();

        let content = FileSystemInstructionSourceGateway
            .read_instruction_file(root_file.path(), root_file.path())
            .unwrap();

        assert_eq!(content.as_deref(), Some("root file"));
    }

    #[test]
    fn read_instruction_file_returns_error_when_read_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_path = tmp.path().join("directory.md");
        std::fs::create_dir(&dir_path).unwrap();

        let error = FileSystemInstructionSourceGateway
            .read_instruction_file(&dir_path, tmp.path())
            .unwrap_err();

        assert!(error.contains("failed to read"));
    }

    #[cfg(unix)]
    #[test]
    fn read_instruction_file_rejects_symlink_that_escapes_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "outside").unwrap();
        let link = root.path().join("linked.md");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();

        let content = FileSystemInstructionSourceGateway
            .read_instruction_file(&link, root.path())
            .unwrap();

        assert!(content.is_none());
    }
}
