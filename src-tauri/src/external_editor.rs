use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize)]
pub struct EditorInfo {
    pub name: String,
    pub path: String,
}

const KNOWN_EDITORS: &[(&str, &str)] = &[
    ("Visual Studio Code", "Visual Studio Code.app"),
    ("Cursor", "Cursor.app"),
    ("Zed", "Zed.app"),
    ("Sublime Text", "Sublime Text.app"),
    ("TextMate", "TextMate.app"),
    ("Nova", "Nova.app"),
    ("BBEdit", "BBEdit.app"),
    ("CotEditor", "CotEditor.app"),
];

fn scan_applications() -> Vec<EditorInfo> {
    let mut editors = Vec::new();

    for (name, app_bundle) in KNOWN_EDITORS {
        let app_path = format!("/Applications/{app_bundle}");
        if Path::new(&app_path).exists() {
            editors.push(EditorInfo {
                name: name.to_string(),
                path: app_path,
            });
        }
    }

    editors
}

#[tauri::command]
pub fn detect_editors() -> Vec<EditorInfo> {
    scan_applications()
}

#[tauri::command]
pub fn open_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    file_path: String,
) -> Result<(), String> {
    if file_path.is_empty() {
        return Err("ファイルパスが指定されていません".to_string());
    }

    use tauri_plugin_opener::OpenerExt;

    let config = state.get_config()?;
    let editor = &config.app.external_editor;

    if editor.is_empty() {
        app.opener()
            .open_path(&file_path, None::<&str>)
            .map_err(|e| format!("ファイルを開けませんでした: {e}"))
    } else {
        app.opener()
            .open_path(&file_path, Some(editor.as_str()))
            .map_err(|e| format!("エディタでファイルを開けませんでした: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_editors_list_is_not_empty() {
        assert!(!KNOWN_EDITORS.is_empty());
    }

    #[test]
    fn scan_applications_returns_only_existing() {
        let editors = scan_applications();
        for editor in &editors {
            assert!(Path::new(&editor.path).exists());
        }
    }

    #[test]
    fn editor_info_serializes() {
        let info = EditorInfo {
            name: "Test Editor".to_string(),
            path: "/Applications/Test.app".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("Test Editor"));
        assert!(json.contains("/Applications/Test.app"));
    }
}
