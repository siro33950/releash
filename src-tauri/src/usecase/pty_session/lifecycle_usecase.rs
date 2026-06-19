use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionGateway;

pub fn kill(manager: &impl PtySessionGateway, pty_id: u64) -> Result<(), UsecaseError> {
    let snapshot = manager
        .snapshot(pty_id)
        .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
    if !snapshot.exited {
        manager.kill_runtime(pty_id)?;
    }
    manager.remove_session(pty_id);
    Ok(())
}

pub fn kill_by_worktree(manager: &impl PtySessionGateway, worktree_path: &str) -> Vec<u64> {
    let targets = manager.select_kill_targets_by_worktree(worktree_path);
    kill_targets(manager, targets)
}

pub fn gc_by_worktree(
    manager: &impl PtySessionGateway,
    worktree_path: &str,
    keep_session_keys: &[String],
) -> Vec<u64> {
    let targets = manager.select_gc_targets(worktree_path, keep_session_keys);
    kill_targets(manager, targets)
}

pub fn remove_if_exited(manager: &impl PtySessionGateway, pty_id: u64) {
    manager.remove_if_exited(pty_id);
}

fn kill_targets(manager: &impl PtySessionGateway, targets: Vec<u64>) -> Vec<u64> {
    let mut killed = Vec::with_capacity(targets.len());
    for pty_id in targets {
        match kill(manager, pty_id) {
            Ok(()) => killed.push(pty_id),
            Err(e) => log::error!("Failed to kill PTY {}: {}", pty_id, e),
        }
    }
    killed
}
