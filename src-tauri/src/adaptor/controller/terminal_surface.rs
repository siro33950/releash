use serde::Serialize;

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalInputPerformanceSampleV1, TerminalLaunchPerformanceSampleV1,
    TerminalPerformanceSwitchesV1, TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1,
    TerminalSurfaceSummaryV1,
};
use crate::usecase::terminal_surface::application::{
    TerminalSurfaceApplication, TerminalSurfaceAttachmentStream,
};
use crate::usecase::terminal_surface::error::UsecaseError;

#[derive(Clone, Copy)]
pub(crate) enum TerminalCommandErrorCode {
    PtyError,
    InvalidRequest,
}

impl TerminalCommandErrorCode {
    fn code(self) -> &'static str {
        match self {
            Self::PtyError => "PTY_ERROR",
            Self::InvalidRequest => "INVALID_REQUEST",
        }
    }
}

pub(crate) fn invalid_owner_error(
    operation: TerminalCommandOperation,
    internal_cause: String,
) -> TerminalCommandError {
    let code = TerminalCommandErrorCode::InvalidRequest;
    log::warn!(
        "Terminal command failed: operation={} code={} cause={}",
        operation.name(),
        code.code(),
        internal_cause
    );
    TerminalCommandError {
        code: code.code().to_string(),
        message: operation.message(code).to_string(),
    }
}

pub(crate) fn invalid_terminal_write_owner_error(internal_cause: String) -> String {
    log::warn!(
        "Terminal command failed: operation=write_terminal_surface code=INVALID_REQUEST cause={}",
        internal_cause
    );
    "Terminal input could not be sent because the request is invalid.".to_string()
}

pub(crate) fn terminal_write_error(error: UsecaseError) -> String {
    log::error!(
        "Terminal command failed: operation=write_terminal_surface code=PTY_ERROR cause={}",
        error
    );
    "Terminal input could not be sent. Try again.".to_string()
}

pub(crate) fn invalid_terminal_resize_owner_error(internal_cause: String) -> String {
    log::warn!(
        "Terminal command failed: operation=resize_terminal_surface code=INVALID_REQUEST cause={}",
        internal_cause
    );
    "Terminal resize failed because the request is invalid.".to_string()
}

pub(crate) fn terminal_resize_error(error: UsecaseError) -> String {
    log::error!(
        "Terminal command failed: operation=resize_terminal_surface code=PTY_ERROR cause={}",
        error
    );
    "Terminal resize failed. Try again.".to_string()
}

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
) -> Result<(), String> {
    if !duration_ms.is_finite() || duration_ms < 0.0 {
        return Err(
            "Terminal launch renderer duration must be finite and non-negative".to_string(),
        );
    }
    let metric = match phase.as_str() {
        "first_xterm_parsed" => crate::other::telemetry::TerminalLaunch::FirstXtermParsed,
        "first_paint" => crate::other::telemetry::TerminalLaunch::FirstPaint,
        _ => return Err("Unknown Terminal launch renderer phase".to_string()),
    };
    let duration = std::time::Duration::try_from_secs_f64(duration_ms / 1_000.0)
        .map_err(|_| "Terminal launch renderer duration is out of range".to_string())?;
    telemetry().record_terminal_launch(metric, duration);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalCommandError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalCommandOperation {
    Initialize,
    GetExisting,
    Attach,
    Resynchronize,
}

impl TerminalCommandOperation {
    fn attachment(recovery: bool) -> Self {
        if recovery {
            Self::Resynchronize
        } else {
            Self::Attach
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Initialize => "get_or_spawn_terminal_surface",
            Self::GetExisting => "get_terminal_surface",
            Self::Attach => "attach_terminal_surface",
            Self::Resynchronize => "attach_terminal_surface_recovery",
        }
    }

    fn message(self, code: TerminalCommandErrorCode) -> &'static str {
        match (self, code) {
            (Self::Initialize, TerminalCommandErrorCode::PtyError) => {
                "Terminal initialization failed. Try again."
            }
            (Self::Initialize, TerminalCommandErrorCode::InvalidRequest) => {
                "Terminal initialization failed because the request is invalid."
            }
            (Self::GetExisting | Self::Attach, TerminalCommandErrorCode::PtyError) => {
                "Terminal attachment failed. Try again."
            }
            (Self::GetExisting | Self::Attach, TerminalCommandErrorCode::InvalidRequest) => {
                "Terminal attachment failed because the request is invalid."
            }
            (Self::Resynchronize, TerminalCommandErrorCode::PtyError) => {
                "Terminal resynchronization failed. Try again."
            }
            (Self::Resynchronize, TerminalCommandErrorCode::InvalidRequest) => {
                "Terminal resynchronization failed because the request is invalid."
            }
        }
    }
}

