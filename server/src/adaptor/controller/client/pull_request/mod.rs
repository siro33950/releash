use super::{invalid_request, outcome, ClientCommandDispatch, ClientDependencies};
use crate::adaptor::presenter::{client as wire, error::AppError};

pub(crate) fn register_shared(router: &mut ClientCommandDispatch, deps: &ClientDependencies) {
    let usecase = deps
        .app_state
        .as_ref()
        .map(|state| state.pull_request_usecase.clone());
    let create = usecase.clone();
    router.register_domain(
        &["create_pull_request"],
        Box::new(move |command| {
            let usecase = create.clone();
            Box::pin(async move {
                let wire::command_request::Command::CreatePullRequest(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase =
                    usecase.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                let result = outcome(
                    usecase
                        .create(
                            &args.worktree_path,
                            args.title,
                            args.body,
                            args.base,
                            args.draft,
                        )
                        .await
                        .map_err(AppError::from_failure),
                )?;
                Ok(wire::command_result::Command::CreatePullRequest(result))
            })
        }),
    );
    router.register_domain(
        &["post_pull_request_comment"],
        Box::new(move |command| {
            let usecase = usecase.clone();
            Box::pin(async move {
                let wire::command_request::Command::PostPullRequestComment(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let usecase =
                    usecase.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                let result = outcome(
                    usecase
                        .post_comment(&args.worktree_path, args.number, args.body)
                        .await
                        .map_err(AppError::from_failure),
                )?;
                Ok(wire::command_result::Command::PostPullRequestComment(
                    result,
                ))
            })
        }),
    );
}
