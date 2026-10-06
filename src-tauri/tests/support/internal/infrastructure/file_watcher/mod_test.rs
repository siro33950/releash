pub(crate) mod tests {
    use super::super::*;

    #[test]
    pub fn stop_watching_removes_session_and_errors_on_unknown_id() {
        let manager = FileWatcherManager::default();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let id = manager.start_watching(1, path, |_| {}).unwrap();
        assert_eq!(id, 1);

        manager.stop_watching(1).unwrap();
        assert!(manager.stop_watching(1).is_err());
    }
}
