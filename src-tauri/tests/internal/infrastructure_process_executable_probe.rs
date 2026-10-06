pub(crate) mod tests {

    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use releash_lib::test_support::integration::process::resolve_executable;
    use releash_lib::test_support::integration::process::ExecutableProbeResult;

    fn executable(path: &Path) {
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    pub fn test_executable_probe_absolute_pathを解決する() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("agent-cli");
        executable(&path);

        assert_eq!(
            resolve_executable(path.to_str().unwrap(), None),
            ExecutableProbeResult::Resolved(path)
        );
    }

    #[test]
    pub fn test_executable_probe_path上のcommandを絶対pathへ解決する() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("agent-cli");
        executable(&path);

        assert_eq!(
            resolve_executable("agent-cli", Some(temporary.path().as_os_str())),
            ExecutableProbeResult::Resolved(path)
        );
    }

    #[test]
    pub fn test_executable_probe_相対pathを絶対pathへ解決する() {
        let current = std::env::current_dir().unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("relative-provider-")
            .tempdir_in(&current)
            .unwrap();
        let path = temporary.path().join("agent-cli");
        executable(&path);
        let relative = path.strip_prefix(&current).unwrap();

        assert_eq!(
            resolve_executable(relative.to_str().unwrap(), None),
            ExecutableProbeResult::Resolved(path.canonicalize().unwrap())
        );
    }

    #[test]
    pub fn test_executable_probe_壊れたpath要素の後も探索を続ける() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid_directory = temporary.path().join("not-a-directory");
        fs::write(&invalid_directory, "file").unwrap();
        let valid_directory = temporary.path().join("bin");
        fs::create_dir(&valid_directory).unwrap();
        let executable_path = valid_directory.join("agent-cli");
        executable(&executable_path);
        let search_path = std::env::join_paths([invalid_directory, valid_directory]).unwrap();

        assert_eq!(
            resolve_executable("agent-cli", Some(&search_path)),
            ExecutableProbeResult::Resolved(executable_path)
        );
    }

    #[test]
    pub fn test_executable_probe未検出と探索環境なしを区別する() {
        let temporary = tempfile::tempdir().unwrap();

        assert_eq!(
            resolve_executable("missing", Some(temporary.path().as_os_str())),
            ExecutableProbeResult::NotFound
        );
        assert_eq!(
            resolve_executable("missing", None),
            ExecutableProbeResult::SearchPathUnavailable
        );
    }

    #[test]
    pub fn test_executable_probe_directoryと実行権限なしをnot_executableにする() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("not-executable");
        fs::write(&file, "not executable").unwrap();

        assert_eq!(
            resolve_executable(temporary.path().to_str().unwrap(), None),
            ExecutableProbeResult::NotExecutable
        );
        assert_eq!(
            resolve_executable(file.to_str().unwrap(), None),
            ExecutableProbeResult::NotExecutable
        );
    }
}
