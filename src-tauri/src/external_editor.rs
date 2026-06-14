use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize)]
pub struct EditorInfo {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptEditorDraftInfo {
    pub id: String,
    pub file_path: String,
}

struct PromptEditorDraft {
    file_path: PathBuf,
    dir_path: PathBuf,
}

#[derive(Default)]
pub struct PromptEditorDraftStore {
    drafts: Mutex<HashMap<String, PromptEditorDraft>>,
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

fn validate_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err(format!("{label}が指定されていません"));
    }
    Ok(())
}

pub(crate) fn open_path_with_opener(
    app: &tauri::AppHandle,
    path: &str,
    editor: &str,
    label: &str,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    if editor.is_empty() {
        app.opener()
            .open_path(path, None::<&str>)
            .map_err(|e| format!("{label}を開けませんでした: {e}"))
    } else {
        app.opener()
            .open_path(path, Some(editor))
            .map_err(|e| format!("エディタで{label}を開けませんでした: {e}"))
    }
}

#[tauri::command]
pub fn open_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    file_path: String,
) -> Result<(), String> {
    validate_path(&file_path, "ファイルパス")?;
    let config = state.get_config()?;
    open_path_with_opener(&app, &file_path, &config.app.external_editor, "ファイル")
}

#[tauri::command]
pub fn open_folder_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    folder_path: String,
) -> Result<(), String> {
    validate_path(&folder_path, "フォルダパス")?;
    let config = state.get_config()?;
    open_path_with_opener(&app, &folder_path, &config.app.external_editor, "フォルダ")
}

fn create_prompt_editor_draft(
    store: &PromptEditorDraftStore,
    content: &str,
) -> Result<PromptEditorDraftInfo, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let dir_path = std::env::temp_dir().join(format!("releash-agent-prompt-{id}"));
    fs::create_dir_all(&dir_path)
        .map_err(|e| format!("Prompt draft用の一時ディレクトリを作成できませんでした: {e}"))?;
    let file_path = dir_path.join("prompt.md");
    fs::write(&file_path, content)
        .map_err(|e| format!("Prompt draftを書き込めませんでした: {e}"))?;
    let file_path_string = file_path.to_string_lossy().into_owned();
    store.drafts.lock().insert(
        id.clone(),
        PromptEditorDraft {
            file_path,
            dir_path,
        },
    );
    Ok(PromptEditorDraftInfo {
        id,
        file_path: file_path_string,
    })
}

fn read_and_remove_prompt_editor_draft(
    store: &PromptEditorDraftStore,
    draft_id: &str,
) -> Result<String, String> {
    let draft = store
        .drafts
        .lock()
        .remove(draft_id)
        .ok_or_else(|| format!("Prompt draft not found: {draft_id}"))?;
    let content = fs::read_to_string(&draft.file_path)
        .map_err(|e| format!("Prompt draftを読み込めませんでした: {e}"));
    let _ = fs::remove_dir_all(&draft.dir_path);
    content
}

fn discard_prompt_editor_draft(store: &PromptEditorDraftStore, draft_id: &str) -> bool {
    let Some(draft) = store.drafts.lock().remove(draft_id) else {
        return false;
    };
    let _ = fs::remove_dir_all(&draft.dir_path);
    true
}

#[tauri::command]
pub fn open_agent_prompt_in_external_editor(
    app: tauri::AppHandle,
    config_state: tauri::State<'_, Arc<AppConfig>>,
    draft_store: tauri::State<'_, PromptEditorDraftStore>,
    content: String,
) -> Result<PromptEditorDraftInfo, String> {
    let draft = create_prompt_editor_draft(&draft_store, &content)?;
    let config = config_state.get_config()?;
    if let Err(e) = open_path_with_opener(
        &app,
        &draft.file_path,
        &config.app.external_editor,
        "Prompt draft",
    ) {
        if let Some(stored_draft) = draft_store.drafts.lock().remove(&draft.id) {
            let _ = fs::remove_dir_all(&stored_draft.dir_path);
        }
        return Err(e);
    }
    Ok(draft)
}

#[tauri::command]
pub fn import_agent_prompt_external_editor_draft(
    draft_store: tauri::State<'_, PromptEditorDraftStore>,
    draft_id: String,
) -> Result<String, String> {
    read_and_remove_prompt_editor_draft(&draft_store, &draft_id)
}

#[tauri::command]
pub fn discard_agent_prompt_external_editor_draft(
    draft_store: tauri::State<'_, PromptEditorDraftStore>,
    draft_id: String,
) -> Result<bool, String> {
    Ok(discard_prompt_editor_draft(&draft_store, &draft_id))
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

    #[test]
    fn prompt_editor_draft_round_trips_content_and_removes_entry() {
        let store = PromptEditorDraftStore::default();
        let draft = create_prompt_editor_draft(&store, "hello\nprompt").unwrap();

        assert!(Path::new(&draft.file_path).exists());
        fs::write(&draft.file_path, "edited prompt").unwrap();

        let edited = read_and_remove_prompt_editor_draft(&store, &draft.id).unwrap();

        assert_eq!(edited, "edited prompt");
        assert!(read_and_remove_prompt_editor_draft(&store, &draft.id).is_err());
    }

    #[test]
    fn reading_missing_prompt_editor_draft_errors() {
        let store = PromptEditorDraftStore::default();

        let err = read_and_remove_prompt_editor_draft(&store, "missing").unwrap_err();

        assert_eq!(err, "Prompt draft not found: missing");
    }

    #[test]
    fn discard_prompt_editor_draft_removes_entry() {
        let store = PromptEditorDraftStore::default();
        let draft = create_prompt_editor_draft(&store, "discard me").unwrap();

        assert!(discard_prompt_editor_draft(&store, &draft.id));
        assert!(!discard_prompt_editor_draft(&store, &draft.id));
        assert!(read_and_remove_prompt_editor_draft(&store, &draft.id).is_err());
    }

    #[test]
    fn validate_path_rejects_empty() {
        let result = validate_path("", "ファイルパス");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "ファイルパスが指定されていません");
    }

    #[test]
    fn validate_path_rejects_empty_folder() {
        let result = validate_path("", "フォルダパス");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "フォルダパスが指定されていません");
    }

    #[test]
    fn validate_path_accepts_non_empty() {
        assert!(validate_path("/some/path", "ファイルパス").is_ok());
        assert!(validate_path("/some/folder", "フォルダパス").is_ok());
    }
}
