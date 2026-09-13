use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["abort_workflow"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::AbortWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            runtime::abort_workflow_shared(
                                &runtime,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::AbortWorkflow(result))
                })
            }),
        );
    }
    {
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["approve_workflow_node"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ApproveWorkflowNode(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            runtime::approve_workflow_node_shared(
                                &runtime,
                                convert(required(args.args, "args")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ApproveWorkflowNode(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["delete_facet"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DeleteFacet(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::delete_facet_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                                convert(required(args.key, "key")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DeleteFacet(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["delete_workflow"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DeleteWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            definition::delete_workflow_shared(
                                &state,
                                convert(required(args.name, "name")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DeleteWorkflow(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["diagnose_all_cmd"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DiagnoseAllCmd(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            diagnostics::diagnose_all_cmd_shared(&state, optional(args.dir)?).await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DiagnoseAllCmd(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["duplicate_facet"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DuplicateFacet(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::duplicate_facet_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                                convert(required(args.source_key, "sourceKey")?)?,
                                convert(required(args.new_key, "newKey")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DuplicateFacet(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["duplicate_workflow"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DuplicateWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            definition::duplicate_workflow_shared(
                                &state,
                                convert(required(args.source_name, "sourceName")?)?,
                                convert(required(args.new_name, "newName")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DuplicateWorkflow(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_automation_config_dir"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetAutomationConfigDir(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(diagnostics::get_automation_config_dir_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetAutomationConfigDir(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_facet"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetFacet(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::get_facet_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                                convert(required(args.key, "key")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetFacet(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            definition::get_workflow_shared(
                                &state,
                                convert(required(args.name, "name")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflow(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow_execution"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowExecution(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::get_workflow_execution_shared(
                                &state,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowExecution(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow_execution_log"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowExecutionLog(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::get_workflow_execution_log_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowExecutionLog(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow_execution_state"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowExecutionState(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::get_workflow_execution_state_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowExecutionState(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow_node_detail"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowNodeDetail(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::get_workflow_node_detail_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                                convert(required(args.node_execution_id, "nodeExecutionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowNodeDetail(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["get_workflow_source"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowSource(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            definition::get_workflow_source_shared(
                                &state,
                                convert(required(args.name, "name")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowSource(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_facet_summaries"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListFacetSummaries(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::list_facet_summaries_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListFacetSummaries(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_facets"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListFacets(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::list_facets_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListFacets(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_workflow_executions"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListWorkflowExecutions(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::list_workflow_executions_shared(
                                &state,
                                optional(args.status)?,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListWorkflowExecutions(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["list_workflows"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ListWorkflows(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(definition::list_workflows_shared(&state).await)
                    }
                    .await?;
                    Ok(wire::command_result::Command::ListWorkflows(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["open_facet_in_editor"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenFacetInEditor(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(facet::open_facet_in_editor_shared(
                            &state,
                            convert(required(args.kind, "kind")?)?,
                            convert(required(args.key, "key")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenFacetInEditor(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["open_workflow_in_editor"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenWorkflowInEditor(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(definition::open_workflow_in_editor_shared(
                            &state,
                            convert(required(args.name, "name")?)?,
                        ))
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenWorkflowInEditor(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["render_facet_preview"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RenderFacetPreview(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            diagnostics::render_facet_preview_shared(
                                &state,
                                convert(required(args.content, "content")?)?,
                                convert(required(args.sample_values, "sampleValues")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RenderFacetPreview(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["resolve_active_execution_by_worktree"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResolveActiveExecutionByWorktree(args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::resolve_active_execution_by_worktree_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResolveActiveExecutionByWorktree(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["resolve_worktree_by_execution"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResolveWorktreeByExecution(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            execution::resolve_worktree_by_execution_shared(
                                &state,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResolveWorktreeByExecution(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["resume_workflow"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::ResumeWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            runtime::resume_workflow_shared(
                                &runtime,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::ResumeWorkflow(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["save_facet"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SaveFacet(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            facet::save_facet_shared(
                                &state,
                                convert(required(args.kind, "kind")?)?,
                                convert(required(args.key, "key")?)?,
                                convert(required(args.content, "content")?)?,
                                optional(args.is_new)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::SaveFacet(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["save_workflow_source"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::SaveWorkflowSource(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            definition::save_workflow_source_shared(
                                &state,
                                convert(required(args.source, "source")?)?,
                                optional(args.original_name)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::SaveWorkflowSource(result))
                })
            }),
        );
    }
    {
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["start_workflow"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StartWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            runtime::start_workflow_shared(
                                &runtime,
                                convert(required(args.workflow_name, "workflowName")?)?,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                optional(args.request)?,
                                optional(args.created_from)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::StartWorkflow(result))
                })
            }),
        );
    }
    {
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["stop_workflow"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StopWorkflow(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            runtime::stop_workflow_shared(
                                &runtime,
                                convert(required(args.execution_id, "executionId")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::StopWorkflow(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["workflow_get_output"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WorkflowGetOutput(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            output::workflow_get_output_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                                convert(required(args.node_name, "nodeName")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::WorkflowGetOutput(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        let runtime = deps.workflow_runtime_usecase.clone();
        router.register_domain(
            &["workflow_submit_output"],
            Box::new(move |command| {
                let state = state.clone();
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WorkflowSubmitOutput(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            output::workflow_submit_output_shared(
                                &state,
                                &runtime,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.node_execution_id, "nodeExecutionId")?)?,
                                optional(args.artifact)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::WorkflowSubmitOutput(result))
                })
            }),
        );
    }
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["workflow_validate_output"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WorkflowValidateOutput(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            output::workflow_validate_output_shared(
                                &state,
                                convert(required(args.worktree_path, "worktreePath")?)?,
                                convert(required(args.execution_id, "executionId")?)?,
                                convert(required(args.node_name, "nodeName")?)?,
                                convert(required(args.structured_output, "structuredOutput")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::WorkflowValidateOutput(
                        result,
                    ))
                })
            }),
        );
    }
}
