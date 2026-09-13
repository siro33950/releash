use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::client::{invalid_request, outcome, value};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        router.register_domain(
            &["detect_editors"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::DetectEditors(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move { value(commands::detect_editors_shared()) }.await?;
                    Ok(wire::command_result::Command::DetectEditors(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["get_external_editor"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::GetExternalEditor(_args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(commands::get_external_editor_shared(&state))
                    }
                    .await?;
                    Ok(wire::command_result::Command::GetExternalEditor(result))
                })
            }),
        );
    }
    {
        let state = deps.config_repository.clone();
        router.register_domain(
            &["update_external_editor"],
            Box::new(move |command| {
                let state = state.clone();
                Box::pin(async move {
                    let wire::command_request::Command::UpdateExternalEditor(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let state = state
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        outcome(
                            commands::update_external_editor_shared(
                                &state,
                                convert(required(args.editor, "editor")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::UpdateExternalEditor(result))
                })
            }),
        );
    }
    {
        let config = deps.config_repository.clone();
        let launcher = deps.editor_launcher.clone();
        router.register_domain(
            &["open_in_editor"],
            Box::new(move |command| {
                let config = config.clone();
                let launcher = launcher.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenInEditor(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let config = config
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let settings =
                        crate::adaptor::gateway::external_editor::EditorSettingsConfigGateway::new(
                            config,
                        );
                        outcome(
                            crate::usecase::external_editor::open_usecase::open_in_editor(
                                launcher.as_ref(),
                                &settings,
                                &required(args.file_path, "filePath")?,
                            ),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenInEditor(result))
                })
            }),
        );
    }
    {
        let config = deps.config_repository.clone();
        let launcher = deps.editor_launcher.clone();
        router.register_domain(
            &["open_folder_in_editor"],
            Box::new(move |command| {
                let config = config.clone();
                let launcher = launcher.clone();
                Box::pin(async move {
                    let wire::command_request::Command::OpenFolderInEditor(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let config = config
                            .ok_or_else(|| invalid_request("Command dependency unavailable"))?;
                        let settings =
                        crate::adaptor::gateway::external_editor::EditorSettingsConfigGateway::new(
                            config,
                        );
                        outcome(
                            crate::usecase::external_editor::open_usecase::open_folder_in_editor(
                                launcher.as_ref(),
                                &settings,
                                &required(args.folder_path, "folderPath")?,
                            ),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::OpenFolderInEditor(result))
                })
            }),
        );
    }
}
