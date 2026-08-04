use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::usecase::terminal_surface::error::UsecaseError;

pub fn kill<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    owner: &TerminalSurfaceOwner,
) -> Result<(), UsecaseError> {
    let session_key = owner.stable_key();
    let surface = manager
        .find_summary_by_session_key(&session_key)
        .ok_or_else(|| {
            UsecaseError::Gateway(format!(
                "Terminal Surface not found for owner {session_key}"
            ))
        })?;
    if &surface.owner != owner {
        return Err(UsecaseError::Gateway(format!(
            "Terminal Surface not found for owner {session_key}"
        )));
    }
    remove_and_kill(manager, surface.runtime_generation.value())
}

#[cfg(test)]
pub(crate) fn kill_runtime_generation<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    runtime_generation: u64,
) -> Result<(), UsecaseError> {
    remove_and_kill(manager, runtime_generation)
}

fn remove_and_kill<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    runtime_generation: u64,
) -> Result<(), UsecaseError> {
    let snapshot = manager
        .snapshot(runtime_generation)
        .ok_or_else(|| UsecaseError::Gateway(format!("PTY {runtime_generation} not found")))?;
    if !snapshot.process_state.is_exited() {
        manager.request_runtime_stop(runtime_generation)?;
    }
    manager.wait_runtime_output_drain(runtime_generation)?;
    manager.delete_terminal_checkpoint(&snapshot.session_key)?;
    manager.remove_surface(runtime_generation);
    Ok(())
}

pub fn kill_by_worktree<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    worktree_path: &str,
) -> Vec<u64> {
    let targets = manager.select_kill_targets_by_worktree(worktree_path);
    kill_targets(manager, targets)
}

pub fn gc_by_worktree<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    worktree_path: &str,
    keep_session_keys: &[String],
) -> Vec<u64> {
    let targets = manager.select_gc_targets(worktree_path, keep_session_keys);
    kill_targets(manager, targets)
}

fn kill_targets<G: TerminalSurfaceGateway + ?Sized>(manager: &G, targets: Vec<u64>) -> Vec<u64> {
    let mut killed = Vec::with_capacity(targets.len());
    for runtime_generation in targets {
        match remove_and_kill(manager, runtime_generation) {
            Ok(()) => killed.push(runtime_generation),
            Err(error) => {
                log::error!("Failed to kill PTY {runtime_generation}: {error}");
            }
        }
    }
    killed
}
