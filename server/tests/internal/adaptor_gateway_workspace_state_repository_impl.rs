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
    let result = store.migrate_legacy(&[], &["/repo/broken".into()]);
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
    let result = store.migrate_legacy(&[], &["/repo/broken".into()]);
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
    assert!(err.to_string().contains("storage"));
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
    let before = event_count(directory.path());
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(result.is_ok());
    assert_eq!(event_count(directory.path()), before);
}

#[test]
pub fn test_表示状態保存_正常に読めた後に破損したファイルの上書きを拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    store.set("wt", make_state());
    store.save("wt").unwrap();
    store.check_readable("wt").unwrap();
    corrupt_latest_event(directory.path());
    // When
    let result = store.check_readable("wt");
    // Then
    assert!(!result.is_ok());
    assert_eq!(event_payload(directory.path()), b"broken");
}

#[test]
pub fn test_表示状態保存_通常ファイルでない保存先を拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = state_file(directory.path(), "wt");
    std::fs::create_dir_all(&path).unwrap();
    // When
    let result = store.migrate_legacy(&[], &["/repo/wt".into()]);
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
    let result = store.migrate_legacy(&[], &["/repo/wt".into()]);
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
    drop(store);
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
    corrupt_latest_event(dir.path());
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
    drop(store);
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
    write_legacy_state(dir.path(), "_repo_wt", &legacy_state());
    store.migrate_legacy(&[], &["/repo/wt".into()]).unwrap();
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
    corrupt_latest_event(dir.path());
    assert!(store.load("/repo/wt", "/repo/wt").is_err());
}

#[test]
fn test_pane保存_異なるworktreeの分割と全タブを新storeで復元する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = WorkspaceStateStore::new(dir.path().into());
    let mut expected = Vec::new();
    for (name, axis, ratio) in [("/repo/a", "horizontal", 0.3), ("/repo/b", "vertical", 0.7)] {
        let mut json = serde_json::json!({
            "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
            "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": false, "rightCollapsed": false, "rightBottomCollapsed": false}
        });
        json["panes"] = serde_json::json!({
            "kind":"split", "id":"root", "axis":axis, "ratio":ratio,
            "first":{"kind":"pane", "id":"a", "tabs":[{"id":"terminal", "kind":"terminal"}], "active_tab":"terminal"},
            "second":{"kind":"pane", "id":"b", "tabs":[{"id":"workflow", "kind":"workflow"},{"id":"file", "kind":"file"}], "active_tab":"file"}
        });
        let file = state_file(dir.path(), name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
    }
    store
        .migrate_legacy(&[], &["/repo/a".into(), "/repo/b".into()])
        .unwrap();
    for name in ["/repo/a", "/repo/b"] {
        let state = store.load(name, "/repo").unwrap().unwrap();
        assert!(state.panes.is_some());
        store.set(name, state.clone());
        store.save(name).unwrap();
        expected.push((name, state));
    }
    // When
    drop(store);
    let restarted = WorkspaceStateStore::new(dir.path().into());
    // Then
    for (name, state) in expected {
        assert_eq!(restarted.load(name, "/repo").unwrap(), Some(state));
    }
}

