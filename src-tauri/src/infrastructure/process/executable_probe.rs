use std::ffi::OsStr;
use std::path::Path;

pub(crate) fn is_executable(executable: &str, search_path: Option<&OsStr>) -> bool {
    let executable_path = Path::new(executable);
    if executable_path.components().count() > 1 {
        return is_executable_file(executable_path);
    }
    let Some(search_path) = search_path else {
        return false;
    };
    std::env::split_paths(search_path).any(|directory| {
        let candidate = directory.join(executable_path);
        if is_executable_file(&candidate) {
            return true;
        }
        #[cfg(windows)]
        {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|extension| is_executable_file(&candidate.with_extension(extension)))
        }
        #[cfg(not(windows))]
        {
            false
        }
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
