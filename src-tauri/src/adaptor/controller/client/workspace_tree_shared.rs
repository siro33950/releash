use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::client::{invalid_request, outcome};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let app_state = deps.app_state.clone();
        router.register_domain(
            &["refresh_workspaces"],
            Box::new(move |command| {
                let app_state = app_state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RefreshWorkspaces(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let app_state = app_state
                        .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    match args.worktree_path.as_deref() {
                        Some(path) => app_state.workspace_list.refresh_worktree(path).await,
                        None => match args.repo_path.as_deref() {
                            Some(path) => app_state.workspace_list.refresh_repository(path).await,
                            None => app_state.workspace_list.refresh().await,
                        },
                    };
                    outcome(Ok::<_, crate::other::AppError>(()))
                        .map(wire::command_result::Command::RefreshWorkspaces)
                })
            }),
        );
    }
    {
        let usecase = deps.workspace_node_command_usecase.clone();
        router.register_domain(
            &["approve_workspace_node"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ApproveWorkspaceNode(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            approve_workspace_node_shared(
                                &usecase,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.node_id, "nodeId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ApproveWorkspaceNode(result))
                })
            }),
        );
    }
    {
        let app_state = deps.app_state.clone();
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["archive_workspace_workflow_execution"],
            Box::new(move |command| {
                let app_state = app_state.clone();
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ArchiveWorkspaceWorkflowExecution(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let app_state = app_state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            archive_workspace_workflow_execution_shared(
                                &app_state,
                                &runtime,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ArchiveWorkspaceWorkflowExecution(result))
                })
            }),
        );
    }
    {
        let usecase = deps.workspace_node_command_usecase.clone();
        router.register_domain(
            &["rename_workspace_session_node"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RenameWorkspaceSessionNode(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            rename_workspace_session_node_shared(
                                &usecase,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.node_id, "nodeId")?)?,
                                convert(required(args.name, "name")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RenameWorkspaceSessionNode(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let app_state = deps.app_state.clone();
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["restore_workspace_workflow_execution"],
            Box::new(move |command| {
                let app_state = app_state.clone();
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RestoreWorkspaceWorkflowExecution(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let app_state = app_state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            restore_workspace_workflow_execution_shared(
                                &app_state,
                                &runtime,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RestoreWorkspaceWorkflowExecution(result))
                })
            }),
        );
    }
    {
        let usecase = deps.workspace_node_command_usecase.clone();
        router.register_domain(
            &["retry_workspace_node"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RetryWorkspaceNode(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            retry_workspace_node_shared(
                                &usecase,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.node_id, "nodeId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RetryWorkspaceNode(result))
                })
            }),
        );
    }
    {
        let usecase = deps.workspace_node_command_usecase.clone();
        router.register_domain(
            &["resume_workspace_session_node"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResumeWorkspaceSessionNode(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let usecase = usecase
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            resume_workspace_session_node_shared(
                                &usecase,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.node_id, "nodeId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResumeWorkspaceSessionNode(
                        result,
                    ))
                })
            }),
        );
    }
}

#[cfg(all(test, feature = "desktop"))]
#[path = "workspace_tree_shared_test.rs"]
mod workspace_tree_shared_tests;
