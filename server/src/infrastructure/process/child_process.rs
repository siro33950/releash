use std::time::Duration;

use tokio::process::{Child, Command};

pub(crate) const FIRST_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
pub(crate) const SECOND_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub fn configure_process_group(command: &mut Command) {
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

#[cfg(unix)]
pub async fn wait_without_reaping(pid: u32) -> std::io::Result<()> {
    loop {
        {
            let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            // SAFETY: info points to writable siginfo_t storage; WNOWAIT preserves the child identity.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                )
            };
            if result == -1 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(error);
                }
            } else {
                // SAFETY: successful waitid initialized info (zero pid means no exit yet).
                if unsafe { info.assume_init().si_pid() } != 0 {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(crate::common::retry::RetryBackoff::POLL.delay(1, 1.0)).await;
    }
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
pub fn signal_process_group(pgid: i32, signal: i32) -> std::io::Result<()> {
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
        // macOS excludes zombies when signalling groups and reports EPERM for zombie-only groups.
        #[cfg(target_os = "macos")]
        if error.raw_os_error() == Some(libc::EPERM) && !group_has_live_members(pgid)? {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn group_has_live_members(pgid: i32) -> std::io::Result<bool> {
    // SAFETY: a null buffer asks libproc for the required PID capacity.
    let capacity = unsafe { libc::proc_listpgrppids(pgid, std::ptr::null_mut(), 0) };
    if capacity <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut pids = vec![0i32; capacity as usize + 1];
    let bytes = i32::try_from(pids.len() * std::mem::size_of::<i32>())
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    // SAFETY: pids provides the writable buffer of the specified byte length.
    let count = unsafe { libc::proc_listpgrppids(pgid, pids.as_mut_ptr().cast(), bytes) };
    if count <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    if count as usize >= pids.len() {
        return Err(std::io::Error::other(
            "process group PID buffer was truncated",
        ));
    }
    for pid in pids.into_iter().take(count as usize) {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // SAFETY: info is writable proc_bsdinfo storage of exactly size bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if read != size {
            let error = std::io::Error::last_os_error();
            if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error);
        }
        // SAFETY: proc_pidinfo returned the complete initialized structure.
        let info = unsafe { info.assume_init() };
        if info.pbi_pgid == pgid as u32 && info.pbi_status != libc::SZOMB {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "child_process_test.rs"]
mod child_process_tests;
