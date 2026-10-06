pub(crate) mod client;
pub(crate) mod desktop_lifecycle;
pub(crate) mod menu;

type InvokeHandler<R = tauri::Wry> = Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>;

pub(crate) struct CommandRouter<H = InvokeHandler> {
    fallback: H,
    domains: Vec<CommandDomainRoute<H>>,
}

struct CommandDomainRoute<H> {
    command_names: &'static [&'static str],
    handler: H,
}

fn shell_operation(command: &str) -> crate::domain::daemon_supervision::ShellOperation {
    use crate::domain::daemon_supervision::ShellOperation;
    match command {
        "get_daemon_status"
        | "subscribe_daemon_status"
        | "stop_daemon_status_subscription"
        | "retry_daemon"
        | "quit_desktop"
        | "validate_daemon_connection"
        | "get_client_endpoint" => ShellOperation::Supervision,
        _ => ShellOperation::Normal,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationUnavailable {
    ApplicationUnavailable,
}

pub(crate) fn gate_invoke_before_domain_routing<R: tauri::Runtime>(
    invoke: tauri::ipc::Invoke<R>,
) -> Result<tauri::ipc::Invoke<R>, bool> {
    let admitted = {
        let supervisor = tauri::Manager::try_state::<
            std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
        >(invoke.message.webview_ref());
        supervisor.is_none_or(|supervisor| {
            supervisor.command_admitted(shell_operation(invoke.message.command()))
        })
    };
    if !admitted {
        invoke
            .resolver
            .reject(ApplicationUnavailable::ApplicationUnavailable);
        Err(true)
    } else {
        Ok(invoke)
    }
}

impl<H> CommandRouter<H> {
    pub(crate) fn new(fallback: H) -> Self {
        Self {
            fallback,
            domains: Vec::new(),
        }
    }

    pub(crate) fn register_domain(&mut self, command_names: &'static [&'static str], handler: H) {
        self.domains.push(CommandDomainRoute {
            command_names,
            handler,
        });
    }

    fn resolve(&self, command: &str) -> &H {
        self.domain_route_index(command)
            .map(|index| &self.domains[index].handler)
            .unwrap_or(&self.fallback)
    }

    fn domain_route_index(&self, command: &str) -> Option<usize> {
        self.domains
            .iter()
            .position(|domain| domain.command_names.contains(&command))
    }
}

impl<R: tauri::Runtime> CommandRouter<InvokeHandler<R>> {
    pub(crate) fn handle(&self, invoke: tauri::ipc::Invoke<R>) -> bool {
        let invoke = match gate_invoke_before_domain_routing(invoke) {
            Ok(invoke) => invoke,
            Err(handled) => return handled,
        };
        (self.resolve(invoke.message.command()))(invoke)
    }
}

pub(crate) fn register_all(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let app_handler: InvokeHandler = Box::new(|_invoke| false);
    let mut router = CommandRouter::new(app_handler);
    register_shell_commands(&mut router);
    builder.invoke_handler(move |invoke: tauri::ipc::Invoke<tauri::Wry>| router.handle(invoke))
}

fn register_shell_commands(router: &mut CommandRouter) {
    desktop_lifecycle::register(router);
    client::register(router);
    menu::register(router);
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;

#[cfg(test)]
pub(crate) use mod_tests::tests;
