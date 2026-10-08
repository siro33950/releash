use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::client::CommandName;
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

pub(crate) type CommandHandler = Box<
    dyn Fn(
            wire::command_request::Command,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<wire::command_result::Command, wire::CommandFailure>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

pub struct ClientCommandDispatch {
    handlers: HashMap<&'static str, Arc<CommandHandler>>,
    pub(crate) daemon: crate::usecase::daemon::DaemonUsecase,
    pub(super) publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    mutations: Option<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
}

impl ClientCommandDispatch {
    pub fn new(daemon: crate::usecase::daemon::DaemonUsecase) -> Self {
        Self {
            handlers: HashMap::new(),
            mutations: None,
            daemon,
            publisher: None,
        }
    }

    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.publisher = Some(publisher);
        self
    }

    pub fn register_dependencies(&mut self, deps: &super::ClientDependencies) {
        self.daemon = deps.daemon.clone();
        self.mutations = deps.workflow_runtime_usecase.clone();
        super::repository::register_shared(self, deps);
        super::code::register_shared(self, deps);
        super::comment::register_shared(self, deps);
        super::agent_session::register_shared(self, deps);
        super::terminal_surface::register_shared(self, deps);
        super::workflow::register_shared(self, deps);
        super::workspace_tree::register_shared(self, deps);
        super::workspace_state::register_shared(self, deps);
        super::app_config::register_shared(self, deps);
        super::notion::register_shared(self, deps);
        super::git_host::register_shared(self, deps);
        super::external_editor::register_shared(self, deps);
        super::telemetry::register_shared(self, deps);
        super::application_lifecycle::register_shared(self, deps);
        super::installation::register_shared(self, deps);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_worktree_mutations(
        mut self,
        runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    ) -> Self {
        self.mutations = Some(runtime);
        self
    }
    pub fn register_domain(&mut self, names: &'static [&'static str], handler: CommandHandler) {
        assert_eq!(names.len(), 1);
        assert!(self.handlers.insert(names[0], Arc::new(handler)).is_none());
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn dispatch(
        &self,
        command: wire::command_request::Command,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<wire::command_result::Command, wire::CommandFailure>> + Send,
        >,
    > {
        self.dispatch_admitted(command)
    }
    pub fn dispatch_admitted(
        &self,
        command: wire::command_request::Command,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<wire::command_result::Command, wire::CommandFailure>> + Send,
        >,
    > {
        if self.mutations.is_none() {
            if let Err(error) = super::worktree_mutation::admit(None, &command) {
                return Box::pin(std::future::ready(Err(error)));
            }
            if let Some(handler) = self.handlers.get(command.name()) {
                return handler(command);
            }
        }
        let handler = self.handlers.get(command.name()).cloned();
        let mutations = self.mutations.clone();
        Box::pin(async move {
            let (guards, command) = crate::common::operation_context::spawn_blocking(move || {
                let guards = super::worktree_mutation::admit(mutations.as_deref(), &command)?;
                Ok::<_, wire::CommandFailure>((guards, command))
            })
            .await
            .map_err(|error| {
                wire::CommandFailure::from(
                    crate::adaptor::presenter::error::AppError::from_failure(
                        crate::domain::failure::TechnicalFailure::from(error),
                    ),
                )
            })??;
            match handler {
                Some(handler) => crate::common::operation_context::wait(
                    &crate::common::operation_context::current(),
                    async { super::worktree_mutation::scope(guards, handler(command)).await },
                )
                .await
                .map_err(|error| {
                    wire::CommandFailure::from(
                        crate::adaptor::presenter::error::AppError::from_failure(
                            crate::domain::failure::TechnicalFailure::from(error),
                        ),
                    )
                })?,
                None => Err(crate::adaptor::presenter::error::AppError::missing_target(
                    "Command was not found",
                )
                .with_code("UNKNOWN_COMMAND")
                .into()),
            }
        })
    }
}

pub fn invalid_request(message: impl Into<String>) -> wire::CommandFailure {
    crate::adaptor::presenter::error::AppError::invalid_request(message)
        .with_code("INVALID_REQUEST")
        .into()
}
pub fn required<T>(value: Option<T>, field: &str) -> Result<T, wire::CommandFailure> {
    value.ok_or_else(|| invalid_request(format!("Missing {field}")))
}
pub(crate) fn convert<T, U: TryFrom<T>>(value: T) -> Result<U, wire::CommandFailure>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| invalid_request(error.to_string()))
}
pub(crate) fn optional<T, U: TryFrom<T>>(
    value: Option<T>,
) -> Result<Option<U>, wire::CommandFailure>
where
    U::Error: std::fmt::Display,
{
    value.map(convert).transpose()
}

#[cfg(test)]
#[path = "dispatch_test.rs"]
mod dispatch_tests;
