use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome, value};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let journal = deps.caller_attempt_journal.clone();
        router.register_domain(
            &["acknowledge_application_attempt"],
            Box::new(move |command| {
                let journal = journal.clone();
                Box::pin(async move {
                    let wire::command_request::Command::AcknowledgeApplicationAttempt(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let journal = journal
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            acknowledge_application_attempt_shared(
                                &journal,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::AcknowledgeApplicationAttempt(result))
                })
            }),
        );
    }
    {
        let coordinator = deps.shutdown_coordinator.clone();
        router.register_domain(
            &["compact_application_shutdown_details"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                Box::pin(async move {
                    let wire::command_request::Command::CompactApplicationShutdownDetails(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            compact_application_shutdown_details_shared(
                                &coordinator,
                                convert(required(args.shutdown_id, "shutdownId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::CompactApplicationShutdownDetails(result))
                })
            }),
        );
    }
    {
        let coordinator = deps.shutdown_coordinator.clone();
        router.register_domain(
            &["get_application_quit_operation"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetApplicationQuitOperation(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            get_application_quit_operation_shared(
                                &coordinator,
                                convert(required(args.operation_id, "operationId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetApplicationQuitOperation(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let coordinator = deps.shutdown_coordinator.clone();
        router.register_domain(
            &["get_application_shutdown"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetApplicationShutdown(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(get_application_shutdown_shared(&coordinator).await)
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetApplicationShutdown(
                        result,
                    ))
                })
            }),
        );
    }
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
        let coordinator = deps.shutdown_coordinator.clone();
        router.register_domain(
            &["get_shutdown_plan"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetShutdownPlan(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            get_shutdown_plan_shared(
                                &coordinator,
                                convert(required(args.shutdown_id, "shutdownId")?)?,
                                optional(args.limit)?,
                                optional(args.cursor)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetShutdownPlan(result))
                })
            }),
        );
    }
    {
        let journal = deps.caller_attempt_journal.clone();
        router.register_domain(
            &["list_pending_application_attempts"],
            Box::new(move |command| {
                let journal = journal.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListPendingApplicationAttempts(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let journal = journal
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            list_pending_application_attempts_shared(
                                &journal,
                                optional(args.limit)?,
                                optional(args.cursor)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListPendingApplicationAttempts(result))
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
        let coordinator = deps.shutdown_coordinator.clone();
        let process_actions = deps.application_process_action_dispatcher.clone();
        let process_port = deps.process_port.clone();
        router.register_domain(
            &["request_application_quit"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                let process_actions = process_actions.clone();
                let process_port = process_port.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RequestApplicationQuit(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let process_actions = process_actions
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            request_application_quit_shared(
                                process_port.as_ref(),
                                &coordinator,
                                &process_actions,
                                convert(required(args.request, "request")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RequestApplicationQuit(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let coordinator = deps.shutdown_coordinator.clone();
        let process_actions = deps.application_process_action_dispatcher.clone();
        let process_port = deps.process_port.clone();
        router.register_domain(
            &["resolve_shutdown_target_action"],
            Box::new(move |command| {
                let coordinator = coordinator.clone();
                let process_actions = process_actions.clone();
                let process_port = process_port.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResolveShutdownTargetAction(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let coordinator = coordinator
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let process_actions = process_actions
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            resolve_shutdown_target_action_shared(
                                process_port.as_ref(),
                                &coordinator,
                                &process_actions,
                                convert(required(args.request, "request")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResolveShutdownTargetAction(
                        result,
                    ))
                })
            }),
        );
    }
}
