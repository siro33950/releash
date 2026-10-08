use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome};
use crate::adaptor::presenter::client as wire;

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
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
        let state = deps.app_state.clone();
        router.register_domain(
            &["resize_terminal_surface"],
            Box::new(move |command| {
                let resize = (|| {
                    let wire::command_request::Command::ResizeTerminalSurface(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let state = state
                        .as_ref()
                        .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    Ok(commands::resize_terminal_surface_shared(
                        state,
                        convert(required(args.owner, "owner")?)?,
                        convert(required(args.rows, "rows")?)?,
                        convert(required(args.cols, "cols")?)?,
                    ))
                })();
                Box::pin(async move {
                    let result = outcome(resize?.await)?;
                    Ok(wire::command_result::Command::ResizeTerminalSurface(result))
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
