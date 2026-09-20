use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::client::{invalid_request, outcome, value};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let authority = deps.application_startup_authority.clone();
        router.register_domain(
            &["get_application_startup_outcome"],
            Box::new(move |command| {
                let authority = authority.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetApplicationStartupOutcome(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let authority = authority
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        value(get_application_startup_outcome_shared(&authority))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetApplicationStartupOutcome(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let authority = deps.application_startup_authority.clone();
        router.register_domain(
            &["quit_after_startup_failure"],
            Box::new(move |command| {
                let authority = authority.clone();
                Box::pin(async move {
                    let wire::command_request::Command::QuitAfterStartupFailure(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let authority = authority
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(quit_after_startup_failure_shared(&authority))
                    }
                    .await?;
                    Ok(wire::command_result::Command::QuitAfterStartupFailure(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let process_port = deps.process_port.clone();
        router.register_domain(
            &["request_application_quit"],
            Box::new(move |command| {
                let process_port = process_port.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RequestApplicationQuit(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        outcome(request_application_quit_shared(
                            process_port.as_ref(),
                            convert(required(args.request, "request")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::RequestApplicationQuit(
                        result,
                    ))
                })
            }),
        );
    }
}
