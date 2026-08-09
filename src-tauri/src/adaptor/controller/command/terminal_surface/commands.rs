use serde::Serialize;
use tauri::{ipc::Channel, Manager, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalInputPerformanceSampleV1, TerminalLaunchPerformanceSampleV1,
    TerminalPerformanceSwitchesV1, TerminalStreamEndpointV1, TerminalSurfaceAvailabilityV1,
    TerminalSurfaceInfoV1, TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1,
    TerminalSurfaceSummaryV1, TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX, TERMINAL_WS_PATH,
};
use crate::usecase::terminal_surface::application::{
    TerminalSurfaceApplication, TerminalSurfaceAttachmentStream,
};
use crate::usecase::terminal_surface::error::UsecaseError;

const PTY_ERROR_CODE_CAP_REACHED: &str = "CAP_REACHED";
const PTY_ERROR_CODE_GENERIC: &str = "PTY_ERROR";
const PTY_ERROR_CODE_INVALID_REQUEST: &str = "INVALID_REQUEST";

fn invalid_owner_error(message: String) -> TerminalCommandError {
    TerminalCommandError {
        code: PTY_ERROR_CODE_INVALID_REQUEST.to_string(),
        message,
    }
}

#[tauri::command(async)]
pub fn get_terminal_performance_switches() -> TerminalPerformanceSwitchesV1 {
    crate::other::performance_switches::terminal_performance_switches().into()
}

#[tauri::command(async)]
pub fn get_terminal_stream_endpoint(app: tauri::AppHandle) -> Option<TerminalStreamEndpointV1> {
    if crate::other::performance_switches::terminal_performance_switches()
        .disable_terminal_websocket
    {
        return None;
    }
    let endpoint = app.try_state::<crate::adaptor::controller::state::TerminalStreamEndpoint>()?;
    Some(TerminalStreamEndpointV1 {
        url: format!("ws://127.0.0.1:{}{}", endpoint.port, TERMINAL_WS_PATH),
        auth_subprotocol: format!(
            "{}{}",
            TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX, endpoint.token
        ),
    })
}

#[tauri::command(async)]
pub fn get_performance_real_app_mode() -> bool {
    crate::other::performance_switches::performance_real_app_mode()
}

#[tauri::command(async)]
pub fn start_terminal_launch_performance_collection() {
    crate::other::telemetry::start_terminal_launch_sample_collection();
}

#[tauri::command(async)]
pub fn take_terminal_launch_performance_samples() -> Vec<TerminalLaunchPerformanceSampleV1> {
    crate::other::telemetry::take_terminal_launch_samples()
        .into_iter()
        .map(|sample| TerminalLaunchPerformanceSampleV1 {
            phase: sample.phase.to_string(),
            duration_ms: sample.duration_ms,
        })
        .collect()
}

#[tauri::command(async)]
pub fn start_terminal_input_performance_collection() {
    crate::other::telemetry::start_terminal_input_sample_collection();
}

#[tauri::command(async)]
pub fn take_terminal_input_performance_samples() -> Vec<TerminalInputPerformanceSampleV1> {
    crate::other::telemetry::take_terminal_input_samples()
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

#[tauri::command(async)]
pub fn record_terminal_launch_renderer_phase(
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
    crate::other::telemetry::record_terminal_launch(metric, duration);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalCommandError {
    pub code: String,
    pub message: String,
}

impl From<UsecaseError> for TerminalCommandError {
    fn from(error: UsecaseError) -> Self {
        let code = match error {
            UsecaseError::CapReached(_) => PTY_ERROR_CODE_CAP_REACHED,
            UsecaseError::Gateway(_) => PTY_ERROR_CODE_GENERIC,
        };
        Self {
            code: code.to_string(),
            message: error.to_string(),
        }
    }
}

#[tauri::command(async)]
pub fn write_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    attachment_id: String,
    sequence: u64,
    client_started_at_unix_ms: Option<f64>,
    data: String,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .write_attached(
            &owner,
            &attachment_id,
            sequence,
            client_started_at_unix_ms,
            &data,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
pub fn write_paths_to_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    paths: Vec<String>,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .write_paths(&owner, &paths)
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
pub fn resize_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .resize(&owner, rows, cols)
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
pub fn list_terminal_surfaces(state: State<'_, AppState>) -> Vec<TerminalSurfaceInfoV1> {
    state
        .terminal_surface
        .list()
        .into_iter()
        .map(Into::into)
        .collect()
}

#[tauri::command(async)]
pub fn reconcile_terminal_surfaces(
    state: State<'_, AppState>,
    session_keys: Vec<String>,
) -> TerminalSurfaceAvailabilityV1 {
    TerminalSurfaceAvailabilityV1 {
        unavailable_session_keys: state.terminal_surface.reconcile_unavailable(&session_keys),
    }
}

#[tauri::command(async)]
pub fn get_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
) -> Result<TerminalSurfaceSummaryV1, TerminalCommandError> {
    let owner = owner.try_into().map_err(invalid_owner_error)?;
    state
        .terminal_surface
        .get_summary(&owner)
        .map(Into::into)
        .map_err(TerminalCommandError::from)
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

#[tauri::command(async)]
pub fn attach_terminal_surface(
    state: State<'_, AppState>,
    attachment_id: String,
    owner: TerminalSurfaceOwnerV1,
    on_event: Channel<TerminalSurfaceStreamItemV1>,
) -> Result<(), TerminalCommandError> {
    let owner = owner.try_into().map_err(invalid_owner_error)?;
    let application = state.terminal_surface.clone();
    let attachment = application
        .attach(&attachment_id, &owner)
        .map_err(TerminalCommandError::from)?;
    tauri::async_runtime::spawn(forward_terminal_surface_attachment(
        application,
        attachment_id,
        attachment,
        move |item| on_event.send(item).map_err(|error| error.to_string()),
    ));
    Ok(())
}

#[tauri::command(async)]
pub fn detach_terminal_surface(state: State<'_, AppState>, attachment_id: String) {
    state.terminal_surface.detach(&attachment_id);
}

#[tauri::command(async)]
pub fn ack_terminal_surface_output(
    state: State<'_, AppState>,
    attachment_id: String,
    sequence: u64,
) {
    state
        .terminal_surface
        .acknowledge_output(&attachment_id, sequence);
}

#[tauri::command(async)]
pub fn kill_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .kill(&owner)
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn_terminal_surface(
    state: State<'_, AppState>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwnerV1,
    label: Option<String>,
    startup_command: Option<String>,
) -> Result<GetOrSpawnTerminalV1, TerminalCommandError> {
    let owner = owner.try_into().map_err(invalid_owner_error)?;
    state
        .terminal_surface
        .get_or_spawn(rows, cols, cwd, owner, label, startup_command)
        .map(Into::into)
        .map_err(TerminalCommandError::from)
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod commands_tests;
