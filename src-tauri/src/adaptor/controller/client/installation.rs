use super::{invalid_request, ClientCommandDispatch, ClientDependencies};
use crate::adaptor::presenter::{client as wire, error::AppError, installation as present};
use std::path::Path;
pub(crate) fn register_shared(router: &mut ClientCommandDispatch, deps: &ClientDependencies) {
    let Some(usecase) = &deps.installation_usecase else {
        return;
    };
    let install = usecase.clone();
    router.register_domain(
        &["install_cli"],
        Box::new(move |command| {
            let usecase = install.clone();
            Box::pin(async move {
                let wire::command_request::Command::InstallCli(_) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let result = crate::common::operation_context::spawn_blocking(move || {
                    usecase.install_cli(Path::new("/usr/local/bin/releash"))
                })
                .await
                .map_err(|error| {
                    AppError::from_failure(crate::domain::failure::TechnicalFailure::from(error))
                })?
                .map_err(present::failure)?;
                Ok(wire::command_result::Command::InstallCli(
                    present::installed(result),
                ))
            })
        }),
    );
    let check = usecase.clone();
    router.register_domain(
        &["check_login_registration"],
        Box::new(move |command| {
            let usecase = check.clone();
            Box::pin(async move {
                let wire::command_request::Command::CheckLoginRegistration(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                if !Path::new(&args.executable_path).is_absolute() {
                    return Err(invalid_request("executable_path must be absolute"));
                }
                let result = crate::common::operation_context::spawn_blocking(move || {
                    usecase.login_registration(Path::new(&args.executable_path))
                })
                .await
                .map_err(|error| {
                    AppError::from_failure(crate::domain::failure::TechnicalFailure::from(error))
                })?
                .map_err(present::failure)?;
                Ok(wire::command_result::Command::CheckLoginRegistration(
                    present::login_registration(result),
                ))
            })
        }),
    );
}

#[cfg(test)]
#[path = "installation_test.rs"]
mod installation_tests;
