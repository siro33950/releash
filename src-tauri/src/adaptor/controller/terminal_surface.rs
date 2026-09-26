use crate::adaptor::presenter::error::AppError;
pub(crate) use crate::adaptor::presenter::terminal_error::{
    invalid_owner_error, invalid_terminal_resize_owner_error, invalid_terminal_write_owner_error,
    terminal_resize_error, terminal_write_error, TerminalCommandError, TerminalCommandOperation,
};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalInputPerformanceSampleV1, TerminalLaunchPerformanceSampleV1,
    TerminalPerformanceSwitchesV1, TerminalSurfaceOwnerV1,
};

pub(crate) fn get_terminal_performance_switches_shared() -> TerminalPerformanceSwitchesV1 {
    telemetry().terminal_performance_switches().into()
}

pub(crate) fn get_performance_real_app_mode_shared() -> bool {
    telemetry().performance_real_app_mode()
}

pub(crate) fn start_terminal_launch_performance_collection_shared() {
    telemetry().start_terminal_launch_collection();
}

pub(crate) fn take_terminal_launch_performance_samples_shared(
) -> Vec<TerminalLaunchPerformanceSampleV1> {
    telemetry()
        .take_terminal_launch_samples()
        .into_iter()
        .map(|sample| TerminalLaunchPerformanceSampleV1 {
            phase: sample.phase.to_string(),
            duration_ms: sample.duration_ms,
        })
        .collect()
}

pub(crate) fn start_terminal_input_performance_collection_shared() {
    telemetry().start_terminal_input_collection();
}

pub(crate) fn take_terminal_input_performance_samples_shared(
) -> Vec<TerminalInputPerformanceSampleV1> {
    telemetry()
        .take_terminal_input_samples()
        .into_iter()
        .map(|sample| TerminalInputPerformanceSampleV1 {
            sequence: sample.sequence,
            on_data_to_command_ingress_ms: sample.on_data_to_command_ingress_ms,
            command_ingress_to_admission_ms: sample.command_ingress_to_admission_ms,
            admission_to_writer_enqueue_ms: sample.admission_to_writer_enqueue_ms,
            writer_enqueue_to_output_read_ms: sample.writer_enqueue_to_output_read_ms,
            output_read_to_model_apply_ms: sample.output_read_to_model_apply_ms,
            model_apply_to_event_publish_ms: sample.model_apply_to_event_publish_ms,
            event_published_at_unix_ms: sample.event_published_at_unix_ms,
        })
        .collect()
}

pub(crate) fn record_terminal_launch_renderer_phase_shared(
    phase: String,
    duration_ms: f64,
) -> Result<(), AppError> {
    if !duration_ms.is_finite() || duration_ms < 0.0 {
        return Err(AppError::invalid_request(
            "Terminal launch renderer duration must be finite and non-negative",
        ));
    }
    let metric = match phase.as_str() {
        "first_xterm_parsed" => crate::usecase::telemetry::TerminalLaunch::FirstXtermParsed,
        "first_paint" => crate::usecase::telemetry::TerminalLaunch::FirstPaint,
        _ => {
            return Err(AppError::invalid_request(
                "Unknown Terminal launch renderer phase",
            ))
        }
    };
    let duration = std::time::Duration::try_from_secs_f64(duration_ms / 1_000.0).map_err(|_| {
        AppError::invalid_request("Terminal launch renderer duration is out of range")
    })?;
    telemetry().record_terminal_launch(metric, duration);
    Ok(())
}

pub(crate) fn write_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    attachment_id: String,
    sequence: u64,
    client_started_at_unix_ms: Option<f64>,
    data: String,
) -> Result<(), AppError> {
    let owner = owner
        .try_into()
        .map_err(invalid_terminal_write_owner_error)?;
    state
        .terminal_surface
        .write_attached(
            &owner,
            &attachment_id,
            sequence,
            client_started_at_unix_ms,
            &data,
        )
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
            .map_err(|error| AppError::new(format!("Terminal resize task failed: {error}")))?
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

fn telemetry() -> crate::usecase::telemetry::TelemetryUsecase<'static> {
    crate::usecase::telemetry::TelemetryUsecase::new(
        &crate::adaptor::gateway::telemetry::TelemetryGateway,
    )
}
