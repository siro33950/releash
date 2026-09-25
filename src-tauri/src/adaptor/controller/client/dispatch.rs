use crate::adaptor::controller::api::protocol::client as wire;
use crate::usecase::application_startup::ApplicationStartupAuthority;
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

#[cfg(test)]
#[path = "dispatch_test.rs"]
mod tests;

pub(crate) struct ClientCommandDispatch {
    handlers: HashMap<&'static str, Arc<CommandHandler>>,
    authority: Arc<ApplicationStartupAuthority>,
    pub(super) publisher: Option<crate::usecase::state_subscription::StateSubscriptionPublisher>,
    mutations: Option<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
}

impl ClientCommandDispatch {
    pub(crate) fn new(authority: Arc<ApplicationStartupAuthority>) -> Self {
        Self {
            handlers: HashMap::new(),
            mutations: None,
            authority,
            publisher: None,
        }
    }

    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) -> Self {
        self.publisher = Some(publisher);
        self
    }

    pub(crate) fn register_dependencies(&mut self, deps: &super::ClientDependencies) {
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
    }
    #[cfg(test)]
    pub(crate) fn with_worktree_mutations(
        mut self,
        runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    ) -> Self {
        self.mutations = Some(runtime);
        self
    }
    pub(crate) fn register_domain(
        &mut self,
        names: &'static [&'static str],
        handler: CommandHandler,
    ) {
        assert_eq!(names.len(), 1);
        assert!(self.handlers.insert(names[0], Arc::new(handler)).is_none());
    }
    #[cfg(test)]
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }
    pub(crate) fn admit(&self, command: &str) -> Result<(), wire::CommandFailure> {
        if !command_admitted(command, Some(&self.authority)) {
            return Err(crate::adaptor::presenter::error::AppError::coded(
                "APPLICATION_UNAVAILABLE",
                "Application is unavailable",
                crate::domain::failure::FailureKind::StateRequired,
            )
            .into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn dispatch(
        &self,
        command: wire::command_request::Command,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<wire::command_result::Command, wire::CommandFailure>> + Send,
        >,
    > {
        if let Err(error) = self.admit(command.name()) {
            return Box::pin(std::future::ready(Err(error)));
        }
        self.dispatch_admitted(command)
    }
    pub(crate) fn dispatch_admitted(
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
                wire::CommandFailure::from(crate::adaptor::presenter::error::AppError::new(
                    error.to_string(),
                ))
            })??;
            match handler {
                Some(handler) => crate::common::operation_context::wait(
                    &crate::common::operation_context::current(),
                    async { super::worktree_mutation::scope(guards, handler(command)).await },
                )
                .await
                .map_err(|error| {
                    wire::CommandFailure::from(
                        crate::adaptor::presenter::error::AppError::from_failure(error),
                    )
                })?,
                None => Err(crate::adaptor::presenter::error::AppError::coded(
                    "UNKNOWN_COMMAND",
                    "Command was not found",
                    crate::domain::failure::FailureKind::Missing,
                )
                .into()),
            }
        })
    }
}

pub(crate) fn invalid_request(message: impl Into<String>) -> wire::CommandFailure {
    crate::adaptor::presenter::error::AppError::coded(
        "INVALID_REQUEST",
        message,
        crate::domain::failure::FailureKind::InvalidInput,
    )
    .into()
}
pub(crate) fn required<T>(value: Option<T>, field: &str) -> Result<T, wire::CommandFailure> {
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
pub(crate) fn value<T, U: TryFrom<T>>(value: T) -> Result<U, wire::CommandFailure>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| {
        crate::adaptor::presenter::error::AppError::coded(
            "INVALID_RESPONSE",
            error.to_string(),
            crate::domain::failure::FailureKind::Internal,
        )
        .into()
    })
}
pub(crate) fn outcome<T, U: TryFrom<T>, E: Into<wire::CommandFailure>>(
    result: Result<T, E>,
) -> Result<U, wire::CommandFailure>
where
    U::Error: std::fmt::Display,
{
    result.map_err(Into::into).and_then(value)
}

pub(crate) const STARTUP_COMMANDS: [&str; 2] = [
    "get_application_startup_outcome",
    "quit_after_startup_failure",
];

pub(crate) fn command_admitted(
    command: &str,
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
) -> bool {
    authority.is_some_and(|authority| {
        STARTUP_COMMANDS.contains(&command) || authority.normal_admission_ready()
    })
}

pub(crate) fn finite(value: f64) -> Result<f64, wire::CommandFailure> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid_request("Expected finite number"))
    }
}
