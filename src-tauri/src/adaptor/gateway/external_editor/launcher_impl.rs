use crate::domain::external_editor::EditorLauncherGateway;

#[derive(Clone)]
pub struct TauriEditorLauncherGateway<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriEditorLauncherGateway<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime + 'static> EditorLauncherGateway for TauriEditorLauncherGateway<R> {
    fn open_path(&self, path: &str, editor: &str, label: &str) -> Result<(), String> {
        use tauri_plugin_opener::OpenerExt;

        if editor.is_empty() {
            self.app
                .opener()
                .open_path(path, None::<&str>)
                .map_err(|e| format!("{label}を開けませんでした: {e}"))
        } else {
            self.app
                .opener()
                .open_path(path, Some(editor))
                .map_err(|e| format!("エディタで{label}を開けませんでした: {e}"))
        }
    }
}
