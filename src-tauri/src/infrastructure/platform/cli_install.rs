#[cfg(any(target_os = "macos", test))]
use std::fmt;
#[cfg(any(target_os = "macos", test))]
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "macos", test))]
const CLI_LINK_PATH: &str = "/usr/local/bin/releash";

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliInstallStatus {
    AlreadyInstalled(PathBuf),
    Installed(PathBuf),
    SkippedTranslocated(PathBuf),
}

#[cfg(any(target_os = "macos", test))]
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

pub(crate) fn ensure_cli_symlink_installed() {
    #[cfg(target_os = "macos")]
    {
        if !should_install_cli_symlink_for_profile(cfg!(debug_assertions)) {
            log::info!("Skipping Releash CLI install on dev build to preserve production CLI");
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(e) => {
                log::warn!("Failed to resolve Releash executable for CLI install: {e}");
                return;
            }
        };
        match install_cli_symlink(&exe, Path::new(CLI_LINK_PATH)) {
            Ok(status) => log::info!("Releash CLI {status}"),
            Err(e) => log::warn!("Failed to install Releash CLI: {e}"),
        }
    }
}

/// `/usr/local/bin/releash` を install してよいかをビルド種別から判定する純粋関数。
///
/// dev ビルドは本番 CLI 名 `releash` を所有しない（spec [01]「dev 起動による本番 CLI の不変性」）。
/// `/usr/local/bin/releash` を debug binary に張り替えると本番 CLI を破壊するため、
/// dev 起動はこの install 経路に関与しない。
#[cfg(any(target_os = "macos", test))]
pub(crate) fn should_install_cli_symlink_for_profile(is_debug_build: bool) -> bool {
    !is_debug_build
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn install_cli_symlink(exe_path: &Path, link_path: &Path) -> Result<CliInstallStatus, String> {
    install_cli_symlink_with_runner(exe_path, link_path, run_admin_script)
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn install_cli_symlink_with_runner<F>(
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

#[cfg(all(unix, any(target_os = "macos", test)))]
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

#[cfg(all(unix, any(target_os = "macos", test)))]
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

#[cfg(all(unix, any(target_os = "macos", test)))]
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

#[cfg(all(unix, any(target_os = "macos", test)))]
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

#[cfg(any(target_os = "macos", test))]
fn is_app_translocated(path: &Path) -> bool {
    path.to_string_lossy().contains("/AppTranslocation/")
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(unix, any(target_os = "macos", test)))]
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

    #[test]
    fn should_install_cli_symlink_skips_dev_build() {
        // dev (debug) ビルドは本番 CLI を所有しないため install しない
        // （spec [01]「dev 起動による本番 CLI の不変性」）。
        assert!(!should_install_cli_symlink_for_profile(true));
    }

    #[test]
    fn should_install_cli_symlink_allows_release_build() {
        // 本番 (release) ビルドは `/usr/local/bin/releash` を更新する。
        assert!(should_install_cli_symlink_for_profile(false));
    }
}
