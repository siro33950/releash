use std::time::Duration;

use tokio::process::{Child, Command};

pub(crate) const FIRST_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
pub(crate) const SECOND_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub(crate) fn configure_process_group(command: &mut Command) {
    #[cfg(unix)]
    // SAFETY: setsid() is async-signal-safe per POSIX and the closure only calls it.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    #[cfg(not(unix))]
    let _ = command;
}

pub(crate) async fn staged_shutdown(child: &mut Child, label: &str) {
    if wait_child(child, FIRST_SHUTDOWN_GRACE, label).await {
        return;
    }
    terminate_child_group(child);
    if wait_child(child, SECOND_SHUTDOWN_GRACE, label).await {
        return;
    }
    kill_child_group(child);
    let _ = child.wait().await;
}

async fn wait_child(child: &mut Child, duration: Duration, label: &str) -> bool {
    match tokio::time::timeout(duration, child.wait()).await {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => {
            log::debug!("failed to wait for {label} child: {error}");
            false
        }
        Err(_) => false,
    }
}

#[cfg(unix)]
pub(crate) fn terminate_child_group(child: &mut Child) {
    if let Some(pid) = child.id() {
        let _ = signal_process_group(pid as i32, libc::SIGTERM);
    } else {
        let _ = child.start_kill();
    }
}

#[cfg(not(unix))]
pub(crate) fn terminate_child_group(child: &mut Child) {
    let _ = child.start_kill();
}

#[cfg(unix)]
pub(crate) fn kill_child_group(child: &mut Child) {
    if let Some(pid) = child.id() {
        let _ = signal_process_group(pid as i32, libc::SIGKILL);
    }
    let _ = child.start_kill();
}

#[cfg(not(unix))]
pub(crate) fn kill_child_group(child: &mut Child) {
    let _ = child.start_kill();
}

#[cfg(unix)]
pub(crate) fn signal_process_group(pgid: i32, signal: i32) -> std::io::Result<()> {
    if pgid <= 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to signal unsafe process group {pgid}"),
        ));
    }
    // SAFETY: kill is called for an explicit process group id.
    let result = unsafe { libc::kill(-pgid, signal) };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn signal_process_group_rejects_unsafe_pgid_values() {
        for pgid in [-1, 0, 1] {
            let error = signal_process_group(pgid, libc::SIGTERM).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn signal_process_group_allows_safe_pgid_values_through_guard() {
        signal_process_group(i32::MAX, 0).unwrap();
    }
}
