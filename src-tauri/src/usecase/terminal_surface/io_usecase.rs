use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::usecase::terminal_surface::error::UsecaseError;

use crate::domain::shell::join_quoted_paths;
use crate::domain::terminal_surface::TerminalSurfaceOwner;

pub fn write<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    owner: &TerminalSurfaceOwner,
    data: &str,
) -> Result<(), UsecaseError> {
    manager
        .write(&owner.stable_key(), data)
        .map_err(UsecaseError::from)
}

pub fn write_paths<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    owner: &TerminalSurfaceOwner,
    paths: &[String],
) -> Result<(), UsecaseError> {
    if paths.is_empty() {
        return Ok(());
    }
    let data = join_quoted_paths(paths);
    manager
        .write(&owner.stable_key(), &data)
        .map_err(UsecaseError::from)
}

pub fn resize<G: TerminalSurfaceGateway + ?Sized>(
    manager: &G,
    owner: &TerminalSurfaceOwner,
    rows: u16,
    cols: u16,
) -> Result<(), UsecaseError> {
    manager
        .resize(&owner.stable_key(), rows, cols)
        .map_err(UsecaseError::from)
}

#[cfg(test)]
#[path = "io_usecase_test.rs"]
mod io_usecase_tests;
