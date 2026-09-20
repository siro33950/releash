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

trait ShutdownChild {
    fn id(&self) -> Option<u32>;
    async fn wait(&mut self) -> std::io::Result<()>;
    fn start_kill(&mut self) -> std::io::Result<()>;
    #[cfg(unix)]
    fn signal_group(&mut self, pgid: i32, signal: i32) -> std::io::Result<()>;
}

impl ShutdownChild for Child {
    fn id(&self) -> Option<u32> {
        self.id()
    }
    async fn wait(&mut self) -> std::io::Result<()> {
        self.wait().await.map(|_| ())
    }
    fn start_kill(&mut self) -> std::io::Result<()> {
        self.start_kill()
    }
    #[cfg(unix)]
    fn signal_group(&mut self, pgid: i32, signal: i32) -> std::io::Result<()> {
        signal_process_group(pgid, signal)
    }
}

pub(crate) async fn staged_shutdown(child: &mut Child, label: &str) {
    staged_shutdown_child(child, label).await;
}

async fn staged_shutdown_child(child: &mut impl ShutdownChild, label: &str) {
    if wait_child(child, FIRST_SHUTDOWN_GRACE, label).await {
        return;
    }
    terminate_child_group(child);
    if wait_child(child, SECOND_SHUTDOWN_GRACE, label).await {
        return;
    }
    kill_child_group(child);
    if let Err(error) = child.wait().await {
        log::error!("failed to reap {label} during shutdown: {error}");
    }
}

async fn wait_child(child: &mut impl ShutdownChild, duration: Duration, label: &str) -> bool {
    match tokio::time::timeout(duration, child.wait()).await {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => {
            log::error!("failed to wait for {label} child: {error}");
            false
        }
        Err(_) => false,
    }
}

#[cfg(unix)]
fn terminate_child_group(child: &mut impl ShutdownChild) {
    if let Some(pid) = child.id() {
        if let Err(error) = child.signal_group(pid as i32, libc::SIGTERM) {
            log::error!("failed to terminate command process group {pid}: {error}");
        }
    } else {
        if let Err(error) = child.start_kill() {
            log::error!("failed to kill command child: {error}");
        }
    }
}

#[cfg(not(unix))]
fn terminate_child_group(child: &mut impl ShutdownChild) {
    if let Err(error) = child.start_kill() {
        log::error!("failed to kill command child: {error}");
    }
}

#[cfg(unix)]
fn kill_child_group(child: &mut impl ShutdownChild) {
    if let Some(pid) = child.id() {
        if let Err(error) = child.signal_group(pid as i32, libc::SIGKILL) {
            log::error!("failed to kill command process group {pid}: {error}");
        }
    }
    if let Err(error) = child.start_kill() {
        log::error!("failed to kill command child: {error}");
    }
}

#[cfg(not(unix))]
fn kill_child_group(child: &mut impl ShutdownChild) {
    if let Err(error) = child.start_kill() {
        log::error!("failed to kill command child: {error}");
    }
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

#[cfg(test)]
#[path = "child_process_test.rs"]
mod child_process_tests;
