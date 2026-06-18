use std::path::PathBuf;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowConfigPathGateway;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowConfigPathFileGateway {
    workflows_dir: PathBuf,
}

impl WorkflowConfigPathFileGateway {
    pub(crate) fn new(workflows_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
        }
    }
}

impl WorkflowConfigPathGateway for WorkflowConfigPathFileGateway {
    fn automation_config_dir(&self) -> Result<String, WorkflowError> {
        Ok(self.workflows_dir.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn returns_workflows_dir_as_automation_config_dir() {
        let tmp = TempDir::new().unwrap();

        assert_eq!(
            WorkflowConfigPathFileGateway::new(tmp.path())
                .automation_config_dir()
                .unwrap(),
            tmp.path().to_string_lossy()
        );
    }
}
