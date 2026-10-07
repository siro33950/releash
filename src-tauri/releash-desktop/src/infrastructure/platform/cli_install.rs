#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
use std::fmt;
#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const CLI_LINK_PATH: &str = "/usr/local/bin/releash";

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliInstallStatus {
    AlreadyInstalled(PathBuf),
    Installed(PathBuf),
    SkippedTranslocated(PathBuf),
}

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
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

pub(crate) fn install_cli() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) {
            return Err("Install the CLI from a release build of Releash.app.".into());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("releash");
        let status = install_cli_symlink(&executable, Path::new(CLI_LINK_PATH))?;
        Ok(format!("Releash CLI {status}"))
    }
    #[cfg(not(target_os = "macos"))]
    Err("CLI installation requires macOS.".into())
}

#[cfg(target_os = "macos")]
fn install_cli_symlink(exe_path: &Path, link_path: &Path) -> Result<CliInstallStatus, String> {
    install_cli_symlink_with_runner(exe_path, link_path, run_admin_script)
}

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
pub fn install_cli_symlink_with_runner<F>(
    exe_path: &Path,
    link_path: &Path,
    mut run_admin_script: F,
) -> Result<CliInstallStatus, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
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

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
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

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
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

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
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

#[cfg(target_os = "macos")]
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

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
fn is_app_translocated(path: &Path) -> bool {
    path.to_string_lossy().contains("/AppTranslocation/")
}

#[cfg(all(unix, any(target_os = "macos", test, feature = "test-support")))]
fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(all(test, unix))]
#[path = "cli_install_test.rs"]
mod cli_install_tests;
