use crate::adaptor::controller::api::protocol::client as wire;
use crate::usecase::{
    application_startup::ApplicationStartupAuthority, repository_usecase::RepositoryUsecase,
};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

pub(crate) type CommandHandler = Box<
    dyn Fn(
            wire::command_request::Command,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<wire::command_result::Command, wire::CommandError>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

pub(crate) struct ClientCommandDispatch {
    handlers: HashMap<&'static str, CommandHandler>,
    authority: Arc<ApplicationStartupAuthority>,
}

impl ClientCommandDispatch {
    pub(crate) fn new(
        repository: Arc<RepositoryUsecase>,
        authority: Arc<ApplicationStartupAuthority>,
    ) -> Self {
        let mut dispatch = Self {
            handlers: HashMap::new(),
            authority,
        };
        dispatch.register_domain(
            &["get_current_branch"],
            Box::new(move |command| {
                let repository = repository.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetCurrentBranch(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let path = required(args.repo_path, "repoPath")?;
                    let result = super::repository::run_blocking(move || {
                        repository.get_current_branch(&path)
                    })
                    .await;
                    outcome(result).map(wire::command_result::Command::GetCurrentBranch)
                })
            }),
        );
        dispatch
    }

    pub(crate) fn register_dependencies(&mut self, deps: &super::ClientDependencies) {
        super::repository::register_shared(self, deps);
        super::code::register_shared(self, deps);
        super::comment::register_shared(self, deps);
        super::agent_session::register_shared(self, deps);
        super::workflow::register_shared(self, deps);
    }
    pub(crate) fn register_domain(
        &mut self,
        names: &'static [&'static str],
        handler: CommandHandler,
    ) {
        assert_eq!(names.len(), 1);
        assert!(self.handlers.insert(names[0], handler).is_none());
    }
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }
    pub(crate) fn admit(&self, command: &str) -> Result<(), wire::CommandError> {
        if !command_admitted(command, Some(&self.authority)) {
            return Err(crate::other::AppError::coded(
                "APPLICATION_UNAVAILABLE",
                "Application is unavailable",
            )
            .into());
        }
        Ok(())
    }
    pub(crate) async fn dispatch(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, wire::CommandError> {
        self.admit(command.name())?;
        self.dispatch_admitted(command).await
    }
    pub(crate) async fn dispatch_admitted(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, wire::CommandError> {
        let handler = self.handlers.get(command.name()).ok_or_else(|| {
            wire::CommandError::from(crate::other::AppError::coded(
                "UNKNOWN_COMMAND",
                "Command was not found",
            ))
        })?;
        handler(command).await
    }
}

pub(crate) fn invalid_request(message: impl Into<String>) -> wire::CommandError {
    crate::other::AppError::coded("INVALID_REQUEST", message).into()
}
pub(crate) fn required<T>(value: Option<T>, field: &str) -> Result<T, wire::CommandError> {
    value.ok_or_else(|| invalid_request(format!("Missing {field}")))
}
pub(crate) fn convert<T, U: TryFrom<T>>(value: T) -> Result<U, wire::CommandError>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| invalid_request(error.to_string()))
}
pub(crate) fn optional<T, U: TryFrom<T>>(value: Option<T>) -> Result<Option<U>, wire::CommandError>
where
    U::Error: std::fmt::Display,
{
    value.map(convert).transpose()
}
pub(crate) fn value<T, U: TryFrom<T>>(value: T) -> Result<U, wire::CommandError>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| {
        crate::other::AppError::coded("INVALID_RESPONSE", error.to_string()).into()
    })
}
pub(crate) fn outcome<T, U: TryFrom<T>, E: Into<wire::CommandError>>(
    result: Result<T, E>,
) -> Result<U, wire::CommandError>
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
