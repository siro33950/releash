use super::CommandRouter;
use crate::adaptor::protocol::client::{ClientEndpoint, CLIENT_WS_PATH};
use crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
use tauri::Manager;

pub(crate) const COMMAND_NAMES: &[&str] = &["get_client_endpoint"];

pub(crate) fn register(router: &mut CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![get_client_endpoint]),
    );
}

#[tauri::command(async)]
fn get_client_endpoint(app: tauri::AppHandle) -> Option<ClientEndpoint> {
    client_endpoint(&app)
}

pub(crate) fn client_endpoint<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Option<ClientEndpoint> {
    let endpoint = app.try_state::<crate::adaptor::controller::state::TerminalStreamEndpoint>()?;
    Some(ClientEndpoint {
        url: format!("ws://127.0.0.1:{}{CLIENT_WS_PATH}", endpoint.port),
        auth_subprotocol: format!("{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{}", endpoint.token),
    })
}

#[cfg(test)]
#[path = "client_test.rs"]
pub(crate) mod client_tests;
