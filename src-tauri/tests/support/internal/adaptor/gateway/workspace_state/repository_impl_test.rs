use super::*;
use crate::domain::workspace_state::value_objects::{
    workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
};
use releash_lib::test_support::integration::domain::workspace_state::WorkspaceStateRepository;
use tempfile::TempDir;

fn make_state() -> WorkspaceState {
    WorkspaceState {
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![
                WorkspaceTabEntry {
                    path: "src/main.rs".to_string(),
                    name: "main.rs".to_string(),
                },
                WorkspaceTabEntry {
                    path: "src/lib.rs".to_string(),
                    name: "lib.rs".to_string(),
                },
            ],
            active_editor_path: Some("src/main.rs".to_string()),
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".to_string(),
            active_view: "git".to_string(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    }
}

#[test]
pub fn test_表示状態読取_破損を未保存と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "broken json").unwrap();
    // When
    let result = store.load("broken", "/repo");
    // Then
    assert!(result.is_err());
}

#[test]
pub fn test_表示状態読取_io失敗を未保存と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::create_dir(&path).unwrap();
    // When
    let result = store.load("broken", "/repo");
    // Then
    assert!(result.is_err());
}

#[test]
pub fn save_and_load_roundtrip() {
    // Given
    let dir = TempDir::new().unwrap();
    let worktree_dir = dir.path().join("worktree");
    std::fs::create_dir_all(worktree_dir.join("src")).unwrap();
    std::fs::write(worktree_dir.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(worktree_dir.join("src/lib.rs"), "// lib").unwrap();

    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    store.set("wt1", make_state());
    store.save("wt1").unwrap();

    // When
    let loaded = store
        .load("wt1", worktree_dir.to_str().unwrap())
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.tabs.editors.len(), 2);
    assert_eq!(loaded.layout.center_tab, "editor");
}

#[test]
pub fn load_nonexistent_returns_none() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    // When
    let loaded = store.load("nonexistent", "/tmp").unwrap();
    // Then
    assert!(loaded.is_none());
}

#[test]
pub fn get_set_in_memory() {
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    assert!(store.get("wt1").is_none());
    store.set("wt1", make_state());
    assert_eq!(store.get("wt1").unwrap().tabs.editors.len(), 2);
}

#[test]
pub fn save_returns_workspace_state_error_when_state_dir_cannot_be_created() {
    let dir = TempDir::new().unwrap();
    let app_data_file = dir.path().join("app-data");
    std::fs::write(&app_data_file, "not a directory").unwrap();

    let store = WorkspaceStateStore::new(app_data_file);
    store.set("wt1", make_state());

    let err = store.save("wt1").unwrap_err();
    assert!(matches!(err, WorkspaceStateError::Message(_)));
    assert!(err.to_string().contains("Failed to create dir"));
}

#[test]
pub fn test_表示状態保存_ファイルが無ければ保存を許す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());

    // When
    let result = store.check_readable("wt");
    // Then
    assert!(result.is_ok());
}

#[test]
pub fn test_表示状態保存_正常なファイルなら保存を許す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    store.set("wt", make_state());
    store.save("wt").unwrap();
    let path = state_file(directory.path(), "wt");
    let before = std::fs::read(&path).unwrap();
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(result.is_ok());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
pub fn test_表示状態保存_正常に読めた後に破損したファイルの上書きを拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "wt");
    store.set("wt", make_state());
    store.save("wt").unwrap();
    store.check_readable("wt").unwrap();
    std::fs::write(&path, "broken").unwrap();
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(!result.is_ok());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
}

#[test]
pub fn test_表示状態保存_通常ファイルでない保存先を拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "wt");
    std::fs::create_dir_all(&path).unwrap();
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(!result.is_ok());
}

#[test]
pub fn test_表示状態保存_壊れた保存ファイルを変更せず拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "wt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "broken").unwrap();
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
}
