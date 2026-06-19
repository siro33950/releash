use std::sync::Arc;

use crate::adaptor::gateway::external_editor::{
    EditorSettingsConfigGateway, MacInstalledEditorGateway, TauriEditorLauncherGateway,
};
use crate::config::AppConfig;
use crate::domain::external_editor::{EditorInfo, EditorSettingsGateway};

#[tauri::command]
pub fn detect_editors() -> Vec<EditorInfo> {
    crate::usecase::external_editor::detect_usecase::detect_editors(&MacInstalledEditorGateway)
}

#[tauri::command]
pub fn open_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    file_path: String,
) -> Result<(), String> {
    let launcher = TauriEditorLauncherGateway::new(app);
    let settings = EditorSettingsConfigGateway::new(state.inner().clone());
    crate::usecase::external_editor::open_usecase::open_in_editor(&launcher, &settings, &file_path)
}

#[tauri::command]
pub fn open_folder_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
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
pub fn get_external_editor(state: tauri::State<'_, Arc<AppConfig>>) -> Result<String, String> {
    EditorSettingsConfigGateway::new(state.inner().clone()).selected_editor()
}

#[tauri::command]
pub async fn update_external_editor(
    state: tauri::State<'_, Arc<AppConfig>>,
    editor: String,
) -> Result<(), String> {
    let settings = EditorSettingsConfigGateway::new(state.inner().clone());
    tokio::task::spawn_blocking(move || settings.update_selected_editor(editor))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
