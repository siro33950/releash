use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PATH_BEGIN: &[u8] = b"__RELEASH_PATH_BEGIN__";
const PATH_END: &[u8] = b"__RELEASH_PATH_END__";
pub(crate) const LOGIN_SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginShellPathError {
    Spawn,
    Wait,
    Output,
    Unsuccessful,
    InvalidOutput,
    Timeout,
}

pub(crate) trait SearchPathSource: Send + Sync {
    fn load(&self) -> Result<OsString, LoginShellPathError>;
}

pub(crate) struct LoginShellSearchPathSource;

impl SearchPathSource for LoginShellSearchPathSource {
    fn load(&self) -> Result<OsString, LoginShellPathError> {
        capture_login_shell_path(LOGIN_SHELL_PATH_TIMEOUT)
    }
}

pub(crate) fn capture_login_shell_path(timeout: Duration) -> Result<OsString, LoginShellPathError> {
    let default_shell = if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/sh"
    };
    let shell = std::env::var_os("SHELL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| default_shell.into());
    let home = dirs::home_dir().ok_or(LoginShellPathError::Spawn)?;
    capture_login_shell_path_from(&shell, &home, timeout)
}

fn capture_login_shell_path_from(
    shell: &Path,
    home: &Path,
    timeout: Duration,
) -> Result<OsString, LoginShellPathError> {
    let mut output = tempfile::tempfile().map_err(|_| LoginShellPathError::Output)?;
    let child_output = output
        .try_clone()
        .map_err(|_| LoginShellPathError::Output)?;
    let mut command = Command::new(shell);
    command
        .arg("-ilc")
        .arg("printf '__RELEASH_PATH_BEGIN__%s__RELEASH_PATH_END__' \"$PATH\"")
        .current_dir(home)
        .env("DISABLE_AUTO_UPDATE", "true")
        .stdout(Stdio::from(child_output))
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn().map_err(|_| LoginShellPathError::Spawn)?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => thread::sleep(
                Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
            ),
            Ok(None) => {
                kill_process_group(&mut child);
                let _ = child.wait();
                return Err(LoginShellPathError::Timeout);
            }
            Err(_) => {
                kill_process_group(&mut child);
                let _ = child.wait();
                return Err(LoginShellPathError::Wait);
            }
        }
    };
    if !status.success() {
        return Err(LoginShellPathError::Unsuccessful);
    }
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| LoginShellPathError::Output)?;
    let mut bytes = Vec::new();
    output
        .read_to_end(&mut bytes)
        .map_err(|_| LoginShellPathError::Output)?;
    extract_path(&bytes)
}

fn extract_path(output: &[u8]) -> Result<OsString, LoginShellPathError> {
    let begin = output
        .windows(PATH_BEGIN.len())
        .rposition(|candidate| candidate == PATH_BEGIN)
        .ok_or(LoginShellPathError::InvalidOutput)?
        + PATH_BEGIN.len();
    let end = output[begin..]
        .windows(PATH_END.len())
        .position(|candidate| candidate == PATH_END)
        .ok_or(LoginShellPathError::InvalidOutput)?
        + begin;
    Ok(OsString::from_vec(output[begin..end].to_vec()))
}

fn kill_process_group(child: &mut std::process::Child) {
    let process_id = child.id();
    if process_id > 1 {
        // SAFETY: process_group(0) created an isolated group whose id is the child pid.
        let _ = unsafe { libc::kill(-(process_id as i32), libc::SIGKILL) };
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    use super::{capture_login_shell_path_from, LoginShellPathError};

    fn shell_script(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let shell = temporary.path().join("shell");
        fs::write(&shell, contents).unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
        (temporary, shell)
    }

    #[test]
    fn test_login_shell_path_startup_outputからpathだけを取得する() {
        let (temporary, shell) = shell_script(
            "#!/bin/sh\nprintf 'startup noise\\n__RELEASH_PATH_BEGIN__/custom/bin:/usr/bin__RELEASH_PATH_END__\\n'\n",
        );

        let path = capture_login_shell_path_from(&shell, temporary.path(), Duration::from_secs(1))
            .unwrap();

        assert_eq!(path, "/custom/bin:/usr/bin");
    }

    #[test]
    fn test_login_shell_path_timeoutでshellを終了する() {
        let (temporary, shell) = shell_script("#!/bin/sh\nsleep 10\n");
        let started = Instant::now();

        let result =
            capture_login_shell_path_from(&shell, temporary.path(), Duration::from_millis(50));

        assert_eq!(result, Err(LoginShellPathError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
