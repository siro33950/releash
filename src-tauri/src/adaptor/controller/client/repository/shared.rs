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
            &["delete_branch"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DeleteBranch(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            branch::delete_branch_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.branch_name, "branchName")?)?,
                                convert(required(args.force, "force")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DeleteBranch(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_branch_base"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetBranchBase(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            git_config::get_branch_base_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                convert(required(args.branch_name, "branchName")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetBranchBase(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_cwd"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetCwd(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(util::get_cwd_shared(&state).await)
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetCwd(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_default_branch"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetDefaultBranch(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            branch::get_default_branch_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetDefaultBranch(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_git_log"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetGitLog(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            log::get_git_log_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                optional(args.limit)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetGitLog(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_git_status"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetGitStatus(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            status::get_git_status_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                                optional(args.include_ignored)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetGitStatus(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_git_status_snapshot"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetGitStatusSnapshot(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            status::get_git_status_snapshot_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetGitStatusSnapshot(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_main_repo_path"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetMainRepoPath(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::get_main_repo_path_shared(
                                &state,
                                convert(required(args.any_path, "anyPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetMainRepoPath(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_releash_base"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetReleashBase(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            git_config::get_releash_base_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetReleashBase(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_repo_git_dir"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetRepoGitDir(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            util::get_repo_git_dir_shared(
                                &state,
                                convert(required(args.file_path, "filePath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetRepoGitDir(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_repo_paths"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetRepoPaths(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        value(repo_paths::get_repo_paths_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetRepoPaths(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_status_diff_stats"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetStatusDiffStats(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            status::get_status_diff_stats_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetStatusDiffStats(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_status_diff_stats_snapshot"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetStatusDiffStatsSnapshot(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            status::get_status_diff_stats_snapshot_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetStatusDiffStatsSnapshot(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_worktree_dirty_count"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorktreeDirtyCount(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::get_worktree_dirty_count_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorktreeDirtyCount(result))
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
            &["list_branches"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListBranches(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            branch::list_branches_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListBranches(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_branches_with_status"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListBranchesWithStatus(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::list_branches_with_status_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListBranchesWithStatus(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_branches_with_status_snapshot"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListBranchesWithStatusSnapshot(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::list_branches_with_status_snapshot_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListBranchesWithStatusSnapshot(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_worktrees"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListWorktrees(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::list_worktrees_shared(
                                &state,
                                convert(required(args.repo_path, "repoPath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListWorktrees(result))
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
        router.register_domain(
            &["remove_worktree"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RemoveWorktree(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            worktree::remove_worktree_shared(
                                &state,
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
