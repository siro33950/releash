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
        let state = deps.app_config_usecase.clone();
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
                        let app = required(args.app, "app")?;
                        outcome(
                            commands::update_app_settings_shared(
                                &state,
                                required(app.close_to_tray, "close_to_tray")?,
                                required(app.start_minimized, "start_minimized")?,
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
        let state = deps.app_config_usecase.clone();
        router.register_domain(
            &["update_login_item_preference"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateLoginItemPreference(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let state =
                        state.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                    let result = outcome(
                        commands::update_login_item_preference_shared(
                            &state,
                            required(args.requested, "requested")?,
                        )
                        .await,
                    )?;
                    Ok(wire::command_result::Command::UpdateLoginItemPreference(
                        result,
                    ))
                })
            }),
        );
    }
    {
        let state = deps.app_config_usecase.clone();
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
        let state = deps.app_config_usecase.clone();
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
    register_workflow_config(router, deps.app_config_usecase.clone());
}

fn register_workflow_config(
    router: &mut ClientCommandDispatch,
    state: Option<std::sync::Arc<crate::usecase::app_config::AppConfigUsecase>>,
) {
    router.register_domain(
        &["update_workflow_config"],
        Box::new(move |command| {
            let state = state.clone();
            Box::pin(async move {
                let wire::command_request::Command::UpdateWorkflowConfig(args) = command else {
                    return Err(invalid_request("Mismatched command"));
                };
                let result = async move {
                    let state =
                        state.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
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

#[cfg(test)]
#[path = "shared_test.rs"]
mod shared_tests;
