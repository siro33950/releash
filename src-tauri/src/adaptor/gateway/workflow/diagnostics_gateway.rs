use std::path::PathBuf;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::{WorkflowDiagnosticsGateway, WorkflowDiagnosticsTarget};

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
    fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<serde_json::Value, WorkflowError> {
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
            .diagnose_all(WorkflowDiagnosticsTarget::AppliedConfigDirectory)
            .unwrap();

        assert!(report["items"].is_array());
        assert!(report["workflow_summaries"].is_object());
        assert!(report["facet_summaries"].is_object());
        assert!(report["facet_usage"].is_object());
    }

    #[test]
    fn test_診断gateway_指定directoryを使う() {
        // Given
        let configured = TempDir::new().unwrap();
        let requested = TempDir::new().unwrap();
        std::fs::write(requested.path().join("broken.yml"), "name: [").unwrap();

        // When
        let report = WorkflowDiagnosticsFileGateway::new(configured.path(), configured.path())
            .diagnose_all(WorkflowDiagnosticsTarget::Directory(
                requested.path().to_path_buf(),
            ))
            .unwrap();

        // Then
        assert!(report["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["code"] == "WFS001" && item["workflow_name"] == "broken" }));
    }

    #[test]
    fn test_診断gateway_適用済みreportを保持する() {
        // Given
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        let expected =
            serde_json::to_value(diagnostics::diagnose_all(workflows.path(), facets.path()))
                .unwrap();

        // When
        let actual = WorkflowDiagnosticsFileGateway::new(workflows.path(), facets.path())
            .diagnose_all(WorkflowDiagnosticsTarget::AppliedConfigDirectory)
            .unwrap();

        // Then
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_診断gateway_存在しない指定directoryをnot_foundにする() {
        // Given
        let configured = TempDir::new().unwrap();
        let missing = configured.path().join("missing");

        // When
        let error = WorkflowDiagnosticsFileGateway::new(configured.path(), configured.path())
            .diagnose_all(WorkflowDiagnosticsTarget::Directory(missing.clone()))
            .unwrap_err();

        // Then
        assert!(matches!(
            error,
            WorkflowError::NotFound(message)
                if message == format!("directory does not exist: {}", missing.display())
        ));
    }

    #[test]
    fn test_診断gateway_通常fileの指定をexternal_errorにする() {
        // Given
        let configured = TempDir::new().unwrap();
        let file = configured.path().join("workflow.yml");
        std::fs::write(&file, "name: workflow").unwrap();

        // When
        let error = WorkflowDiagnosticsFileGateway::new(configured.path(), configured.path())
            .diagnose_all(WorkflowDiagnosticsTarget::Directory(file))
            .unwrap_err();

        // Then
        assert!(matches!(error, WorkflowError::External(_)));
    }

    #[cfg(unix)]
    #[test]
    fn test_診断gateway_列挙不能な指定directoryをexternal_errorにする() {
        use std::os::unix::fs::PermissionsExt;

        // Given
        let configured = TempDir::new().unwrap();
        let unreadable = configured.path().join("unreadable");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();

        // When
        let result = WorkflowDiagnosticsFileGateway::new(configured.path(), configured.path())
            .diagnose_all(WorkflowDiagnosticsTarget::Directory(unreadable.clone()));
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();

        // Then
        assert!(matches!(error, WorkflowError::External(_)));
    }
}
