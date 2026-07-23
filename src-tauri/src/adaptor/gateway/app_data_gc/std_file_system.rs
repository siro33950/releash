use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::adaptor::gateway::workflow::WorkflowExecutionArchiveFileRepository;
use crate::usecase::app_data_gc::{
    gc_file_system_error, GcFileSystem, GcFileSystemError, WorkflowArchivePruneResult,
    WorkflowArchivePruner,
};

pub(crate) struct StdGcFileSystem;

impl GcFileSystem for StdGcFileSystem {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    #[cfg(test)]
    fn is_file(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
        std::fs::read_dir(path)
            .map_err(gc_file_system_error)?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(gc_file_system_error)
            })
            .collect()
    }

    #[cfg(test)]
    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError> {
        std::fs::read_to_string(path).map_err(gc_file_system_error)
    }

    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(gc_file_system_error(error)),
        };
        if metadata.file_type().is_dir() {
            std::fs::remove_dir_all(path).map_err(gc_file_system_error)?;
        } else {
            std::fs::remove_file(path).map_err(gc_file_system_error)?;
        }
        Ok(true)
    }

    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError> {
        let metadata = std::fs::symlink_metadata(path).map_err(gc_file_system_error)?;
        if metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Ok(metadata.len());
        }
        if !metadata.file_type().is_dir() {
            return Ok(0);
        }
        let mut size = 0;
        for entry in self.read_dir(path)? {
            size += self.recursive_size(&entry)?;
        }
        Ok(size)
    }
}

pub(crate) struct StdWorkflowArchivePruner;

impl WorkflowArchivePruner for StdWorkflowArchivePruner {
    fn prune_workflow_archive_records(
        &self,
        app_data_dir: &Path,
        execution_ids: &HashSet<String>,
    ) -> Result<WorkflowArchivePruneResult, GcFileSystemError> {
        let result = WorkflowExecutionArchiveFileRepository::new(app_data_dir)
            .prune_records(execution_ids)
            .map_err(|error| GcFileSystemError::other(error.to_string()))?;
        Ok(WorkflowArchivePruneResult {
            records_removed: result.records_removed,
            reclaimed_bytes: result.reclaimed_bytes,
        })
    }
}
