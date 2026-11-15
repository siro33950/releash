use crate::adaptor::controller::client::{
    invalid_request, outcome, ClientCommandDispatch, ClientDependencies,
};
use crate::adaptor::presenter::client as wire;

pub(super) fn register(router: &mut ClientCommandDispatch, deps: &ClientDependencies) {
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_commit"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitCommit(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let follow_up = wire::GitCommitFollowUp::try_from(args.follow_up)
                    .map_err(|_| invalid_request("Invalid commit follow-up"))?;
                if follow_up == wire::GitCommitFollowUp::Unspecified {
                    return Err(invalid_request("Commit follow-up is required"));
                }
                let result = super::run_blocking(move || match follow_up {
                    wire::GitCommitFollowUp::Push => {
                        usecase.commit_and_push(&args.worktree_path, &args.message)
                    }
                    wire::GitCommitFollowUp::Sync => {
                        usecase.commit_and_sync(&args.worktree_path, &args.message)
                    }
                    _ => usecase.commit(&args.worktree_path, &args.message),
                })
                .await;
                Ok(R::GitCommit(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_push"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitPush(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result = super::run_blocking(move || {
                    usecase.push(&args.worktree_path, args.force_with_lease)
                })
                .await;
                Ok(R::GitPush(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_pull"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitPull(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result = super::run_blocking(move || usecase.pull(&args.worktree_path)).await;
                Ok(R::GitPull(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_sync"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitSync(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result = super::run_blocking(move || usecase.sync(&args.worktree_path)).await;
                Ok(R::GitSync(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_fetch"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitFetch(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result = super::run_blocking(move || usecase.fetch(&args.worktree_path)).await;
                Ok(R::GitFetch(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_discard"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitDiscard(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result =
                    super::run_blocking(move || usecase.discard(&args.worktree_path, &args.paths))
                        .await;
                Ok(R::GitDiscard(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_continue_operation"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitContinueOperation(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result = super::run_blocking(move || {
                    usecase.continue_operation(&args.worktree_path, &args.message)
                })
                .await;
                Ok(R::GitContinueOperation(outcome(result)?))
            })
        }),
    );
    let state = deps.app_state.clone();
    router.register_domain(
        &["git_abort_operation"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                use wire::command_request::Command as C;
                use wire::command_result::Command as R;
                let C::GitAbortOperation(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase = state
                    .ok_or_else(|| invalid_request("Command dependency unavailable"))?
                    .git_usecase
                    .clone();

                let result =
                    super::run_blocking(move || usecase.abort_operation(&args.worktree_path)).await;
                Ok(R::GitAbortOperation(outcome(result)?))
            })
        }),
    );
}
