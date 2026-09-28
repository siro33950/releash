use crate::domain::external_editor::services::validate_path;
use crate::domain::external_editor::EditorError;
use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};

pub fn open_in_editor(
    launcher: &dyn EditorLauncherGateway,
    settings: &dyn EditorSettingsGateway,
    file_path: &str,
) -> Result<(), EditorError> {
    validate_path(file_path, "ファイルパス").map_err(EditorError::InvalidInput)?;
    let editor = settings.selected_editor()?;
    launcher.open_path(file_path, &editor, "ファイル")
}

pub fn open_folder_in_editor(
    launcher: &dyn EditorLauncherGateway,
    settings: &dyn EditorSettingsGateway,
    folder_path: &str,
) -> Result<(), EditorError> {
    validate_path(folder_path, "フォルダパス").map_err(EditorError::InvalidInput)?;
    let editor = settings.selected_editor()?;
    launcher.open_path(folder_path, &editor, "フォルダ")
}

pub fn get_external_editor(settings: &dyn EditorSettingsGateway) -> Result<String, EditorError> {
    settings.selected_editor()
}

pub fn update_external_editor(
    settings: &dyn EditorSettingsGateway,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionOutputRef>,
    editor: String,
) -> Result<(), EditorError> {
    settings.update_selected_editor(editor)?;
    if let Some(publisher) = publisher {
        publisher.invalidate(crate::usecase::state_subscription::StateChangeSource::AppConfig);
    }
    Ok(())
}

#[cfg(test)]
#[path = "open_usecase_test.rs"]
mod open_usecase_tests;
