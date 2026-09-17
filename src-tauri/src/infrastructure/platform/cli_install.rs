#[cfg(any(target_os = "macos", test, debug_assertions))]
use std::fmt;
#[cfg(any(target_os = "macos", test, debug_assertions))]
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const CLI_LINK_PATH: &str = "/usr/local/bin/releash";

#[cfg(any(target_os = "macos", test, debug_assertions))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliInstallStatus {
    AlreadyInstalled(PathBuf),
    Installed(PathBuf),
    SkippedTranslocated(PathBuf),
}

#[cfg(any(target_os = "macos", test, debug_assertions))]
impl fmt::Display for CliInstallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliInstallStatus::AlreadyInstalled(path) => {
                write!(f, "already installed at {}", path.display())
            }
            CliInstallStatus::Installed(path) => write!(f, "installed at {}", path.display()),
            CliInstallStatus::SkippedTranslocated(path) => write!(
                f,
                "skipped because app appears to be translocated: {}",
                path.display()
            ),
        }
    }
}

#[cfg(feature = "desktop")]
pub(crate) fn install_cli() -> Result<String, String> {
    #[cfg(feature = "performance")]
    probe_install_attempt()?;
    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) || cfg!(feature = "performance") {
            return Err("Install the CLI from a release build of Releash.app.".into());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("releash-backend");
        let status = install_cli_symlink(&executable, Path::new(CLI_LINK_PATH))?;
        Ok(format!("Releash CLI {status}"))
    }
    #[cfg(not(target_os = "macos"))]
    Err("CLI installation requires macOS.".into())
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn install_cli_symlink(exe_path: &Path, link_path: &Path) -> Result<CliInstallStatus, String> {
    install_cli_symlink_with_runner(exe_path, link_path, run_admin_script)
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn install_cli_symlink_with_runner<F>(
    exe_path: &Path,
    link_path: &Path,
    mut run_admin_script: F,
) -> Result<CliInstallStatus, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    #[cfg(feature = "performance")]
    probe_install_attempt()?;
    if is_app_translocated(exe_path) {
        return Ok(CliInstallStatus::SkippedTranslocated(
            exe_path.to_path_buf(),
        ));
    }

    if let Some(existing) = existing_symlink_target(link_path)? {
        if existing == exe_path {
            return Ok(CliInstallStatus::AlreadyInstalled(link_path.to_path_buf()));
        }
    } else if link_path.exists() {
        return Err(format!(
            "refusing to overwrite non-symlink CLI path: {}",
            link_path.display()
        ));
    }

    match try_install_cli_symlink(exe_path, link_path) {
        Ok(()) => Ok(CliInstallStatus::Installed(link_path.to_path_buf())),
        Err(direct_error) => {
            let script = build_admin_install_script(exe_path, link_path)?;
            run_admin_script(&script).map_err(|admin_error| {
                format!(
                    "direct install failed ({direct_error}); administrator install failed ({admin_error})"
                )
            })?;
            match existing_symlink_target(link_path)? {
                Some(target) if target == exe_path => {
                    Ok(CliInstallStatus::Installed(link_path.to_path_buf()))
                }
                Some(target) => Err(format!(
                    "administrator install created unexpected symlink target: {} -> {}",
                    link_path.display(),
                    target.display()
                )),
                None => Err(format!(
                    "administrator install did not create symlink: {}",
                    link_path.display()
                )),
            }
        }
    }
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn existing_symlink_target(path: &Path) -> Result<Option<PathBuf>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::read_link(path).map(Some).map_err(|e| {
                format!(
                    "failed to read existing CLI symlink {}: {e}",
                    path.display()
                )
            })
        }
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to stat CLI path {}: {e}", path.display())),
    }
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn try_install_cli_symlink(exe_path: &Path, link_path: &Path) -> Result<(), std::io::Error> {
    let parent = link_path
        .parent()
        .ok_or_else(|| std::io::Error::other("CLI link path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    if std::fs::symlink_metadata(link_path).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(link_path)?;
    }
    std::os::unix::fs::symlink(exe_path, link_path)
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn build_admin_install_script(exe_path: &Path, link_path: &Path) -> Result<String, String> {
    let parent = link_path
        .parent()
        .ok_or_else(|| "CLI link path has no parent".to_string())?;
    Ok(format!(
        "mkdir -p {} && rm -f {} && ln -sf {} {}",
        shell_quote(parent),
        shell_quote(link_path),
        shell_quote(exe_path),
        shell_quote(link_path)
    ))
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn run_admin_script(script: &str) -> Result<(), String> {
    let expression = format!(
        "do shell script {} with administrator privileges",
        applescript_string(script)
    );
    let status = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &expression])
        .status()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("osascript exited with status {status}"))
    }
}

