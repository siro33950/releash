use std::path::PathBuf;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::{WorkflowDiagnosticsGateway, WorkflowDiagnosticsTarget};

use super::diagnostics;

#[derive(Debug, Clone)]
pub struct WorkflowDiagnosticsFileGateway {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDiagnosticsFileGateway {
    pub fn new(workflows_dir: impl Into<PathBuf>, facets_base_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }
}

impl WorkflowDiagnosticsGateway for WorkflowDiagnosticsFileGateway {
    fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<crate::usecase::workflow::diagnostic_dto::DiagnosticReport, WorkflowError> {
        let report = match target {
            WorkflowDiagnosticsTarget::AppliedConfigDirectory => {
                diagnostics::diagnose_all(&self.workflows_dir, &self.facets_base_dir)
            }
            WorkflowDiagnosticsTarget::Directory(dir) => {
                if let Err(error) = std::fs::read_dir(&dir) {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        return Err(WorkflowError::NotFound(format!(
                            "directory does not exist: {}",
                            dir.display()
                        )));
                    }
                    return Err(WorkflowError::external(format!(
                        "failed to read diagnostics target directory '{}': {error}",
                        dir.display()
                    )));
                }
                diagnostics::diagnose_directory(&dir)
            }
        };
        report.map_err(|error| WorkflowError::external(error.to_string()))
    }
}
