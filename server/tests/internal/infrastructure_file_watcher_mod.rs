pub(crate) mod tests {

    use releashd::test_support::integration::platform::FileWatcherManager;

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

    #[cfg(unix)]
    #[tokio::test]
    pub async fn test_木監視_読めない子を除いて既存と新しいディレクトリの変更を通知する() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        struct RestorePermissions(std::path::PathBuf, std::fs::Permissions);
        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                std::fs::set_permissions(&self.0, self.1.clone()).unwrap();
            }
        }

        // Given
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("blocked");
        let readable = directory.path().join("readable/nested");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::create_dir_all(&readable).unwrap();
        let _restore = RestorePermissions(
            blocked.clone(),
            std::fs::metadata(&blocked).unwrap().permissions(),
        );
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            std::fs::read_dir(&blocked).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let manager = FileWatcherManager::default();
        #[cfg(target_os = "linux")]
        assert!(manager
            .start_watching(2, blocked.to_string_lossy().into_owned(), |_| {})
            .is_err());
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();

        // When
        let id = manager
            .start_watching(
                1,
                directory.path().to_string_lossy().into_owned(),
                move |event| {
                    let _ = sender.send(event);
                },
            )
            .unwrap();

        // Then
        for (path, is_directory) in [
            (readable.join("file"), false),
            (directory.path().join("created"), true),
            (directory.path().join("created/file"), false),
        ] {
            if is_directory {
                std::fs::create_dir(&path).unwrap();
            } else {
                std::fs::write(&path, "updated").unwrap();
            }
            let expected = path.canonicalize().unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if events.recv().await.unwrap().unwrap().path == expected {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        }
        manager.stop_watching(id).unwrap();
    }
}
