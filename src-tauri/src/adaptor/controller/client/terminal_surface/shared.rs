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
        let state = deps.app_state.clone();
        router.register_domain(
            &["ack_terminal_surface_output"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::AckTerminalSurfaceOutput(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        commands::ack_terminal_surface_output_shared(
                            &state,
                            convert(required(args.attachment_id, "attachmentId")?)?,
                            convert(required(args.sequence, "sequence")?)?,
                        );
                        value(())
                    }
                    .await?;
                    Ok(wire::command_result::Command::AckTerminalSurfaceOutput(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["detach_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DetachTerminalSurface(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        commands::detach_terminal_surface_shared(
                            &state,
                            convert(required(args.attachment_id, "attachmentId")?)?,
                        );
                        value(())
                    }
                    .await?;
                    Ok(wire::command_result::Command::DetachTerminalSurface(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_or_spawn_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetOrSpawnTerminalSurface(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_or_spawn_terminal_surface_shared(
                            &state,
                            convert(required(args.rows, "rows")?)?,
                            convert(required(args.cols, "cols")?)?,
                            optional(args.cwd)?,
                            convert(required(args.owner, "owner")?)?,
                            optional(args.label)?,
                            optional(args.startup_command)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetOrSpawnTerminalSurface(
                        result,
                    ))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["get_performance_real_app_mode"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::GetPerformanceRealAppMode(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result =
                        async move { value(commands::get_performance_real_app_mode_shared()) }
                            .await?;
                    Ok(wire::command_result::Command::GetPerformanceRealAppMode(
                        result,
                    ))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["get_terminal_performance_switches"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::GetTerminalPerformanceSwitches(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result =
                        async move { value(commands::get_terminal_performance_switches_shared()) }
                            .await?;
                    Ok(wire::command_result::Command::GetTerminalPerformanceSwitches(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetTerminalSurface(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_terminal_surface_shared(
                            &state,
                            convert(required(args.owner, "owner")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetTerminalSurface(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["kill_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::KillTerminalSurface(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::kill_terminal_surface_shared(
                            &state,
                            convert(required(args.owner, "owner")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::KillTerminalSurface(result))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["record_terminal_launch_renderer_phase"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::RecordTerminalLaunchRendererPhase(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        outcome(commands::record_terminal_launch_renderer_phase_shared(
                            convert(required(args.phase, "phase")?)?,
                            crate::adaptor::controller::client::finite(required(
                                args.duration_ms,
                                "durationMs",
                            )?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::RecordTerminalLaunchRendererPhase(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["resize_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResizeTerminalSurface(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::resize_terminal_surface_shared(
                            &state,
                            convert(required(args.owner, "owner")?)?,
                            convert(required(args.rows, "rows")?)?,
                            convert(required(args.cols, "cols")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResizeTerminalSurface(result))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["start_terminal_input_performance_collection"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::StartTerminalInputPerformanceCollection(
                        _args,
                    ) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        commands::start_terminal_input_performance_collection_shared();
                        value(())
                    }
                    .await?;
                    Ok(
                        wire::command_result::Command::StartTerminalInputPerformanceCollection(
                            result,
                        ),
                    )
                })
            }),
        );
    }
    {
        router.register_domain(
            &["start_terminal_launch_performance_collection"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::StartTerminalLaunchPerformanceCollection(
                        _args,
                    ) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        commands::start_terminal_launch_performance_collection_shared();
                        value(())
                    }
                    .await?;
                    Ok(
                        wire::command_result::Command::StartTerminalLaunchPerformanceCollection(
                            result,
                        ),
                    )
                })
            }),
        );
    }
    {
        router.register_domain(
            &["take_terminal_input_performance_samples"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::TakeTerminalInputPerformanceSamples(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        value(commands::take_terminal_input_performance_samples_shared())
                    }
                    .await?;
                    Ok(wire::command_result::Command::TakeTerminalInputPerformanceSamples(result))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["take_terminal_launch_performance_samples"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::TakeTerminalLaunchPerformanceSamples(_args) = command else { return Err(invalid_request("Mismatched command")); };
                    let result = async move {
                    value(commands::take_terminal_launch_performance_samples_shared())
                    }.await?;
                    Ok(wire::command_result::Command::TakeTerminalLaunchPerformanceSamples(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["write_paths_to_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WritePathsToTerminalSurface(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::write_paths_to_terminal_surface_shared(
                            &state,
                            convert(required(args.owner, "owner")?)?,
                            convert(required(args.paths, "paths")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::WritePathsToTerminalSurface(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["write_terminal_surface"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WriteTerminalSurface(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::write_terminal_surface_shared(
                            &state,
                            convert(required(args.owner, "owner")?)?,
                            convert(required(args.attachment_id, "attachmentId")?)?,
                            convert(required(args.sequence, "sequence")?)?,
                            args.client_started_at_unix_ms
                                .map(crate::adaptor::controller::client::finite)
                                .transpose()?,
                            convert(required(args.data, "data")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::WriteTerminalSurface(result))
                })
            }),
        );
    }
}
