pub(crate) mod commands;

pub(super) const COMMAND_NAMES: &[&str] = &[
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
    "get_terminal_stream_endpoint",
    "get_terminal_surface",
    "get_or_spawn_terminal_surface",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::ack_terminal_surface_output,
        commands::attach_terminal_surface,
        commands::detach_terminal_surface,
        commands::write_terminal_surface,
        commands::write_paths_to_terminal_surface,
        commands::resize_terminal_surface,
        commands::record_terminal_launch_renderer_phase,
        commands::start_terminal_launch_performance_collection,
        commands::take_terminal_launch_performance_samples,
        commands::start_terminal_input_performance_collection,
        commands::take_terminal_input_performance_samples,
        commands::kill_terminal_surface,
        commands::get_performance_real_app_mode,
        commands::get_terminal_performance_switches,
        commands::get_terminal_stream_endpoint,
        commands::get_terminal_surface,
        commands::get_or_spawn_terminal_surface,
    ]
}
