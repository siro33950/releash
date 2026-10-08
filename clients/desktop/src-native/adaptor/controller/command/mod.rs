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
