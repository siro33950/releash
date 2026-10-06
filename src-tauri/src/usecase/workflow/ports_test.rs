pub(crate) mod tests {
    use super::super::*;
    use std::path::PathBuf;

    #[test]
    fn test_診断対象_directory省略時は適用済みconfigになる() {
        // Given
        let directory = None;

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(directory).unwrap();

        // Then
        assert_eq!(target, WorkflowDiagnosticsTarget::AppliedConfigDirectory);
    }

    #[test]
    fn test_診断対象_空directoryを拒否する() {
        // Given
        let directories = [String::new(), "   ".to_string()];

        // When
        let targets = directories
            .map(|directory| WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)));

        // Then
        assert!(targets
            .into_iter()
            .all(|target| matches!(target, Err(WorkflowError::Validation(_)))));
    }

    #[test]
    fn test_診断対象_相対directoryを拒否する() {
        // Given
        let directory = "workflows".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory));

        // Then
        assert!(matches!(target, Err(WorkflowError::Validation(_))));
    }

    #[test]
    fn test_診断対象_先頭空白付き相対directoryを拒否する() {
        // Given
        let directory = " workflows".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory));

        // Then
        assert!(matches!(target, Err(WorkflowError::Validation(_))));
    }

    #[test]
    fn test_診断対象_絶対directoryを受理する() {
        // Given
        let directory = "/tmp/x".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)).unwrap();

        // Then
        assert_eq!(
            target,
            WorkflowDiagnosticsTarget::Directory(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn test_診断対象_絶対directoryの末尾空白を保持する() {
        // Given
        let directory = "/tmp/workflows ".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)).unwrap();

        // Then
        assert_eq!(
            target,
            WorkflowDiagnosticsTarget::Directory(PathBuf::from("/tmp/workflows "))
        );
    }
}
