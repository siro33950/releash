use releashd::test_support::integration::platform::repository_impl_state_file as state_file;
use releashd::test_support::integration::platform::WorkspaceLayoutState;
use releashd::test_support::integration::platform::WorkspaceState;
use releashd::test_support::integration::platform::WorkspaceStateError;
use releashd::test_support::integration::platform::WorkspaceStateStore;
use releashd::test_support::integration::platform::WorkspaceTabEntry;
use releashd::test_support::integration::platform::WorkspaceTabsState;
use releashd::test_support::integration::repository::WorkspaceStateRepository;
use tempfile::TempDir;

fn make_state() -> WorkspaceState {
    WorkspaceState {
        panes: None,
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

#[test]
pub fn test_repositoryグループ_独立して保存し新しいstoreで復元する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    // When
    store.save_repository_group("/a/repo", true).unwrap();
    store.save_repository_group("/b/repo", false).unwrap();
    let restored = WorkspaceStateStore::new(directory.path().into());
    // Then
    assert!(restored.load_repository_group("/a/repo").unwrap());
    assert!(!restored.load_repository_group("/b/repo").unwrap());
    assert!(!restored.load_repository_group("/missing").unwrap());
}

#[test]
pub fn test_repository折りたたみ_壊れた保存値を既定値で隠さない() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    store.save_repository_group("/repo", true).unwrap();
    let file = std::fs::read_dir(dir.path().join("repository_group_state"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(file, "broken").unwrap();
    // When / Then
    assert!(store.load_repository_group("/repo").is_err());
}

#[test]
pub fn test_worktree表示状態_絶対pathの区切りとunderscoreが衝突しない() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    let mut one = make_state();
    one.layout.left_nav_collapsed = true;
    let two = make_state();
    // When
    store.set("/repo/a/b", one);
    store.save("/repo/a/b").unwrap();
    store.set("/repo/a_b", two);
    store.save("/repo/a_b").unwrap();
    let restored = WorkspaceStateStore::new(dir.path().to_path_buf());
    // Then
    assert_ne!(
        state_file(dir.path(), "/repo/a/b"),
        state_file(dir.path(), "/repo/a_b")
    );
    assert!(
        restored
            .load("/repo/a/b", "/repo/a/b")
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert!(
        !restored
            .load("/repo/a_b", "/repo/a_b")
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
}

#[test]
pub fn test_worktree表示状態_絶対pathの旧保存を読み新しい保存を優先する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().to_path_buf());
    store.set("_repo_wt", make_state());
    store.save("_repo_wt").unwrap();
    // When / Then
    assert!(store.load("/repo/wt", "/repo/wt").unwrap().is_some());
    let mut next = make_state();
    next.layout.left_nav_collapsed = true;
    store.set("/repo/wt", next);
    store.save("/repo/wt").unwrap();
    assert!(
        store
            .load("/repo/wt", "/repo/wt")
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    std::fs::write(state_file(dir.path(), "/repo/wt"), "broken").unwrap();
    assert!(store.load("/repo/wt", "/repo/wt").is_err());
}

#[test]
fn test_pane保存_異なるworktreeの分割と全タブを新storeで復元する() {
    use releashd::test_support::integration::platform::WorkspaceStateDto;
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().into());
    let mut expected = Vec::new();
    for (name, axis, ratio) in [("/repo/a", "horizontal", 0.3), ("/repo/b", "vertical", 0.7)] {
        let mut dto = WorkspaceStateDto::from(make_state());
        dto.tabs.editors.clear();
        dto.tabs.active_editor_path = None;
        let mut json = serde_json::to_value(dto).unwrap();
        json["panes"] = serde_json::json!({
            "kind":"split", "id":"root", "axis":axis, "ratio":ratio,
            "first":{"kind":"pane", "id":"a", "tabs":[{"id":"terminal", "kind":"terminal"}], "active_tab":"terminal"},
            "second":{"kind":"pane", "id":"b", "tabs":[{"id":"workflow", "kind":"workflow"},{"id":"file", "kind":"file"}], "active_tab":"file"}
        });
        let file = state_file(dir.path(), name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        let state = store.load(name, "/repo").unwrap().unwrap();
        state.panes.as_ref().unwrap().validate().unwrap();
        store.set(name, state.clone());
        store.save(name).unwrap();
        expected.push((name, state));
    }
    // When
    let restarted = WorkspaceStateStore::new(dir.path().into());
    // Then
    for (name, state) in expected {
        assert_eq!(restarted.load(name, "/repo").unwrap(), Some(state));
    }
}