fn legacy_state() -> serde_json::Value {
    serde_json::json!({"version":1,"tabs":{"editors":[],"activeEditorPath":null},"layout":{"centerTab":"agent","activeView":"git","leftNavCollapsed":false,"rightCollapsed":false,"rightBottomCollapsed":false}})
}
fn write_legacy_state(directory: &std::path::Path, name: &str, state: &serde_json::Value) {
    let path = state_file(directory, name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(state).unwrap()).unwrap();
}
fn event_connection(directory: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open(directory.join("local-event-store.sqlite3")).unwrap()
}
fn event_count(directory: &std::path::Path) -> i64 {
    event_connection(directory)
        .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
        .unwrap()
}
fn event_payload(directory: &std::path::Path) -> Vec<u8> {
    event_connection(directory)
        .query_row(
            "SELECT payload FROM events ORDER BY global_sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}
fn corrupt_latest_event(directory: &std::path::Path) {
    event_connection(directory).execute("UPDATE events SET payload = ?1 WHERE global_sequence = (SELECT max(global_sequence) FROM events)", [b"broken".as_slice()]).unwrap();
}

#[test]
fn test_旧表示状態_一意な名前だけ取り込み曖昧と不在の名前は使わない() {
    // Given
    let dir = TempDir::new().unwrap();
    for name in ["unique", "a", "missing"] {
        write_legacy_state(dir.path(), name, &legacy_state());
    }
    let store = WorkspaceStateStore::new(dir.path().into());
    let paths = vec![
        "/repo-worktrees/unique".into(),
        "/repo-worktrees/feature/a".into(),
        "/repo-worktrees/bugfix/a".into(),
    ];
    // When
    store.migrate_legacy(&[], &paths).unwrap();
    // Then
    assert!(store.load(&paths[0], &paths[0]).unwrap().is_some());
    for path in &paths[1..] {
        assert!(store.load(path, path).unwrap().is_none());
    }
    assert!(store.load("missing", "/repo").unwrap().is_none());
    assert_eq!(event_count(dir.path()), 1);
    store.migrate_legacy(&[], &paths).unwrap();
    assert_eq!(event_count(dir.path()), 1);
    assert!(dir.path().join("workspace_state/a.json").exists());
    assert!(dir.path().join("workspace_state/missing.json").exists());
    assert!(dir
        .path()
        .join("workspace_state.migrated/unique.json")
        .exists());
}

#[test]
fn test_旧表示状態_絶対pathを名前より優先し最新の事実を上書きしない() {
    // Given
    let dir = TempDir::new().unwrap();
    let mut state = legacy_state();
    write_legacy_state(dir.path(), "unique", &state);
    state["layout"]["leftNavCollapsed"] = serde_json::json!(true);
    write_legacy_state(dir.path(), "/repo/unique", &state);
    let store = WorkspaceStateStore::new(dir.path().into());
    // When
    store.migrate_legacy(&[], &["/repo/unique".into()]).unwrap();
    // Then
    assert!(
        store
            .load("/repo/unique", "/repo/unique")
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    let mut next = make_state();
    next.layout.left_nav_collapsed = false;
    store.set("/repo/unique", next);
    store.save("/repo/unique").unwrap();
    write_legacy_state(dir.path(), "/repo/unique", &state);
    store.migrate_legacy(&[], &["/repo/unique".into()]).unwrap();
    assert!(
        !store
            .load("/repo/unique", "/repo/unique")
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert_eq!(event_count(dir.path()), 2);
}

#[test]
fn test_旧repositoryグループ_取り込んだ折りたたみを再起動後に復元する() {
    // Given
    use sha2::Digest;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("repository_group_state");
    std::fs::create_dir_all(&path).unwrap();
    for (repo, collapsed) in [("/repo/a", true), ("/repo/b", false)] {
        let key = hex::encode(sha2::Sha256::digest(repo.as_bytes()));
        std::fs::write(path.join(format!("{key}.json")), collapsed.to_string()).unwrap();
    }
    let store = WorkspaceStateStore::new(dir.path().into());
    // When
    store
        .migrate_legacy(&["/repo/a".into(), "/repo/b".into()], &[])
        .unwrap();
    store.save_repository_group("/repo/a", false).unwrap();
    drop(store);
    let store = WorkspaceStateStore::new(dir.path().into());
    // Then
    assert!(!store.load_repository_group("/repo/a").unwrap());
    assert!(!store.load_repository_group("/repo/b").unwrap());
    assert_eq!(event_count(dir.path()), 3);
}

#[test]
fn test_旧表示状態の移行_途中失敗後に再実行しても取り込み済みの事実を増やさない() {
    // Given
    use sha2::Digest;
    let dir = TempDir::new().unwrap();
    write_legacy_state(dir.path(), "unique", &legacy_state());
    let group = dir.path().join("repository_group_state");
    std::fs::create_dir_all(&group).unwrap();
    let key = hex::encode(sha2::Sha256::digest(b"/repo"));
    let file = group.join(format!("{key}.json"));
    std::fs::write(&file, "broken").unwrap();
    let store = WorkspaceStateStore::new(dir.path().into());
    // When
    assert!(store
        .migrate_legacy(&["/repo".into()], &["/repo/unique".into()])
        .is_err());
    assert_eq!(event_count(dir.path()), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "broken");
    std::fs::write(file, "true").unwrap();
    store
        .migrate_legacy(&["/repo".into()], &["/repo/unique".into()])
        .unwrap();
    // Then
    assert!(store.load_repository_group("/repo").unwrap());
    assert!(store
        .load("/repo/unique", "/repo/unique")
        .unwrap()
        .is_some());
    assert_eq!(event_count(dir.path()), 2);
}

#[test]
fn test_旧表示状態_名前がhashキーのprefixと同じでも絶対pathの状態を優先する() {
    // Given
    let dir = TempDir::new().unwrap();
    let name = "worktree-feature";
    let path = format!("/repo/{name}");
    let mut state = legacy_state();
    write_legacy_state(dir.path(), name, &state);
    state["layout"]["leftNavCollapsed"] = serde_json::json!(true);
    write_legacy_state(dir.path(), &path, &state);
    let store = WorkspaceStateStore::new(dir.path().into());
    // When
    store
        .migrate_legacy(&[], std::slice::from_ref(&path))
        .unwrap();
    // Then
    assert!(
        store
            .load(&path, &path)
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert_eq!(event_count(dir.path()), 1);
}

#[test]
fn test_旧表示状態_曖昧な絶対path旧形式と名前を別pathへ取り込まない() {
    // Given
    let dir = TempDir::new().unwrap();
    for name in ["_repo_a_b", "_repo_target"] {
        write_legacy_state(dir.path(), name, &legacy_state());
    }
    let paths = vec![
        "/repo/a/b".into(),
        "/repo/a_b".into(),
        "/one/_repo_target".into(),
        "/two/_repo_target".into(),
        "/repo/target".into(),
    ];
    let store = WorkspaceStateStore::new(dir.path().into());
    // When
    store.migrate_legacy(&[], &paths).unwrap();
    // Then
    for path in &paths {
        assert!(store.load(path, path).unwrap().is_none());
    }
    assert_eq!(event_count(dir.path()), 0);
}

#[test]
fn test_配置の交互保存_reactの省略後もswiftの分割とタブを再起動時に復元する() {
    use releashd::test_support::integration::platform::{
        save_workspace_state, PaneLayout, PaneTab, PaneTabKind, SplitAxis,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let key = "/repo-worktrees/feature/a";
    let panes = PaneLayout::split(
        "split".into(),
        SplitAxis::Vertical,
        0.7,
        PaneLayout::pane(
            "left".into(),
            vec![PaneTab {
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
            }],
            Some("terminal".into()),
        )
        .unwrap(),
        PaneLayout::pane(
            "right".into(),
            vec![
                PaneTab {
                    id: "workflow".into(),
                    kind: PaneTabKind::Workflow,
                },
                PaneTab {
                    id: "file".into(),
                    kind: PaneTabKind::File,
                },
                PaneTab {
                    id: "session".into(),
                    kind: PaneTabKind::Session,
                },
            ],
            Some("file".into()),
        )
        .unwrap(),
    )
    .unwrap();
    let mut swift = make_state();
    swift.panes = Some(panes.clone());
    save_workspace_state(&store, None, key, swift).unwrap();
    let mut react = make_state();
    react.layout.left_nav_collapsed = true;
    react.tabs.editors.clear();
    react.tabs.active_editor_path = None;
    // When
    save_workspace_state(&store, None, key, react.clone()).unwrap();
    // Then
    let saved = store.load(key, key).unwrap().unwrap();
    assert_eq!(saved.panes, Some(panes.clone()));
    assert_eq!(saved.tabs, react.tabs);
    assert_eq!(saved.layout, react.layout);
    drop(store);
    let restarted = WorkspaceStateStore::new(directory.path().into());
    assert_eq!(restarted.load(key, key).unwrap().unwrap(), saved);
    let mut changed = react.clone();
    changed.panes = Some(PaneLayout::pane("all-closed".into(), vec![], None).unwrap());
    save_workspace_state(&restarted, None, key, changed.clone()).unwrap();
    save_workspace_state(&restarted, None, key, react).unwrap();
    assert_eq!(
        restarted.load(key, key).unwrap().unwrap().panes,
        changed.panes
    );
}

#[test]
fn test_旧状態移行_破損した対象を残して同じディレクトリの正常な対象を取り込む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    write_legacy_state(directory.path(), "healthy", &legacy_state());
    let broken = state_file(directory.path(), "broken");
    std::fs::write(&broken, "broken").unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    // When
    assert!(store
        .migrate_legacy(&[], &["/repo/healthy".into(), "/repo/broken".into()])
        .is_err());
    // Then
    assert!(store
        .load("/repo/healthy", "/repo/healthy")
        .unwrap()
        .is_some());
    assert_eq!(std::fs::read_to_string(&broken).unwrap(), "broken");
    assert!(directory
        .path()
        .join("workspace_state.migrated/healthy.json")
        .exists());
    std::fs::write(&broken, serde_json::to_vec(&legacy_state()).unwrap()).unwrap();
    store
        .migrate_legacy(&[], &["/repo/healthy".into(), "/repo/broken".into()])
        .unwrap();
    assert!(store
        .load("/repo/broken", "/repo/broken")
        .unwrap()
        .is_some());
    assert_eq!(event_count(directory.path()), 2);
}

#[test]
fn test_ui状態の無効化_走査後に更新された状態を消さず不正なkeyを拒否する() {
    use releashd::test_support::integration::persistence::WorkspaceStateGc;
    use releashd::test_support::integration::platform::storage_key;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = WorkspaceStateStore::new(directory.path().into());
    let path = "/repo/removed";
    store.set(path, make_state());
    store.save(path).unwrap();
    let inventory = store.states().unwrap();
    assert_eq!(inventory.len(), 1);
    let (key, version) = &inventory[0];
    assert_eq!(key, &storage_key(path));
    let mut next = make_state();
    next.layout.left_nav_collapsed = true;
    store.set(path, next);
    store.save(path).unwrap();
    // When / Then
    assert!(!store.invalidate(key, *version).unwrap());
    assert!(
        store
            .load(path, path)
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert!(store.invalidate("unexpected", *version).is_err());
    assert_eq!(event_count(directory.path()), 2);
}
