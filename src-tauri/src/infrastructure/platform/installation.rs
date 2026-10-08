use std::path::{Path, PathBuf};

pub fn read_only(executable: &Path) -> Result<bool, std::io::Error> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(executable.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { info.assume_init() }.f_flags & libc::MNT_RDONLY as u32 != 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { info.assume_init() }.f_flag & libc::ST_RDONLY != 0)
    }
}
pub fn cli_link(path: &Path) -> Result<Option<Option<PathBuf>>, std::io::Error> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::read_link(path).map(|target| Some(Some(target)))
        }
        Ok(_) => Ok(Some(None)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
pub fn create_cli_link(exe_path: &Path, link_path: &Path) -> Result<(), std::io::Error> {
    let parent = link_path
        .parent()
        .ok_or_else(|| std::io::Error::other("CLI link path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    if std::fs::symlink_metadata(link_path).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(link_path)?;
    }
    std::os::unix::fs::symlink(exe_path, link_path)
}

#[cfg(target_os = "macos")]
fn build_admin_install_script(exe_path: &Path, link_path: &Path) -> Result<String, String> {
    let parent = link_path
        .parent()
        .ok_or_else(|| "CLI link path has no parent".to_string())?;
    Ok(format!(
        "mkdir -p {} && if [ -L {} ] || [ ! -e {} ]; then ln -sfn {} {}; else exit 1; fi",
        shell_quote(parent),
        shell_quote(link_path),
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

#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}
#[cfg(target_os = "macos")]
fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
#[cfg(target_os = "macos")]
pub(crate) fn create_cli_link_as_admin(target: &Path, link: &Path) -> Result<(), String> {
    run_admin_script(&build_admin_install_script(target, link)?)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "installation_test.rs"]
mod installation_tests;
