use std::path::PathBuf;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowDiagnosticsGateway;

use super::diagnostics;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowDiagnosticsFileGateway {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDiagnosticsFileGateway {
    pub(crate) fn new(
        workflows_dir: impl Into<PathBuf>,
        facets_base_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }
}

impl WorkflowDiagnosticsGateway for WorkflowDiagnosticsFileGateway {
    fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError> {
        let report = diagnostics::diagnose_all(&self.workflows_dir, &self.facets_base_dir);
        serde_json::to_value(report)
            .map_err(|e| WorkflowError::external(format!("serialize workflow diagnostics: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn returns_existing_diagnostic_report_wire_shape() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();

        let report = WorkflowDiagnosticsFileGateway::new(workflows.path(), facets.path())
            .diagnose_all()
            .unwrap();

        assert!(report["items"].is_array());
        assert!(report["workflow_summaries"].is_object());
        assert!(report["facet_summaries"].is_object());
        assert!(report["facet_usage"].is_object());
    }
}
