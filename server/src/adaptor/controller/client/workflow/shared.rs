use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, optional, required};
use crate::adaptor::controller::client::{invalid_request, outcome};
use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::error::AppError;

pub fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let state = deps.app_state.clone();
        router.register_domain(
            &["diagnose_workflow_directory"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::DiagnoseWorkflowDirectory(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            diagnostics::diagnose_workflow_directory_shared(&state, args.dir).await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::DiagnoseWorkflowDirectory(
                        result,
                    ))
                })
            }),
        );
    }
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
            &["open_workflow_directory_in_editor"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenWorkflowDirectoryInEditor(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            state
                                .workflow_usecase
                                .open_workflow_directory_in_editor()
                                .map_err(AppError::from_failure),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenWorkflowDirectoryInEditor(result))
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
            &["workflow_submit_output"],
            Box::new(move |command| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    let wire::command_request::Command::WorkflowSubmitOutput(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let runtime = runtime
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            output::workflow_submit_output_shared(
                                &runtime,
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
}
