use crate::adaptor::presenter::error::AppError;
pub(crate) use crate::adaptor::presenter::terminal_error::{
    invalid_owner_error, invalid_terminal_resize_owner_error, invalid_terminal_write_owner_error,
    terminal_resize_error, terminal_write_error, TerminalCommandError, TerminalCommandOperation,
};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::terminal::{GetOrSpawnTerminalV1, TerminalSurfaceOwnerV1};

pub(crate) fn write_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    attachment_id: String,
    sequence: u64,
    data: String,
) -> Result<(), AppError> {
    let owner = owner
        .try_into()
        .map_err(invalid_terminal_write_owner_error)?;
    state
        .terminal_surface
        .write_attached(&owner, &attachment_id, sequence, &data)
        .map_err(terminal_write_error)
}

pub(crate) fn write_paths_to_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let owner = owner.try_into().map_err(AppError::invalid_request)?;
    state
        .terminal_surface
        .write_paths(&owner, &paths)
        .map_err(AppError::from_failure)
}

pub(crate) fn resize_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    rows: u16,
    cols: u16,
) -> impl std::future::Future<Output = Result<(), AppError>> + Send + use<> {
    let resize = owner
        .try_into()
        .map_err(invalid_terminal_resize_owner_error)
        .map(|owner| state.terminal_surface.prepare_resize(owner, rows, cols));
    async move {
        super::client::worktree_mutation::spawn_blocking(resize?)
            .await
            .map_err(|error| {
                AppError::from_failure(crate::domain::failure::TechnicalFailure::from(error))
            })?
            .map_err(terminal_resize_error)
    }
}

pub(crate) fn kill_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
) -> Result<(), AppError> {
    let owner = owner.try_into().map_err(AppError::invalid_request)?;
    state
        .terminal_surface
        .kill(&owner)
        .map_err(AppError::from_failure)
}

pub(crate) fn get_or_spawn_terminal_surface_shared(
    state: &AppState,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwnerV1,
    label: Option<String>,
    startup_command: Option<String>,
) -> Result<GetOrSpawnTerminalV1, TerminalCommandError> {
    let owner = owner
        .try_into()
        .map_err(|cause| invalid_owner_error(TerminalCommandOperation::Initialize, cause))?;
    state
        .terminal_surface
        .get_or_spawn(rows, cols, cwd, owner, label, startup_command)
        .map(Into::into)
        .map_err(|error| {
            TerminalCommandError::from_usecase(error, TerminalCommandOperation::Initialize)
        })
}

#[cfg(test)]
#[path = "terminal_surface_test.rs"]
mod terminal_surface_tests;
