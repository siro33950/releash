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
        let store = deps.workspace_state_store.clone();
        router.register_domain(
            &["load_workspace_state"],
            Box::new(move |command| {
                let store = store.clone();
                Box::pin(async move {
                    let wire::command_request::Command::LoadWorkspaceState(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let store = store
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        value(commands::load_workspace_state_shared(
                            &store,
                            convert(required(args.worktree_name, "worktreeName")?)?,
                            convert(required(args.worktree_root, "worktreeRoot")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::LoadWorkspaceState(result))
                })
            }),
        );
    }
    {
        let store = deps.workspace_state_store.clone();
        router.register_domain(
            &["save_workspace_state"],
            Box::new(move |command| {
                let store = store.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SaveWorkspaceState(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let store = store
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::save_workspace_state_shared(
                            &store,
                            convert(required(args.worktree_name, "worktreeName")?)?,
                            convert(required(args.state, "state")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::SaveWorkspaceState(result))
                })
            }),
        );
    }
}
