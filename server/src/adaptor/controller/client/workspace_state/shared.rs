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
        let store = deps.workspace_state_store.clone();
        let publisher = router.publisher.clone();
        router.register_domain(
            &["save_repository_group_state"],
            Box::new(move |command| {
                let store = store.clone();
                let publisher = publisher.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SaveRepositoryGroupState(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let store =
                        store.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    crate::usecase::workspace_state::usecase::save_repository_group_state(
                        store.as_ref(),
                        publisher.as_ref(),
                        &args.repository_path,
                        args.collapsed,
                    )
                    .map_err(crate::adaptor::presenter::error::AppError::from_failure)?;
                    Ok(wire::command_result::Command::SaveRepositoryGroupState(
                        wire::Unit {},
                    ))
                })
            }),
        );
    }
    {
        let store = deps.workspace_state_store.clone();
        let publisher = router.publisher.clone();
        router.register_domain(
            &["save_workspace_state"],
            Box::new(move |command| {
                let store = store.clone();
                let publisher = publisher.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SaveWorkspaceState(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let store = store
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::save_workspace_state_shared(
                            &store,
                            publisher.as_ref(),
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
