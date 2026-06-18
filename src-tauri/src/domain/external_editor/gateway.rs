use crate::domain::external_editor::EditorInfo;

pub trait InstalledEditorGateway: Send + Sync {
    fn scan(&self) -> Vec<EditorInfo>;
}

pub trait EditorLauncherGateway: Send + Sync {
    fn open_path(&self, path: &str, editor: &str, label: &str) -> Result<(), String>;
}

pub trait EditorSettingsGateway: Send + Sync {
    fn selected_editor(&self) -> Result<String, String>;
    fn update_selected_editor(&self, editor: String) -> Result<(), String>;
}
