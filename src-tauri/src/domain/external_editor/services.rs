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
    fn validate_path_rejects_empty() {
        let result = validate_path("", "ファイルパス");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "ファイルパスが指定されていません");
    }

    #[test]
    fn validate_path_accepts_non_empty() {
        assert!(validate_path(Path::new("/some/path").to_str().unwrap(), "ファイルパス").is_ok());
    }
}
