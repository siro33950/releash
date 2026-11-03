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
        let directory = WorkflowDirectory::new(self.workflows_dir.clone());
        super::storage::ensure_dir(directory.path())
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        Ok(directory)
    }
}
