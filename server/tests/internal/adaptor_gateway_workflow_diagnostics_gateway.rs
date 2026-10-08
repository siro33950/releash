pub(crate) mod tests {

    use releashd::test_support::integration::workflow::WorkflowDiagnosticsFileGateway;
    use releashd::test_support::integration::workflow::WorkflowDiagnosticsGateway;
    use releashd::test_support::integration::workflow::WorkflowDiagnosticsTarget;
    use releashd::test_support::integration::workflow::WorkflowError;
    use tempfile::TempDir;

    #[test]
    pub fn test_診断gateway_指定directoryを使う() {
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
        assert!(report
            .items
            .iter()
            .any(|item| item.code == "WFS001" && item.workflow_name.as_deref() == Some("broken")));
    }

    #[test]
    pub fn test_診断gateway_適用済みreportを保持する() {
        // Given
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        let expected = releashd::test_support::integration::workflow::diagnose_all(
            workflows.path(),
            facets.path(),
        )
        .unwrap();

        // When
        let actual = WorkflowDiagnosticsFileGateway::new(workflows.path(), facets.path())
            .diagnose_all(WorkflowDiagnosticsTarget::AppliedConfigDirectory)
            .unwrap();

        // Then
        assert_eq!(actual, expected);
    }

    #[test]
    pub fn test_診断gateway_存在しない指定directoryをnot_foundにする() {
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
    pub fn test_診断gateway_通常fileの指定をexternal_errorにする() {
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
    pub fn test_診断gateway_列挙不能な指定directoryをexternal_errorにする() {
        use std::os::unix::fs::PermissionsExt;

        // Given
        let configured = TempDir::new().unwrap();
        let unreadable = configured.path().join("unreadable");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&unreadable).is_ok() {
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!(
                "skipping unreadable directory test because this process can read mode 0o000"
            );
            return;
        }

        // When
        let result = WorkflowDiagnosticsFileGateway::new(configured.path(), configured.path())
            .diagnose_all(WorkflowDiagnosticsTarget::Directory(unreadable.clone()));
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();

        // Then
        assert!(matches!(error, WorkflowError::External(_)));
    }
}
