use std::sync::Arc;

use crate::adaptor::gateway::external_editor::{
    EditorSettingsConfigGateway, MacInstalledEditorGateway, TauriEditorLauncherGateway,
};
use crate::domain::app_config::ConfigRepository;
use crate::domain::external_editor::EditorSettingsGateway;
use crate::usecase::external_editor::dto::EditorInfoDto;

#[tauri::command]
pub fn detect_editors() -> Vec<EditorInfoDto> {
    crate::usecase::external_editor::detect_usecase::detect_editors(&MacInstalledEditorGateway)
        .into_iter()
        .map(Into::into)
        .collect()
}

#[tauri::command]
pub fn open_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    file_path: String,
) -> Result<(), String> {
    let launcher = TauriEditorLauncherGateway::new(app);
    let settings = EditorSettingsConfigGateway::new(state.inner().clone());
    crate::usecase::external_editor::open_usecase::open_in_editor(&launcher, &settings, &file_path)
}

#[tauri::command]
pub fn open_folder_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    folder_path: String,
) -> Result<(), String> {
    let launcher = TauriEditorLauncherGateway::new(app);
    let settings = EditorSettingsConfigGateway::new(state.inner().clone());
    crate::usecase::external_editor::open_usecase::open_folder_in_editor(
        &launcher,
        &settings,
        &folder_path,
    )
}

#[tauri::command]
pub fn get_external_editor(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
) -> Result<String, String> {
    EditorSettingsConfigGateway::new(state.inner().clone()).selected_editor()
}

#[tauri::command]
pub async fn update_external_editor(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    editor: String,
) -> Result<(), String> {
    let settings = EditorSettingsConfigGateway::new(state.inner().clone());
    tokio::task::spawn_blocking(move || settings.update_selected_editor(editor))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
