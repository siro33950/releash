use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome};
use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::client::value;

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let state = deps.app_state.clone();
        let sessions = deps.agent_session_launch_usecase.clone();
        let workflows = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["create_worktrees"],
            Box::new(move |command| {
                let state = state.clone();
                let sessions = sessions.clone();
                let workflows = workflows.clone();
                Box::pin(async move {
                    let wire::command_request::Command::CreateWorktrees(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let state =
                        state.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    let launcher = crate::usecase::create_worktrees::WorktreeLauncher {
                        sessions: sessions
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?,
                        workflows: workflows
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?,
                    };
                    let launch = worktree::parse_launch(args.launch)?;
                    let paths = crate::usecase::create_worktrees::create_worktrees(
                        state.repository_usecase.clone(),
                        &launcher,
                        args.repo_path,
                        args.branches,
                        args.base_branch,
                        launch,
                    )
                    .await
                    .map_err(AppError::from)?;
                    Ok(wire::command_result::Command::CreateWorktrees(value(
                        paths,
                    )?))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["find_repository_root"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::FindRepositoryRoot(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let state =
                        state.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    let path = required(args.path, "path")?;
                    let result =
                        run_blocking(move || state.repository_usecase.find_main_repo_path(&path))
                            .await;
                    Ok(wire::command_result::Command::FindRepositoryRoot(outcome(
                        result,
                    )?))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["add_repo_path"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::AddRepoPath(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            repo_paths::add_repo_path_shared(
                                &state,
                                convert(required(args.path, "path")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::AddRepoPath(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["create_worktree"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::CreateWorktree(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::create_worktree_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.branch, "branch")?)?,
                                convert(required(args.create_branch, "createBranch")?)?,
                                optional(args.base_branch)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::CreateWorktree(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["git_create_branch"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GitCreateBranch(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            branch::git_create_branch_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.branch_name, "branchName")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GitCreateBranch(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["remove_repo_path"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RemoveRepoPath(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            repo_paths::remove_repo_path_shared(
                                &state,
                                convert(required(args.path, "path")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RemoveRepoPath(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["remove_worktree"],
            Box::new(move |command| {
                let state = state.clone();
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RemoveWorktree(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::remove_worktree_shared(
                                &state,
                                runtime,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.force, "force")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RemoveWorktree(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["set_branch_base"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SetBranchBase(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            git_config::set_branch_base_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.branch_name, "branchName")?)?,
                                optional(args.base)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::SetBranchBase(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["set_releash_base"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SetReleashBase(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            git_config::set_releash_base_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                optional(args.base)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::SetReleashBase(result))
                })
            }),
        );
    }
}
