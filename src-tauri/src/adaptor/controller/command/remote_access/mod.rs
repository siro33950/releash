pub(crate) mod commands;

const COMMAND_NAMES: &[&str] = &["get_network_info", "detect_vpn_tunnel", "get_connection_qr"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::get_network_info,
        commands::detect_vpn_tunnel,
        commands::get_connection_qr,
    ]
}
