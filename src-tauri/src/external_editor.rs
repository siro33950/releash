use serde::Serialize;
use std::path::PathBuf;
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

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    dirs
}

fn scan_applications_in(dirs: &[PathBuf]) -> Vec<EditorInfo> {
    let mut editors = Vec::new();

    for (name, app_bundle) in KNOWN_EDITORS {
        for dir in dirs {
            let app_path = dir.join(app_bundle);
            if app_path.exists() {
                editors.push(EditorInfo {
                    name: name.to_string(),
                    path: app_path.to_string_lossy().into_owned(),
                });
                break;
            }
        }
    }

    editors
}

fn scan_applications() -> Vec<EditorInfo> {
    scan_applications_in(&application_dirs())
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

#[tauri::command]
pub fn open_folder_in_editor(
    state: tauri::State<'_, Arc<AppConfig>>,
    folder_path: String,
) -> Result<(), String> {
    if folder_path.is_empty() {
        return Err("フォルダパスが指定されていません".to_string());
    }

    let config = state.get_config()?;
    let editor = &config.app.external_editor;

    let output = if editor.is_empty() {
        std::process::Command::new("open")
            .arg(&folder_path)
            .output()
    } else {
        std::process::Command::new("open")
            .arg("-a")
            .arg(editor)
            .arg(&folder_path)
            .output()
    };

    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            Err(format!("エディタでフォルダを開けませんでした: {stderr}"))
        }
        Err(e) => Err(format!("コマンドの実行に失敗しました: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn known_editors_list_is_not_empty() {
        assert!(!KNOWN_EDITORS.is_empty());
    }

    #[test]
    fn scan_finds_existing_editors_in_dirs() {
        let tmp = TempDir::new().unwrap();
        let apps_dir = tmp.path().to_path_buf();
        fs::create_dir_all(apps_dir.join("Cursor.app")).unwrap();
        fs::create_dir_all(apps_dir.join("Zed.app")).unwrap();

        let editors = scan_applications_in(&[apps_dir]);
        assert_eq!(editors.len(), 2);
        assert_eq!(editors[0].name, "Cursor");
        assert_eq!(editors[1].name, "Zed");
    }

    #[test]
    fn scan_returns_empty_for_no_editors() {
        let tmp = TempDir::new().unwrap();
        let editors = scan_applications_in(&[tmp.path().to_path_buf()]);
        assert!(editors.is_empty());
    }

    #[test]
    fn scan_deduplicates_across_dirs() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        fs::create_dir_all(tmp1.path().join("Cursor.app")).unwrap();
        fs::create_dir_all(tmp2.path().join("Cursor.app")).unwrap();

        let editors = scan_applications_in(&[tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
        assert_eq!(editors.len(), 1);
        assert!(editors[0].path.contains(tmp1.path().to_str().unwrap()));
    }

    #[test]
    fn scan_prefers_first_dir() {
        let sys_dir = TempDir::new().unwrap();
        let user_dir = TempDir::new().unwrap();
        fs::create_dir_all(sys_dir.path().join("Zed.app")).unwrap();
        fs::create_dir_all(user_dir.path().join("Zed.app")).unwrap();

        let editors =
            scan_applications_in(&[sys_dir.path().to_path_buf(), user_dir.path().to_path_buf()]);
        assert_eq!(editors.len(), 1);
        assert!(editors[0].path.contains(sys_dir.path().to_str().unwrap()));
    }

    #[test]
    fn application_dirs_includes_system() {
        let dirs = application_dirs();
        assert!(dirs.iter().any(|d| d == Path::new("/Applications")));
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
