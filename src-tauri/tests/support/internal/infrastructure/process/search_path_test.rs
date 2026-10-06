pub(crate) mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use super::super::{capture_login_shell_path_from, LoginShellPathError};

    fn shell_script(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let shell = temporary.path().join("shell");
        fs::write(&shell, contents).unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
        (temporary, shell)
    }

    #[test]
    pub fn test_login_shell_path_startup_outputからpathだけを取得する() {
        let (temporary, shell) = shell_script(
            "#!/bin/sh\nprintf 'startup noise\\n__RELEASH_PATH_BEGIN__/custom/bin:/usr/bin__RELEASH_PATH_END__\\n'\n",
        );

        let path = capture_login_shell_path_from(&shell, temporary.path(), Duration::from_secs(1))
            .unwrap();

        assert_eq!(path, "/custom/bin:/usr/bin");
    }

    #[test]
    pub fn test_login_shell_path_timeoutでshellを終了する() {
        let (temporary, shell) = shell_script("#!/bin/sh\nsleep 10\n");
        let started = std::time::Instant::now();

        let result =
            capture_login_shell_path_from(&shell, temporary.path(), Duration::from_millis(50));

        assert_eq!(result, Err(LoginShellPathError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
