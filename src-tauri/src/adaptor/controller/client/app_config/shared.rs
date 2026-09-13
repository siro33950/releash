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
        let state = deps.config_repository.clone();
        router.register_domain(
            &["get_app_settings"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetAppSettings(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_app_settings_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetAppSettings(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["get_crash_reporting_enabled"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetCrashReportingEnabled(_args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_crash_reporting_enabled_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetCrashReportingEnabled(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["get_performance_telemetry_enabled"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetPerformanceTelemetryEnabled(_args) =
                        command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_performance_telemetry_enabled_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetPerformanceTelemetryEnabled(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["get_workflow_config"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetWorkflowConfig(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_workflow_config_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetWorkflowConfig(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["update_app_settings"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateAppSettings(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            commands::update_app_settings_shared(
                                &state,
                                convert(required(args.app, "app")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdateAppSettings(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["update_crash_reporting"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateCrashReporting(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            commands::update_crash_reporting_shared(
                                &state,
                                convert(required(args.enabled, "enabled")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdateCrashReporting(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["update_performance_telemetry"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdatePerformanceTelemetry(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            commands::update_performance_telemetry_shared(
                                &state,
                                convert(required(args.enabled, "enabled")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdatePerformanceTelemetry(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["update_workflow_config"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateWorkflowConfig(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            commands::update_workflow_config_shared(
                                &state,
                                convert(required(args.workflow, "workflow")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdateWorkflowConfig(result))
                })
            }),
        );
    }
}
