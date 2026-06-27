use tauri::Manager;

pub(crate) struct TestDataDir(pub std::path::PathBuf);

pub(crate) fn resolve_data_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<std::path::PathBuf, String> {
    if let Some(data_dir) = app.try_state::<TestDataDir>() {
        return Ok(data_dir.0.clone());
    }

    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}