impl TerminalCommandError {
    fn from_usecase(error: UsecaseError, operation: TerminalCommandOperation) -> Self {
        let internal_cause = error.to_string();
        let code = match error {
            UsecaseError::Gateway(_)
            | UsecaseError::OwnerConflict
            | UsecaseError::PtySpawn { .. }
            | UsecaseError::OtherSpawnFailure { .. } => TerminalCommandErrorCode::PtyError,
        };
        log::error!(
            "Terminal command failed: operation={} code={} cause={}",
            operation.name(),
            code.code(),
            internal_cause
        );
        Self {
            code: code.code().to_string(),
            message: operation.message(code).to_string(),
        }
    }
}

pub(crate) fn write_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    attachment_id: String,
    sequence: u64,
    client_started_at_unix_ms: Option<f64>,
    data: String,
) -> Result<(), String> {
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
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .write_paths(&owner, &paths)
        .map_err(|error| error.to_string())
}

pub(crate) fn resize_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
    rows: u16,
    cols: u16,
) -> impl std::future::Future<Output = Result<(), String>> + Send + use<> {
    let resize = owner
        .try_into()
        .map_err(invalid_terminal_resize_owner_error)
        .map(|owner| state.terminal_surface.prepare_resize(owner, rows, cols));
    async move {
        tokio::task::spawn_blocking(resize?)
            .await
            .map_err(|error| format!("Terminal resize task failed: {error}"))?
            .map_err(terminal_resize_error)
    }
}

pub(crate) fn get_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
) -> Result<TerminalSurfaceSummaryV1, TerminalCommandError> {
    let owner = owner
        .try_into()
        .map_err(|cause| invalid_owner_error(TerminalCommandOperation::GetExisting, cause))?;
    state
        .terminal_surface
        .get_summary(&owner)
        .map(Into::into)
        .map_err(|error| {
            TerminalCommandError::from_usecase(error, TerminalCommandOperation::GetExisting)
        })
}

pub(crate) async fn forward_terminal_surface_attachment<F>(
    application: std::sync::Arc<TerminalSurfaceApplication>,
    attachment_id: String,
    mut attachment: TerminalSurfaceAttachmentStream,
    mut send: F,
) where
    F: FnMut(TerminalSurfaceStreamItemV1) -> Result<(), String>,
{
    while let Some(item) = attachment.next().await {
        if send(item.into()).is_err() {
            break;
        }
    }
    application.detach(&attachment_id);
}

pub(crate) fn detach_terminal_surface_shared(state: &AppState, attachment_id: String) {
    state.terminal_surface.detach(&attachment_id);
}

pub(crate) fn ack_terminal_surface_output_shared(
    state: &AppState,
    attachment_id: String,
    sequence: u64,
) {
    state
        .terminal_surface
        .acknowledge_output(&attachment_id, sequence);
}

pub(crate) fn kill_terminal_surface_shared(
    state: &AppState,
    owner: TerminalSurfaceOwnerV1,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .kill(&owner)
        .map_err(|error| error.to_string())
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

pub(crate) fn attach(
    application: &TerminalSurfaceApplication,
    attachment_id: &str,
    owner: TerminalSurfaceOwnerV1,
    recovery: bool,
) -> Result<TerminalSurfaceAttachmentStream, TerminalCommandError> {
    if attachment_id.is_empty() || attachment_id.len() > 128 {
        return Err(TerminalCommandError {
            code: TerminalCommandErrorCode::InvalidRequest.code().to_string(),
            message: "Invalid attachment ID".to_string(),
        });
    }
    let operation = TerminalCommandOperation::attachment(recovery);
    let owner = owner
        .try_into()
        .map_err(|cause| invalid_owner_error(operation, cause))?;
    application
        .attach(attachment_id, &owner)
        .map_err(|error| TerminalCommandError::from_usecase(error, operation))
}

#[cfg(test)]
#[path = "terminal_surface_test.rs"]
mod terminal_surface_tests;

fn telemetry() -> crate::usecase::telemetry::TelemetryUsecase<'static> {
    crate::usecase::telemetry::TelemetryUsecase::new(
        &crate::adaptor::gateway::telemetry::TelemetryGateway,
    )
}
