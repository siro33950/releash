use std::path::PathBuf;

use crate::domain::workflow::{WorkflowDirectory, WorkflowDirectoryService, WorkflowError};

#[derive(Debug, Clone)]
pub struct WorkflowDirectoryFileService {
    workflows_dir: PathBuf,
}

impl WorkflowDirectoryFileService {
    pub fn new(workflows_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
        }
    }
}

impl WorkflowDirectoryService for WorkflowDirectoryFileService {
    fn prepare(&self) -> Result<WorkflowDirectory, WorkflowError> {
        super::storage::ensure_dir(&self.workflows_dir)
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        Ok(WorkflowDirectory::new(self.workflows_dir.clone()))
    }
}
