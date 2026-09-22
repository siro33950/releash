use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let lifecycle = deps.agent_session_lifecycle_usecase.clone();
        router.register_domain(
            &["archive_agent_session"],
            Box::new(move |command| {
                let lifecycle = lifecycle.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ArchiveAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let lifecycle = lifecycle
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::archive_agent_session_shared(
                                &lifecycle,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ArchiveAgentSession(result))
                })
            }),
        );
    }
    {
        let launch = deps.agent_session_launch_usecase.clone();
        router.register_domain(
            &["create_agent_session"],
            Box::new(move |command| {
                let launch = launch.clone();
                Box::pin(async move {
                    let wire::command_request::Command::CreateAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let launch = launch
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::create_agent_session_shared(
                                &launch,
                                convert(required(args.workspace_identity, "workspaceIdentity")?)?,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.provider, "provider")?)?,
                                convert(required(args.rows, "rows")?)?,
                                convert(required(args.cols, "cols")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::CreateAgentSession(result))
                })
            }),
        );
    }
    {
        let lifecycle = deps.agent_session_lifecycle_usecase.clone();
        router.register_domain(
            &["delete_agent_session"],
            Box::new(move |command| {
                let lifecycle = lifecycle.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DeleteAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let lifecycle = lifecycle
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::delete_agent_session_shared(
                                &lifecycle,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DeleteAgentSession(result))
                })
            }),
        );
    }
    {
        let read = deps.agent_session_read_usecase.clone();
        router.register_domain(
            &["get_agent_session"],
            Box::new(move |command| {
                let read = read.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let read =
                            read.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::get_agent_session_shared(
                                &read,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetAgentSession(result))
                })
            }),
        );
    }
    {
        let availability = deps.provider_availability_usecase.clone();
        router.register_domain(
            &["get_provider_availability"],
            Box::new(move |command| {
                let availability = availability.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetProviderAvailability(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let availability = availability
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(provider_tui::get_provider_availability_shared(
                            &availability,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetProviderAvailability(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let query = deps.agent_session_history_read_usecase.clone();
        router.register_domain(
            &["list_agent_session_history"],
            Box::new(move |command| {
                let query = query.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListAgentSessionHistory(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let query = query
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::list_agent_session_history_shared(
                                &query,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                optional(args.limit)?,
                                optional(args.after)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListAgentSessionHistory(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let availability = deps.provider_availability_usecase.clone();
        router.register_domain(
            &["list_available_agent_session_providers"],
            Box::new(move |command| {
                let availability = availability.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListAvailableAgentSessionProviders(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let availability = availability
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(provider_tui::list_available_agent_session_providers_shared(
                            &availability,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListAvailableAgentSessionProviders(result))
                })
            }),
        );
    }
    {
        let query = deps.provider_hook_health_read_usecase.clone();
        router.register_domain(
            &["list_provider_hook_health_warnings"],
            Box::new(move |command| {
                let query = query.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListProviderHookHealthWarnings(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let query = query
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::list_provider_hook_health_warnings_shared(&query).await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListProviderHookHealthWarnings(result))
                })
            }),
        );
    }
    {
        let lifecycle = deps.agent_session_lifecycle_usecase.clone();
        router.register_domain(
            &["open_agent_session"],
            Box::new(move |command| {
                let lifecycle = lifecycle.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let lifecycle = lifecycle
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::open_agent_session_shared(
                                &lifecycle,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                                convert(required(args.rows, "rows")?)?,
                                convert(required(args.cols, "cols")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenAgentSession(result))
                })
            }),
        );
    }
    {
        let availability = deps.provider_availability_usecase.clone();
        router.register_domain(
            &["refresh_provider_availability"],
            Box::new(move |command| {
                let availability = availability.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RefreshProviderAvailability(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let availability = availability
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::refresh_provider_availability_shared(&availability).await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RefreshProviderAvailability(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let availability = deps.provider_availability_usecase.clone();
        router.register_domain(
            &["reset_provider_executable"],
            Box::new(move |command| {
                let availability = availability.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResetProviderExecutable(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let availability = availability
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::reset_provider_executable_shared(
                                &availability,
                                convert(required(args.provider, "provider")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResetProviderExecutable(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let lifecycle = deps.agent_session_lifecycle_usecase.clone();
        router.register_domain(
            &["restore_agent_session"],
            Box::new(move |command| {
                let lifecycle = lifecycle.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RestoreAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let lifecycle = lifecycle
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::restore_agent_session_shared(
                                &lifecycle,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                                convert(required(args.rows, "rows")?)?,
                                convert(required(args.cols, "cols")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RestoreAgentSession(result))
                })
            }),
        );
    }
    {
        let lifecycle = deps.agent_session_lifecycle_usecase.clone();
        router.register_domain(
            &["resume_agent_session"],
            Box::new(move |command| {
                let lifecycle = lifecycle.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResumeAgentSession(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let lifecycle = lifecycle
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::resume_agent_session_shared(
                                &lifecycle,
                                convert(required(args.agent_session_id, "agentSessionId")?)?,
                                convert(required(args.rows, "rows")?)?,
                                convert(required(args.cols, "cols")?)?,
                                convert(required(args.caller_request_id, "callerRequestId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResumeAgentSession(result))
                })
            }),
        );
    }
    {
        let launch = deps.agent_session_launch_usecase.clone();
        router.register_domain(
            &["resume_agent_session_history_candidate"],
            Box::new(move |command| {
                let launch = launch.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResumeAgentSessionHistoryCandidate(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let launch = launch
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::resume_agent_session_history_candidate_shared(
                                &launch, args,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResumeAgentSessionHistoryCandidate(result))
                })
            }),
        );
    }
    {
        let availability = deps.provider_availability_usecase.clone();
        router.register_domain(
            &["update_provider_executable"],
            Box::new(move |command| {
                let availability = availability.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateProviderExecutable(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let availability = availability
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            provider_tui::update_provider_executable_shared(
                                &availability,
                                convert(required(args.provider, "provider")?)?,
                                convert(required(args.executable, "executable")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdateProviderExecutable(
                        result,
                    ))
                })
            }),
        );
    }
}
