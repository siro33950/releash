pub(crate) const COMMAND_NAMES: &[&str] = &[
    "ack_terminal_surface_output",
    "attach_terminal_surface",
    "detach_terminal_surface",
    "write_terminal_surface",
    "write_paths_to_terminal_surface",
    "resize_terminal_surface",
    "record_terminal_launch_renderer_phase",
    "start_terminal_launch_performance_collection",
    "take_terminal_launch_performance_samples",
    "start_terminal_input_performance_collection",
    "take_terminal_input_performance_samples",
    "kill_terminal_surface",
    "get_performance_real_app_mode",
    "get_terminal_performance_switches",
    "get_terminal_surface",
    "get_or_spawn_terminal_surface",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync {
    let shell: super::InvokeHandler<R> =
        Box::new(tauri::generate_handler![attach_terminal_surface]);
    move |invoke: tauri::ipc::Invoke<R>| {
        if ["attach_terminal_surface"].contains(&invoke.message.command()) {
            shell(invoke)
        } else {
            super::client::handle_registered_invoke(invoke)
        }
    }
}
use crate::adaptor::controller::client::terminal_surface::commands::forward_terminal_surface_attachment;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::controller::terminal_surface::attach;
use crate::adaptor::controller::terminal_surface::TerminalCommandError;
use crate::adaptor::protocol::terminal::{TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1};
use tauri::{ipc::Channel, State};
#[tauri::command(async)]
pub fn attach_terminal_surface(
    state: State<'_, AppState>,
    attachment_id: String,
    owner: TerminalSurfaceOwnerV1,
    recovery: bool,
    on_event: Channel<TerminalSurfaceStreamItemV1>,
) -> Result<(), TerminalCommandError> {
    let application = state.terminal_surface.clone();
    let attachment = attach(&application, &attachment_id, owner, recovery)?;
    tauri::async_runtime::spawn(forward_terminal_surface_attachment(
        application,
        attachment_id,
        attachment,
        move |item| on_event.send(item).map_err(|error| error.to_string()),
    ));
    Ok(())
}
