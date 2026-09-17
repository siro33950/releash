use std::io::Read;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};

static CHILD_SPAWNS: parking_lot::RwLock<()> = parking_lot::RwLock::new(());

pub(crate) fn spawn_guard() -> parking_lot::RwLockReadGuard<'static, ()> {
    CHILD_SPAWNS.read()
}

pub(crate) fn watch_parent_pipe() {
    if std::env::var_os("RELEASH_DAEMON_PARENT_PIPE").is_none() {
        return;
    }
    std::env::remove_var("RELEASH_DAEMON_PARENT_PIPE");
    std::thread::spawn(|| {
        let mut byte = [0];
        loop {
            match std::io::stdin().read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    eprintln!("Parent lifetime pipe failed: {error}");
                    break;
                }
            }
        }
        let _stop_spawning = CHILD_SPAWNS.write();
        terminate_descendants(std::process::id());
        std::process::exit(1);
    });
}

pub(crate) fn terminate_descendants(root: u32) {
    let mut system = System::new();
    let mut parents = vec![Pid::from_u32(root)];
    let mut descendants = Vec::new();
    while !parents.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        let children: Vec<_> = system
            .processes()
            .values()
            .filter(|process| {
                process
                    .parent()
                    .is_some_and(|parent| parents.contains(&parent))
            })
            .map(|process| {
                process.kill_with(Signal::Stop);
                process.pid()
            })
            .collect();
        descendants.extend(children.iter().copied());
        parents = children;
    }
    for pid in descendants.into_iter().rev() {
        if let Some(process) = system.process(pid) {
            if !process.kill() {
                eprintln!("Could not terminate daemon child {pid}");
            }
        }
    }
}

#[cfg(test)]
#[path = "parent_lifetime_test.rs"]
mod parent_lifetime_tests;
