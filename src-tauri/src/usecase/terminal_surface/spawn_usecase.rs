use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceSpawnReservation, TerminalSurfaceSummary,
};
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceGateway,
};
use crate::domain::terminal_surface::{
    TerminalSurfaceCheckpoint, TerminalSurfaceOwner, TerminalSurfaceStartupCommand,
};
use crate::usecase::terminal_surface::error::UsecaseError;

pub struct GetOrSpawnTerminalOutcome {
    pub surface: TerminalSurfaceSummary,
    pub restored_from_checkpoint: bool,
    pub is_new: bool,
}

#[allow(clippy::too_many_arguments)]
fn spawn_reserved<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    reservation: TerminalSurfaceSpawnReservation,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwner,
    label: Option<String>,
    startup_command: Option<String>,
    initial_terminal_surface: Option<TerminalSurfaceCheckpoint>,
) -> Result<TerminalSurfaceSummary, UsecaseError> {
    let session_key = owner.stable_key();
    let restored_from_checkpoint = initial_terminal_surface.is_some();
    let startup_input = TerminalSurfaceStartupCommand::new(startup_command.as_deref())
        .and_then(|command| command.into_input(restored_from_checkpoint));

    let runtime_generation = manager.next_runtime_generation();
    let (backend_rows, backend_cols) = initial_terminal_surface
        .as_ref()
        .map_or((rows, cols), |surface| (surface.rows, surface.cols));
    let initial_checkpoint = initial_terminal_surface.as_ref().map_or_else(
        || TerminalSurfaceCheckpoint::empty(backend_cols, backend_rows),
        |checkpoint| TerminalSurfaceCheckpoint {
            replay: checkpoint.replay.clone(),
            sequence: checkpoint.sequence,
            cols: checkpoint.cols,
            rows: checkpoint.rows,
        },
    );
    if let Err(error) = manager.spawn_runtime(TerminalRuntimeSpawnRequest {
        runtime_generation,
        session_key: session_key.clone(),
        rows: backend_rows,
        cols: backend_cols,
        cwd,
        exec_command: None,
        initial_terminal_surface,
    }) {
        manager.rollback_spawn_slot(&reservation);
        return Err(error.into());
    }

    let surface =
        TerminalSurface::with_checkpoint(runtime_generation, owner, label, initial_checkpoint);
    let surface_summary = surface.summary();
    manager.insert_surface(surface);
    if let Err(error) = manager.start_output_reader(runtime_generation) {
        cleanup_failed_spawn(manager, runtime_generation);
        manager.rollback_spawn_slot(&reservation);
        return Err(error.into());
    }
    if let Some(startup_input) = startup_input {
        if let Err(error) = manager.write(&session_key, &startup_input) {
            cleanup_failed_spawn(manager, runtime_generation);
            manager.rollback_spawn_slot(&reservation);
            return Err(error.into());
        }
    }

    manager.complete_spawn_slot(&reservation);

    Ok(surface_summary)
}

fn cleanup_failed_spawn<G: TerminalSurfaceGateway + ?Sized>(manager: &G, runtime_generation: u64) {
    if let Some(surface) = manager.snapshot(runtime_generation) {
        match manager.request_runtime_stop(runtime_generation) {
            Ok(()) => {
                if let Err(error) = manager.wait_runtime_output_drain(runtime_generation) {
                    log::error!(
                        "Failed to drain PTY {} during failed spawn cleanup: {}",
                        runtime_generation,
                        error
                    );
                    return;
                }
                match manager.delete_terminal_checkpoint(&surface.session_key) {
                    Ok(()) => {
                        manager.remove_surface(runtime_generation);
                    }
                    Err(error) => {
                        log::error!(
                            "Failed to delete Terminal Surface checkpoint {} during failed spawn cleanup: {}",
                            surface.session_key,
                            error
                        );
                    }
                }
            }
            Err(error) => {
                log::error!(
                    "Failed to kill PTY {} during failed spawn cleanup: {}",
                    runtime_generation,
                    error
                );
            }
        }
    } else {
        manager.remove_runtime(runtime_generation);
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn get_or_spawn<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwner,
    label: Option<String>,
) -> Result<GetOrSpawnTerminalOutcome, UsecaseError> {
    get_or_spawn_with_startup(manager, rows, cols, cwd, owner, label, None)
}

#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn_with_startup<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwner,
    label: Option<String>,
    startup_command: Option<String>,
) -> Result<GetOrSpawnTerminalOutcome, UsecaseError> {
    let session_key = owner.stable_key();
    loop {
        if let Some(surface) = manager.find_summary_by_session_key(&session_key) {
            if surface.owner != owner {
                return Err(UsecaseError::Gateway(
                    "Terminal Surface owner identity collision".to_string(),
                ));
            }
            return Ok(GetOrSpawnTerminalOutcome {
                surface,
                restored_from_checkpoint: false,
                is_new: false,
            });
        }

        let worktree_path = owner.workspace_identity().as_str();
        let reservation = match manager.reserve_spawn_slot(&session_key, Some(worktree_path)) {
            Ok(reservation) => reservation,
            Err(
                crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservationError::OwnerOccupied(_),
            ) => {
                if let Some(surface) = manager.wait_for_spawn_resolution(&session_key) {
                    if surface.owner != owner {
                        return Err(UsecaseError::Gateway(
                            "Terminal Surface owner identity collision".to_string(),
                        ));
                    }
                    return Ok(GetOrSpawnTerminalOutcome {
                        surface,
                        restored_from_checkpoint: false,
                        is_new: false,
                    });
                }
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let restored_terminal_surface = match manager.load_terminal_checkpoint(&session_key) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                manager.rollback_spawn_slot(&reservation);
                return Err(error.into());
            }
        };
        let restored_from_checkpoint = restored_terminal_surface.is_some();
        let surface = spawn_reserved(
            manager,
            reservation,
            rows,
            cols,
            cwd.clone(),
            owner.clone(),
            label.clone(),
            startup_command.clone(),
            restored_terminal_surface,
        )?;

        return Ok(GetOrSpawnTerminalOutcome {
            surface,
            restored_from_checkpoint,
            is_new: true,
        });
    }
}

#[cfg(test)]
#[path = "spawn_usecase_test.rs"]
mod spawn_usecase_tests;
