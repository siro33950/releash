use super::commands;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome, ClientCommandDispatch};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        router.register_domain(
            &["list_review_threads"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListReviewThreads(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::list_review_threads_shared(
                                data_dir,
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                optional(args.filter)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListReviewThreads(result))
                })
            }),
        );
    }
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        let notify = deps.comment_notify.clone();
        router.register_domain(
            &["create_review_thread"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                let notify = notify.clone();
                Box::pin(async move {
                    let wire::command_request::Command::CreateReviewThread(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::create_review_thread_shared(
                                data_dir,
                                notify.as_ref(),
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                crate::domain::comment::ReviewTarget {
                                    file_path: optional(args.file_path)?,
                                    line_number: optional(args.line_number)?,
                                    end_line: optional(args.end_line)?,
                                },
                                convert(required(args.content, "content")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::CreateReviewThread(result))
                })
            }),
        );
    }
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        let notify = deps.comment_notify.clone();
        router.register_domain(
            &["append_review_comment"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                let notify = notify.clone();
                Box::pin(async move {
                    let wire::command_request::Command::AppendReviewComment(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::append_review_comment_shared(
                                data_dir,
                                notify.as_ref(),
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                convert(required(args.thread_id, "threadId")?)?,
                                convert(required(args.content, "content")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::AppendReviewComment(result))
                })
            }),
        );
    }
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        let notify = deps.comment_notify.clone();
        router.register_domain(
            &["resolve_review_thread"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                let notify = notify.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResolveReviewThread(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::resolve_review_thread_shared(
                                data_dir,
                                notify.as_ref(),
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                convert(required(args.thread_id, "threadId")?)?,
                                convert(required(args.outcome, "outcome")?)?,
                                convert(required(args.summary, "summary")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResolveReviewThread(result))
                })
            }),
        );
    }
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        let notify = deps.comment_notify.clone();
        router.register_domain(
            &["delete_review_thread"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                let notify = notify.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DeleteReviewThread(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::delete_review_thread_shared(
                                data_dir,
                                notify.as_ref(),
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                convert(required(args.thread_id, "threadId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DeleteReviewThread(result))
                })
            }),
        );
    }
    {
        let data_dir = deps.data_dir.clone();
        let usecase = deps.review_comment_usecase.clone();
        router.register_domain(
            &["build_review_thread_handoff"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                let data_dir = data_dir.clone();
                Box::pin(async move {
                    let wire::command_request::Command::BuildReviewThreadHandoff(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let data_dir = data_dir.map_err(wire::CommandError::from)?;
                        outcome(
                            commands::build_review_thread_handoff_shared(
                                data_dir,
                                &usecase,
                                convert(required(args.worktree_name, "worktreeName")?)?,
                                convert(required(args.thread_id, "threadId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::BuildReviewThreadHandoff(
                        result,
                    ))
                })
            }),
        );
    }
}
