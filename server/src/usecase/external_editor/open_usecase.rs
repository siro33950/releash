use crate::domain::app_config::AppConfigRepository;
use crate::domain::external_editor::services::validate_path;
use crate::domain::external_editor::EditorError;
use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};

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

/// アプリ設定の外部エディタ選択を保存し、変更を通知する。
pub fn update_external_editor(
    repository: &dyn AppConfigRepository,
    publisher: Option<&StateSubscriptionUsecase>,
    editor: String,
    config_mutation: &parking_lot::Mutex<()>,
) -> Result<(), EditorError> {
    let guard = config_mutation.lock();
    let mut config = repository.load().map_err(EditorError::Settings)?;
    config.update_external_editor(editor);
    repository.save(config).map_err(EditorError::Settings)?;
    drop(guard);

    if let Some(publisher) = publisher {
        publisher.notify(StateChangeSource::AppConfig);
    }
    Ok(())
}

#[cfg(test)]
#[path = "open_usecase_test.rs"]
mod open_usecase_tests;