#[cfg(any(target_os = "macos", test, debug_assertions))]
fn is_app_translocated(path: &Path) -> bool {
    path.to_string_lossy().contains("/AppTranslocation/")
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(unix, any(target_os = "macos", test, debug_assertions)))]
fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn install_cli_symlink_creates_link_directly() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("Releash.app/Contents/MacOS/releash");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let link = tmp.path().join("bin/releash");

        let mut admin_called = false;
        let status = install_cli_symlink_with_runner(&exe, &link, |_| {
            admin_called = true;
            Ok(())
        })
        .unwrap();

        assert_eq!(status, CliInstallStatus::Installed(link.clone()));
        assert_eq!(std::fs::read_link(&link).unwrap(), exe);
        assert!(!admin_called);
    }

    #[test]
    fn install_cli_symlink_noops_when_link_is_current() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash");
        std::fs::write(&exe, "").unwrap();
        let link = tmp.path().join("releash-link");
        std::os::unix::fs::symlink(&exe, &link).unwrap();

        let status = install_cli_symlink_with_runner(&exe, &link, |_| {
            panic!("admin runner must not be called");
        })
        .unwrap();

        assert_eq!(status, CliInstallStatus::AlreadyInstalled(link));
    }

    #[test]
    fn install_cli_symlink_replaces_stale_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let old_exe = tmp.path().join("old-releash");
        let new_exe = tmp.path().join("new-releash");
        std::fs::write(&old_exe, "").unwrap();
        std::fs::write(&new_exe, "").unwrap();
        let link = tmp.path().join("bin/releash");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&old_exe, &link).unwrap();

        install_cli_symlink_with_runner(&new_exe, &link, |_| {
            panic!("admin runner must not be called");
        })
        .unwrap();

        assert_eq!(std::fs::read_link(&link).unwrap(), new_exe);
    }

    #[test]
    fn install_cli_symlink_refuses_regular_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash");
        let link = tmp.path().join("releash-link");
        std::fs::write(&exe, "").unwrap();
        std::fs::write(&link, "user owned command").unwrap();

        let err = install_cli_symlink_with_runner(&exe, &link, |_| {
            panic!("admin runner must not be called for non-symlink path");
        })
        .unwrap_err();

        assert!(err.contains("refusing to overwrite non-symlink"));
        assert_eq!(
            std::fs::read_to_string(&link).unwrap(),
            "user owned command"
        );
    }

    #[test]
    fn install_cli_symlink_falls_back_to_admin_script() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("Releash's App.app/Contents/MacOS/releash");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let link = tmp.path().join("missing-parent/releash");

        let status = install_cli_symlink_with_runner(&exe, &link, |script| {
            assert!(script.contains("mkdir -p"));
            assert!(script.contains("'Releash'\\''s App.app"));
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            Ok(())
        })
        .unwrap();

        assert_eq!(status, CliInstallStatus::Installed(link));
    }

    #[test]
    fn install_cli_symlink_skips_app_translocation_path() {
        let exe = PathBuf::from(
            "/private/var/folders/x/AppTranslocation/abc/Releash.app/Contents/MacOS/releash",
        );
        let link = PathBuf::from("/usr/local/bin/releash");

        let status = install_cli_symlink_with_runner(&exe, &link, |_| {
            panic!("admin runner must not be called");
        })
        .unwrap();

        assert_eq!(status, CliInstallStatus::SkippedTranslocated(exe));
    }

    #[test]
    fn applescript_string_escapes_backslashes_and_quotes() {
        assert_eq!(applescript_string(r#"echo "a\b""#), r#""echo \"a\\b\"""#);
    }
}

#[cfg(feature = "performance")]
fn probe_install_attempt() -> Result<(), String> {
    if let Some(path) = std::env::var_os("RELEASH_TEST_CLI_INSTALL_ATTEMPT") {
        std::fs::write(path, b"CLI installation attempted").map_err(|e| e.to_string())?;
        return Err("CLI installation intercepted by acceptance probe".into());
    }
    Ok(())
}
