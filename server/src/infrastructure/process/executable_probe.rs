use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableProbeResult {
    Resolved(PathBuf),
    NotFound,
    NotExecutable,
    SearchPathUnavailable,
    ProbeFailed,
}

pub fn resolve_executable(executable: &str, search_path: Option<&OsStr>) -> ExecutableProbeResult {
    let executable_path = Path::new(executable);
    if executable_path.components().count() > 1 {
        return inspect_candidate(executable_path);
    }
    let Some(search_path) = search_path else {
        return ExecutableProbeResult::SearchPathUnavailable;
    };
    let mut non_executable_found = false;
    let mut probe_failed = false;
    for directory in std::env::split_paths(search_path) {
        let candidate = directory.join(executable_path);
        match inspect_candidate(&candidate) {
            ExecutableProbeResult::Resolved(path) => {
                return ExecutableProbeResult::Resolved(path);
            }
            ExecutableProbeResult::NotExecutable => non_executable_found = true,
            ExecutableProbeResult::ProbeFailed => probe_failed = true,
            ExecutableProbeResult::NotFound | ExecutableProbeResult::SearchPathUnavailable => {}
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            match inspect_candidate(&candidate.with_extension(extension)) {
                ExecutableProbeResult::Resolved(path) => {
                    return ExecutableProbeResult::Resolved(path);
                }
                ExecutableProbeResult::NotExecutable => non_executable_found = true,
                ExecutableProbeResult::ProbeFailed => probe_failed = true,
                ExecutableProbeResult::NotFound | ExecutableProbeResult::SearchPathUnavailable => {}
            }
        }
    }
    if probe_failed {
        ExecutableProbeResult::ProbeFailed
    } else if non_executable_found {
        ExecutableProbeResult::NotExecutable
    } else {
        ExecutableProbeResult::NotFound
    }
}

fn inspect_candidate(path: &Path) -> ExecutableProbeResult {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return ExecutableProbeResult::NotFound;
        }
        Err(_) => return ExecutableProbeResult::ProbeFailed,
    };
    if !metadata.is_file() {
        return ExecutableProbeResult::NotExecutable;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o111 == 0 {
            return ExecutableProbeResult::NotExecutable;
        }
    }
    match std::path::absolute(path) {
        Ok(path) => ExecutableProbeResult::Resolved(path),
        Err(_) => ExecutableProbeResult::ProbeFailed,
    }
}
