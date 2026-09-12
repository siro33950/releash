use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use tauri::Manager;

use super::CommandRouter;
use crate::adaptor::protocol::client::{ClientEndpoint, CLIENT_WS_PATH};
use crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
use crate::other::AppError;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::repository_usecase::RepositoryUsecase;

pub(super) type CommandHandler = Box<
    dyn Fn(
            Arc<RepositoryUsecase>,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send>>
        + Send
        + Sync,
>;

pub(crate) struct ClientCommandDispatch {
    router: CommandRouter<CommandHandler>,
    repository: Arc<RepositoryUsecase>,
    authority: Arc<ApplicationStartupAuthority>,
}

impl ClientCommandDispatch {
    pub(crate) fn new(
        repository: Arc<RepositoryUsecase>,
        authority: Arc<ApplicationStartupAuthority>,
    ) -> Self {
        let fallback: CommandHandler = Box::new(|_, _| {
            Box::pin(async { Err(AppError::coded("UNKNOWN_COMMAND", "Command was not found")) })
        });
        let mut router = CommandRouter::new(fallback);
        super::repository::register_shared(&mut router);
        Self {
            router,
            repository,
            authority,
        }
    }

    pub(crate) fn contains(&self, command: &str) -> bool {
        self.router.domain_route_index(command).is_some()
    }

    pub(crate) async fn dispatch(&self, command: &str, args: Value) -> Result<Value, AppError> {
        if !super::command_admitted(command, Some(&self.authority)) {
            return Err(AppError::coded(
                "APPLICATION_UNAVAILABLE",
                "Application is unavailable",
            ));
        }
        let handler = self.router.resolve(command);
        handler(self.repository.clone(), args).await
    }
}

pub(super) fn handle_invoke<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
    dispatch: Arc<ClientCommandDispatch>,
) -> bool {
    let command = invoke.message.command().to_string();
    let tauri::ipc::InvokeBody::Json(args) = invoke.message.payload() else {
        invoke.resolver.reject(AppError::coded(
            "INVALID_REQUEST",
            "Command arguments must be JSON",
        ));
        return true;
    };
    let args = args.clone();
    invoke
        .resolver
        .respond_async(async move { dispatch.dispatch(&command, args).await.map_err(Into::into) });
    true
}

pub(super) const COMMAND_NAMES: &[&str] = &["get_client_endpoint"];

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
mod client_tests;
