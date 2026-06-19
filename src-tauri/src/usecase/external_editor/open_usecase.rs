use crate::domain::external_editor::services::validate_path;
use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};

pub fn open_in_editor(
    launcher: &dyn EditorLauncherGateway,
    settings: &dyn EditorSettingsGateway,
    file_path: &str,
) -> Result<(), String> {
    validate_path(file_path, "ファイルパス")?;
    let editor = settings.selected_editor()?;
    launcher.open_path(file_path, &editor, "ファイル")
}

pub fn open_folder_in_editor(
    launcher: &dyn EditorLauncherGateway,
    settings: &dyn EditorSettingsGateway,
    folder_path: &str,
) -> Result<(), String> {
    validate_path(folder_path, "フォルダパス")?;
    let editor = settings.selected_editor()?;
    launcher.open_path(folder_path, &editor, "フォルダ")
}
