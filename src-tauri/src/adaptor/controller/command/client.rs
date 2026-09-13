use super::CommandRouter;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::protocol::client::{ClientEndpoint, CLIENT_WS_PATH};
use crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn handle_registered_invoke<R: tauri::Runtime>(invoke: tauri::ipc::Invoke<R>) -> bool {
    let Some(dispatch) = invoke
        .message
        .state_ref()
        .try_get::<Arc<ClientCommandDispatch>>()
        .map(|state| state.inner().clone())
    else {
        invoke.resolver.reject(crate::other::AppError::coded(
            "APPLICATION_UNAVAILABLE",
            "Application is unavailable",
        ));
        return true;
    };
    if !dispatch.contains(invoke.message.command()) {
        return false;
    }
    handle_invoke(invoke, dispatch)
}

pub(super) fn handle_invoke<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
    dispatch: Arc<ClientCommandDispatch>,
) -> bool {
    let command = invoke.message.command().to_string();
    let tauri::ipc::InvokeBody::Json(args) = invoke.message.payload() else {
        invoke.resolver.reject(crate::other::AppError::coded(
            "INVALID_REQUEST",
            "Command arguments must be JSON",
        ));
        return true;
    };
    let request = wire::CommandRequest::from_value(&command, args.clone());
    invoke.resolver.respond_async(async move {
        let result = match request {
            Ok(request) => {
                dispatch
                    .dispatch(request.command.expect("parsed command"))
                    .await
            }
            Err(error) => Err(crate::adaptor::controller::client::invalid_request(error)),
        };
        match result {
            Ok(command) => wire::from_value(wire::CommandResult {
                command: Some(command),
            })
            .map_err(Into::into),
            Err(error) => Err(wire::from_value(error).expect("command error").into()),
        }
    });
    true
}

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
