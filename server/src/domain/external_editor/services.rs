use std::path::PathBuf;

use crate::domain::external_editor::EditorInfo;

pub const KNOWN_EDITORS: &[(&str, &str)] = &[
    ("Visual Studio Code", "Visual Studio Code.app"),
    ("Cursor", "Cursor.app"),
    ("Zed", "Zed.app"),
    ("Sublime Text", "Sublime Text.app"),
    ("TextMate", "TextMate.app"),
    ("Nova", "Nova.app"),
    ("BBEdit", "BBEdit.app"),
    ("CotEditor", "CotEditor.app"),
];

pub fn scan_applications_in(dirs: &[PathBuf]) -> Vec<EditorInfo> {
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

pub fn validate_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err(format!("{label}が指定されていません"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "services_test.rs"]
pub(crate) mod services_tests;
