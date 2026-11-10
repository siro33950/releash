use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::client::{invalid_request, outcome};
use crate::adaptor::presenter::client as wire;

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["git_stage"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GitStage(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            staging::git_stage_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.paths, "paths")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GitStage(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["git_unstage"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GitUnstage(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            staging::git_unstage_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.paths, "paths")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GitUnstage(result))
                })
            }),
        );
    }
}
