use releashd::test_support::integration::platform::repository_impl_state_file as state_file;
use releashd::test_support::integration::platform::WorkspaceLayoutState;
use releashd::test_support::integration::platform::WorkspaceState;
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

#[tokio::test]
pub async fn test_表示状態読取_破損を未保存と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = state_file(directory.path(), "broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "broken json").unwrap();
    // When
    let result = store.migrate_legacy(&[], &["/repo/broken".into()]).await;
    // Then
    assert!(result.is_err());
}

#[tokio::test]
pub async fn test_表示状態読取_io失敗を未保存と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = state_file(directory.path(), "broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::create_dir(&path).unwrap();
    // When
    let result = store.migrate_legacy(&[], &["/repo/broken".into()]).await;
    // Then
    assert!(result.is_err());
}

#[tokio::test]
pub async fn save_and_load_roundtrip() {
    // Given
    let dir = TempDir::new().unwrap();
    let worktree_dir = dir.path().join("worktree");
    std::fs::create_dir_all(worktree_dir.join("src")).unwrap();
    std::fs::write(worktree_dir.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(worktree_dir.join("src/lib.rs"), "// lib").unwrap();

    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    let pending_state = make_state();
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "wt1",
        pending_state,
    )
    .await
    .unwrap();

    // When
    let loaded = store
        .load("wt1", worktree_dir.to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.tabs.editors.len(), 2);
    assert_eq!(loaded.layout.center_tab, "editor");
}

#[tokio::test]
pub async fn load_nonexistent_returns_none() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    // When
    let loaded = store.load("nonexistent", "/tmp").await.unwrap();
    // Then
    assert!(loaded.is_none());
}

#[tokio::test]
pub async fn get_set_in_memory() {
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    assert!(store.read("wt1").await.unwrap().1.is_none());
    let pending_state = make_state();
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "wt1",
        pending_state,
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .read("wt1")
            .await
            .unwrap()
            .1
            .unwrap()
            .tabs
            .editors
            .len(),
        2
    );
}

#[tokio::test]
pub async fn save_returns_workspace_state_error_when_state_dir_cannot_be_created() {
    let dir = TempDir::new().unwrap();
    let app_data_file = dir.path().join("app-data");
    std::fs::write(&app_data_file, "not a directory").unwrap();
    use releashd::test_support::integration::persistence::{
        LocalEventStore, LocalEventStoreConfig,
    };
    let result = LocalEventStore::open(LocalEventStoreConfig::production(
        app_data_file,
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::default()),
    ));
    assert!(result.is_err());
}

#[tokio::test]
pub async fn test_表示状態保存_ファイルが無ければ保存を許す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );

    // When
    let result = store.read("wt").await;
    // Then
    assert!(result.is_ok());
}

#[tokio::test]
pub async fn test_表示状態保存_正常なファイルなら保存を許す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let pending_state = make_state();
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "wt",
        pending_state,
    )
    .await
    .unwrap();
    let before = event_count(directory.path());
    // When
    let result = store.read("wt").await;
    // Then
    assert!(result.is_ok());
    assert_eq!(event_count(directory.path()), before);
}

#[tokio::test]
pub async fn test_表示状態保存_正常に読めた後に破損したファイルの上書きを拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let pending_state = make_state();
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "wt",
        pending_state,
    )
    .await
    .unwrap();
    store.read("wt").await.unwrap();
    corrupt_latest_event(directory.path());
    // When
    let result = store.read("wt").await;
    // Then
    assert!(!result.is_ok());
    assert_eq!(event_payload(directory.path()), b"broken");
}

#[tokio::test]
pub async fn test_表示状態保存_通常ファイルでない保存先を拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = state_file(directory.path(), "wt");
    std::fs::create_dir_all(&path).unwrap();
    // When
    let result = store.migrate_legacy(&[], &["/repo/wt".into()]).await;
    // Then
    assert!(!result.is_ok());
}

#[tokio::test]
pub async fn test_表示状態保存_壊れた保存ファイルを変更せず拒む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = state_file(directory.path(), "wt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "broken").unwrap();
    // When
    let result = store.migrate_legacy(&[], &["/repo/wt".into()]).await;
    // Then
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
}

#[tokio::test]
pub async fn test_repositoryグループ_独立して保存し新しいstoreで復元する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    // When
    store
        .save_repository_group(
            "/a/repo",
            true,
            store.read_repository_group("/a/repo").await.unwrap().0,
        )
        .await
        .unwrap();
    store
        .save_repository_group(
            "/b/repo",
            false,
            store.read_repository_group("/b/repo").await.unwrap().0,
        )
        .await
        .unwrap();
    drop(store);
    let restored = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    // Then
    assert!(restored.load_repository_group("/a/repo").await.unwrap());
    assert!(!restored.load_repository_group("/b/repo").await.unwrap());
    assert!(!restored.load_repository_group("/missing").await.unwrap());
}

#[tokio::test]
pub async fn test_repository折りたたみ_壊れた保存値を既定値で隠さない() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    store
        .save_repository_group(
            "/repo",
            true,
            store.read_repository_group("/repo").await.unwrap().0,
        )
        .await
        .unwrap();
    corrupt_latest_event(dir.path());
    // When / Then
    assert!(store.load_repository_group("/repo").await.is_err());
}

#[tokio::test]
pub async fn test_worktree表示状態_絶対pathの区切りとunderscoreが衝突しない() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    let mut one = make_state();
    one.layout.left_nav_collapsed = true;
    let two = make_state();
    // When
    let pending_state = one;
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "/repo/a/b",
        pending_state,
    )
    .await
    .unwrap();
    let pending_state = two;
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "/repo/a_b",
        pending_state,
    )
    .await
    .unwrap();
    drop(store);
    let restored = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    // Then
    assert_ne!(
        state_file(dir.path(), "/repo/a/b"),
        state_file(dir.path(), "/repo/a_b")
    );
    assert!(
        restored
            .load("/repo/a/b", "/repo/a/b")
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert!(
        !restored
            .load("/repo/a_b", "/repo/a_b")
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
}

#[tokio::test]
pub async fn test_worktree表示状態_絶対pathの旧保存を読み新しい保存を優先する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    write_legacy_state(dir.path(), "/repo/wt", &legacy_state());
    store
        .migrate_legacy(&[], &["/repo/wt".into()])
        .await
        .unwrap();
    // When / Then
    assert!(store.load("/repo/wt", "/repo/wt").await.unwrap().is_some());
    let mut next = make_state();
    next.layout.left_nav_collapsed = true;
    let pending_state = next;
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "/repo/wt",
        pending_state,
    )
    .await
    .unwrap();
    assert!(
        store
            .load("/repo/wt", "/repo/wt")
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    corrupt_latest_event(dir.path());
    assert!(store.load("/repo/wt", "/repo/wt").await.is_err());
}

#[tokio::test]
async fn test_pane保存_異なるworktreeの分割と全タブを新storeで復元する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    let mut expected = Vec::new();
    for (name, axis, ratio) in [("/repo/a", "horizontal", 0.3), ("/repo/b", "vertical", 0.7)] {
        let mut json = serde_json::json!({
            "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
            "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": false, "rightCollapsed": false, "rightBottomCollapsed": false}
        });
        json["panes"] = serde_json::json!({
            "kind":"split", "id":"root", "axis":axis, "ratio":ratio,
            "first":{"kind":"pane", "id":"a", "tabs":[{"id":"terminal", "kind":"terminal"}], "active_tab":"terminal"},
            "second":{"kind":"pane", "id":"b", "tabs":[{"id":"workflow", "kind":"workflow"},{"id":"file", "kind":"file", "file_path":"src/file.rs", "file_mode":"diff", "file_base":"branch-base"}], "active_tab":"file"}
        });
        let file = state_file(dir.path(), name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
    }
    store
        .migrate_legacy(&[], &["/repo/a".into(), "/repo/b".into()])
        .await
        .unwrap();
    for name in ["/repo/a", "/repo/b"] {
        let state = store.load(name, "/repo").await.unwrap().unwrap();
        assert!(state.panes.is_some());
        let pending_state = state.clone();
        releashd::test_support::integration::platform::save_workspace_state(
            &store,
            None,
            name,
            pending_state,
        )
        .await
        .unwrap();
        expected.push((name, state));
    }
    // When
    drop(store);
    let restarted =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // Then
    for (name, state) in expected {
        assert_eq!(restarted.load(name, "/repo").await.unwrap(), Some(state));
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

#[tokio::test]
async fn test_旧表示状態_一意な名前だけ取り込み曖昧と不在の名前は使わない() {
    // Given
    let dir = TempDir::new().unwrap();
    for name in ["unique", "a", "missing"] {
        write_legacy_state(dir.path(), name, &legacy_state());
    }
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    let paths = vec![
        "/repo-worktrees/unique".into(),
        "/repo-worktrees/feature/a".into(),
        "/repo-worktrees/bugfix/a".into(),
    ];
    // When
    store.migrate_legacy(&[], &paths).await.unwrap();
    // Then
    assert!(store.load(&paths[0], &paths[0]).await.unwrap().is_some());
    for path in &paths[1..] {
        assert!(store.load(path, path).await.unwrap().is_none());
    }
    assert!(store.load("missing", "/repo").await.unwrap().is_none());
    assert_eq!(event_count(dir.path()), 1);
    store.migrate_legacy(&[], &paths).await.unwrap();
    assert_eq!(event_count(dir.path()), 1);
    assert!(dir.path().join("workspace_state/a.json").exists());
    assert!(dir.path().join("workspace_state/missing.json").exists());
    assert!(dir
        .path()
        .join("workspace_state.migrated/unique.json")
        .exists());
}

#[tokio::test]
async fn test_旧表示状態_絶対pathを名前より優先し最新の事実を上書きしない() {
    // Given
    let dir = TempDir::new().unwrap();
    let mut state = legacy_state();
    write_legacy_state(dir.path(), "unique", &state);
    state["layout"]["leftNavCollapsed"] = serde_json::json!(true);
    write_legacy_state(dir.path(), "/repo/unique", &state);
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // When
    store
        .migrate_legacy(&[], &["/repo/unique".into()])
        .await
        .unwrap();
    // Then
    assert!(
        store
            .load("/repo/unique", "/repo/unique")
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    let mut next = make_state();
    next.layout.left_nav_collapsed = false;
    let pending_state = next;
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        "/repo/unique",
        pending_state,
    )
    .await
    .unwrap();
    write_legacy_state(dir.path(), "/repo/unique", &state);
    store
        .migrate_legacy(&[], &["/repo/unique".into()])
        .await
        .unwrap();
    assert!(
        !store
            .load("/repo/unique", "/repo/unique")
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert_eq!(event_count(dir.path()), 2);
}

#[tokio::test]
async fn test_旧repositoryグループ_取り込んだ折りたたみを再起動後に復元する() {
    // Given
    use sha2::Digest;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("repository_group_state");
    std::fs::create_dir_all(&path).unwrap();
    for (repo, collapsed) in [("/repo/a", true), ("/repo/b", false)] {
        let key = hex::encode(sha2::Sha256::digest(repo.as_bytes()));
        std::fs::write(path.join(format!("{key}.json")), collapsed.to_string()).unwrap();
    }
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // When
    store
        .migrate_legacy(&["/repo/a".into(), "/repo/b".into()], &[])
        .await
        .unwrap();
    store
        .save_repository_group(
            "/repo/a",
            false,
            store.read_repository_group("/repo/a").await.unwrap().0,
        )
        .await
        .unwrap();
    drop(store);
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // Then
    assert!(!store.load_repository_group("/repo/a").await.unwrap());
    assert!(!store.load_repository_group("/repo/b").await.unwrap());
    assert_eq!(event_count(dir.path()), 3);
}

#[tokio::test]
async fn test_旧表示状態の移行_途中失敗後に再実行しても取り込み済みの事実を増やさない() {
    // Given
    use sha2::Digest;
    let dir = TempDir::new().unwrap();
    write_legacy_state(dir.path(), "unique", &legacy_state());
    let group = dir.path().join("repository_group_state");
    std::fs::create_dir_all(&group).unwrap();
    let key = hex::encode(sha2::Sha256::digest(b"/repo"));
    let file = group.join(format!("{key}.json"));
    std::fs::write(&file, "broken").unwrap();
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // When
    assert!(store
        .migrate_legacy(&["/repo".into()], &["/repo/unique".into()])
        .await
        .is_err());
    assert_eq!(event_count(dir.path()), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "broken");
    std::fs::write(file, "true").unwrap();
    store
        .migrate_legacy(&["/repo".into()], &["/repo/unique".into()])
        .await
        .unwrap();
    // Then
    assert!(store.load_repository_group("/repo").await.unwrap());
    assert!(store
        .load("/repo/unique", "/repo/unique")
        .await
        .unwrap()
        .is_some());
    assert_eq!(event_count(dir.path()), 2);
}

#[tokio::test]
async fn test_旧表示状態_名前がhashキーのprefixと同じでも絶対pathの状態を優先する() {
    // Given
    let dir = TempDir::new().unwrap();
    let name = "worktree-feature";
    let path = format!("/repo/{name}");
    let mut state = legacy_state();
    write_legacy_state(dir.path(), name, &state);
    state["layout"]["leftNavCollapsed"] = serde_json::json!(true);
    write_legacy_state(dir.path(), &path, &state);
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // When
    store
        .migrate_legacy(&[], std::slice::from_ref(&path))
        .await
        .unwrap();
    // Then
    assert!(
        store
            .load(&path, &path)
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert_eq!(event_count(dir.path()), 1);
}

#[tokio::test]
async fn test_旧表示状態_曖昧な絶対path旧形式と名前を別pathへ取り込まない() {
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
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    // When
    store.migrate_legacy(&[], &paths).await.unwrap();
    // Then
    for path in &paths {
        assert!(store.load(path, path).await.unwrap().is_none());
    }
    assert_eq!(event_count(dir.path()), 0);
}

#[tokio::test]
async fn test_配置の交互保存_reactの省略後もswiftの分割とタブを再起動時に復元する() {
    use releashd::test_support::integration::platform::{
        save_workspace_state, PaneLayout, PaneTab, PaneTabKind, SplitAxis,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let key = "/repo-worktrees/feature/a";
    let panes = PaneLayout::split(
        "split".into(),
        SplitAxis::Vertical,
        0.7,
        PaneLayout::pane(
            "left".into(),
            vec![PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
                file_path: None,
                file_mode: None,
                file_base: None,
            }],
            Some("terminal".into()),
        )
        .unwrap(),
        PaneLayout::pane(
            "right".into(),
            vec![
                PaneTab {
                    agent_session_id: None,
                    node_id: None,
                    id: "workflow".into(),
                    kind: PaneTabKind::Command,
                    file_path: None,
                    file_mode: None,
                    file_base: None,
                },
                PaneTab {
                    agent_session_id: None,
                    node_id: None,
                    id: "file".into(),
                    kind: PaneTabKind::File,
                    file_path: None,
                    file_mode: None,
                    file_base: None,
                },
                PaneTab {
                    agent_session_id: Some("agent-session".into()),
                    node_id: None,
                    id: "session".into(),
                    kind: PaneTabKind::Session,
                    file_path: None,
                    file_mode: None,
                    file_base: None,
                },
                PaneTab {
                    agent_session_id: None,
                    node_id: Some("command-node".into()),
                    id: "command".into(),
                    kind: PaneTabKind::Command,
                    file_path: None,
                    file_mode: None,
                    file_base: None,
                },
            ],
            Some("file".into()),
        )
        .unwrap(),
    )
    .unwrap();
    let mut swift = make_state();
    swift.panes = Some(panes.clone());
    save_workspace_state(&store, None, key, swift)
        .await
        .unwrap();
    let mut react = make_state();
    react.layout.left_nav_collapsed = true;
    react.tabs.editors.clear();
    react.tabs.active_editor_path = None;
    // When
    save_workspace_state(&store, None, key, react.clone())
        .await
        .unwrap();
    // Then
    let saved = store.load(key, key).await.unwrap().unwrap();
    assert_eq!(saved.panes, Some(panes.clone()));
    assert_eq!(saved.tabs, react.tabs);
    assert_eq!(saved.layout, react.layout);
    drop(store);
    let restarted = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    assert_eq!(restarted.load(key, key).await.unwrap().unwrap(), saved);
    let mut changed = react.clone();
    changed.panes = Some(PaneLayout::pane("all-closed".into(), vec![], None).unwrap());
    save_workspace_state(&restarted, None, key, changed.clone())
        .await
        .unwrap();
    save_workspace_state(&restarted, None, key, react)
        .await
        .unwrap();
    assert_eq!(
        restarted.load(key, key).await.unwrap().unwrap().panes,
        changed.panes
    );
}

#[tokio::test]
async fn test_旧状態移行_破損した対象を残して同じディレクトリの正常な対象を取り込む() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    write_legacy_state(directory.path(), "healthy", &legacy_state());
    let broken = state_file(directory.path(), "broken");
    std::fs::write(&broken, "broken").unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    // When
    assert!(store
        .migrate_legacy(&[], &["/repo/healthy".into(), "/repo/broken".into()])
        .await
        .is_err());
    // Then
    assert!(store
        .load("/repo/healthy", "/repo/healthy")
        .await
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
        .await
        .unwrap();
    assert!(store
        .load("/repo/broken", "/repo/broken")
        .await
        .unwrap()
        .is_some());
    assert_eq!(event_count(directory.path()), 2);
}

#[tokio::test]
async fn test_ui状態の無効化_走査後に更新された状態を消さず不正なkeyを拒否する() {
    use releashd::test_support::integration::platform::storage_key;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = "/repo/removed";
    let pending_state = make_state();
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        path,
        pending_state,
    )
    .await
    .unwrap();
    let inventory = store.states().await.unwrap();
    assert_eq!(inventory.len(), 1);
    let (key, version) = &inventory[0];
    assert_eq!(
        key,
        &releashd::test_support::integration::platform::WorkspaceStateKey::for_test(storage_key(
            path
        ))
    );
    let mut next = make_state();
    next.layout.left_nav_collapsed = true;
    let pending_state = next;
    releashd::test_support::integration::platform::save_workspace_state(
        &store,
        None,
        path,
        pending_state,
    )
    .await
    .unwrap();
    // When / Then
    assert!(!store.invalidate(key, *version).await.unwrap());
    assert!(
        store
            .load(path, path)
            .await
            .unwrap()
            .unwrap()
            .layout
            .left_nav_collapsed
    );
    assert!(store
        .invalidate(
            &releashd::test_support::integration::platform::WorkspaceStateKey::for_test(
                "unexpected"
            ),
            *version
        )
        .await
        .is_err());
    assert_eq!(event_count(directory.path()), 2);
}

#[tokio::test]
async fn test_旧状態の再取り込み_移行先があれば元jsonと空directoryを除去し事実を増やさない() {
    use sha2::Digest;
    // Given
    let directory = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let group_name = hex::encode(sha2::Sha256::digest(b"/repo"));
    let group = directory.path().join("repository_group_state");
    std::fs::create_dir(&group).unwrap();
    std::fs::write(group.join(format!("{group_name}.json")), "true").unwrap();
    write_legacy_state(directory.path(), "wt", &legacy_state());
    store
        .migrate_legacy(&["/repo".into()], &["/repo/wt".into()])
        .await
        .unwrap();
    let count = event_count(directory.path());
    let first_state = store.read("/repo/wt").await.unwrap();
    let migrated_group = directory
        .path()
        .join("repository_group_state.migrated")
        .join(format!("{group_name}.json"));
    // When
    std::fs::create_dir(&group).unwrap();
    std::fs::write(group.join(format!("{group_name}.json")), "false").unwrap();
    let mut next = legacy_state();
    next["layout"]["leftNavCollapsed"] = serde_json::json!(true);
    write_legacy_state(directory.path(), "wt", &next);
    store
        .migrate_legacy(&["/repo".into()], &["/repo/wt".into()])
        .await
        .unwrap();
    // Then
    assert!(!group.exists());
    assert!(!directory.path().join("workspace_state").exists());
    assert_eq!(event_count(directory.path()), count);
    assert_eq!(store.read("/repo/wt").await.unwrap(), first_state);
    assert!(store.load_repository_group("/repo").await.unwrap());
    assert_eq!(std::fs::read_to_string(migrated_group).unwrap(), "true");
}

#[tokio::test]
async fn test_表示状態の版競合_古い版で保存せず最新の状態と版を保持する() {
    use releashd::test_support::integration::platform::save_workspace_state;
    // Given
    let directory = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let path = "/repo/wt";
    let (version, _) = store.read(path).await.unwrap();
    let mut winner = make_state();
    winner.layout.left_nav_collapsed = true;
    save_workspace_state(&store, None, path, winner.clone())
        .await
        .unwrap();
    // When
    assert!(store.save(path, make_state(), version).await.is_err());
    // Then
    let (latest, state) = store.read(path).await.unwrap();
    assert_eq!(state, Some(winner));
    assert_eq!(latest.value(), 1);
    let inventory = store.states().await.unwrap();
    assert_eq!(inventory[0].1, latest);
    assert!(store.invalidate(&inventory[0].0, latest).await.unwrap());
    assert!(store.states().await.unwrap().is_empty());
    assert!(store.read(path).await.unwrap().1.is_none());
}

fn blocking_fact() -> releashd::test_support::integration::persistence::NewNodeEventRow {
    releashd::test_support::integration::persistence::NewNodeEventRow {
        tree_id: "tree".into(),
        node_execution_id: "node".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: "session".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: None,
        detail: "{}".into(),
    }
}

async fn wait_for_ui_writes(
    store: &releashd::test_support::integration::persistence::LocalEventStore,
    count: usize,
) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while store.pending_write_request_count() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ui状態の並行保存_版競合を再読取し直前のpanesと後続の状態を復元する() {
    use releashd::test_support::integration::persistence::{
        LocalEventStore, LocalEventStoreConfig,
    };
    use releashd::test_support::integration::platform::{
        save_repository_group_state, save_workspace_state, PaneLayout, RetryLimiter,
        WorkspaceStateStore,
    };
    use std::sync::Arc;
    for group in [false, true] {
        // Given
        let dir = TempDir::new().unwrap();
        let events = LocalEventStore::open(LocalEventStoreConfig::production(
            dir.path().into(),
            Arc::new(RetryLimiter::new()),
        ))
        .unwrap();
        let store = Arc::new(WorkspaceStateStore::with_store(
            dir.path().into(),
            events.clone(),
        ));
        let stall = events.fault_injector().arm_node_event_append_stall();
        let append = events.append_node_event(blocking_fact(), None);
        tokio::pin!(append);
        assert!(futures_util::poll!(&mut append).is_pending());
        stall.wait_until_arrived();
        let panes = PaneLayout::pane("saved-pane".into(), vec![], None).unwrap();
        let first_store = store.clone();
        let first_panes = panes.clone();
        let first = tokio::spawn(async move {
            if group {
                save_repository_group_state(first_store.as_ref(), None, "/repo", true).await
            } else {
                let mut state = make_state();
                state.panes = Some(first_panes);
                save_workspace_state(first_store.as_ref(), None, "/repo/wt", state.clone()).await
            }
        });
        wait_for_ui_writes(events.as_ref(), 1).await;
        let second_store = store.clone();
        let second = tokio::spawn(async move {
            if group {
                save_repository_group_state(second_store.as_ref(), None, "/repo", false).await
            } else {
                let mut state = make_state();
                state.layout.left_nav_collapsed = true;
                save_workspace_state(second_store.as_ref(), None, "/repo/wt", state.clone()).await
            }
        });
        wait_for_ui_writes(events.as_ref(), 2).await;
        // When
        stall.release();
        append.await.unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        drop(store);
        let restored = WorkspaceStateStore::with_store(dir.path().into(), events.clone());
        // Then
        if group {
            assert!(!restored.load_repository_group("/repo").await.unwrap());
        } else {
            let (version, state) = restored.read("/repo/wt").await.unwrap();
            assert_eq!(version.value(), 2);
            let state = state.unwrap();
            assert!(state.layout.left_nav_collapsed);
            assert_eq!(state.panes, Some(panes));
        }
        assert_eq!(event_count(dir.path()), 2);
    }
}

#[tokio::test]
async fn test_ui状態保存_handler破棄後も受理済みjobが完了するまで削除を待つ() {
    use releashd::test_support::integration::persistence::{
        LocalEventStore, LocalEventStoreConfig,
    };
    use releashd::test_support::integration::platform::{
        save_workspace_state, RetryLimiter, WorkspaceStateStore, WorktreeOperations,
    };
    use releashd::test_support::integration::transport::scope;
    use std::sync::Arc;
    // Given
    let dir = TempDir::new().unwrap();
    let events = LocalEventStore::open(LocalEventStoreConfig::production(
        dir.path().into(),
        Arc::new(RetryLimiter::new()),
    ))
    .unwrap();
    let store = Arc::new(WorkspaceStateStore::with_store(
        dir.path().into(),
        events.clone(),
    ));
    let stall = events.fault_injector().arm_node_event_append_stall();
    let append = events.append_node_event(blocking_fact(), None);
    tokio::pin!(append);
    assert!(futures_util::poll!(&mut append).is_pending());
    stall.wait_until_arrived();
    let operations = WorktreeOperations::default();
    let guard = operations.mutate("/repo/wt").unwrap();
    let handler_store = store.clone();
    let handler = tokio::spawn(scope(vec![guard], async move {
        save_workspace_state(handler_store.as_ref(), None, "/repo/wt", make_state()).await
    }));
    wait_for_ui_writes(events.as_ref(), 1).await;
    // When
    handler.abort();
    assert!(handler.await.unwrap_err().is_cancelled());
    let mut deletion = Box::pin(operations.delete("/repo/wt"));
    // Then
    assert!(futures_util::poll!(&mut deletion).is_pending());
    assert!(operations.mutate("/repo/wt").is_err());
    stall.release();
    append.await.unwrap();
    drop(
        tokio::time::timeout(std::time::Duration::from_secs(2), deletion)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(store.read("/repo/wt").await.unwrap().1.is_some());
    assert!(operations.mutate("/repo/wt").is_ok());
}

#[tokio::test]
async fn test_ui状態読取_最新の未知の版と種類を未保存でなく非互換として返す() {
    for group in [false, true] {
        for unknown_version in [false, true] {
            // Given
            let dir = TempDir::new().unwrap();
            let store = releashd::test_support::integration::platform::workspace_state_store(
                dir.path().into(),
            );
            if group {
                store
                    .save_repository_group(
                        "/repo",
                        true,
                        store.read_repository_group("/repo").await.unwrap().0,
                    )
                    .await
                    .unwrap();
            } else {
                let (version, _) = store.read("/repo/wt").await.unwrap();
                store.save("/repo/wt", make_state(), version).await.unwrap();
            }
            let connection = event_connection(dir.path());
            if unknown_version {
                connection
                    .execute("UPDATE events SET payload_version = 999", [])
                    .unwrap();
            } else {
                connection
                    .execute("UPDATE events SET event_type = 'unknown.event'", [])
                    .unwrap();
            }
            // When / Then
            let error = if group {
                store.load_repository_group("/repo").await.unwrap_err()
            } else {
                let error = store.read("/repo/wt").await.unwrap_err();
                assert!(error.to_string().contains("incompatible stored event"));
                store.load("/repo/wt", "/repo/wt").await.unwrap_err()
            };
            assert!(error.to_string().contains("incompatible stored event"));
        }
    }
}

#[tokio::test]
async fn test_ui状態読取_hash破損はstreamとcorrelationをログに残す() {
    use releashd::test_support::integration::platform::{
        captured_error_messages, install_capturing_logger,
    };
    // Given
    install_capturing_logger();
    let dir = TempDir::new().unwrap();
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    let path = format!("/repo/{}", uuid::Uuid::new_v4());
    let (version, _) = store.read(&path).await.unwrap();
    store.save(&path, make_state(), version).await.unwrap();
    corrupt_latest_event(dir.path());
    let stream: String = event_connection(dir.path())
        .query_row("SELECT stream_id FROM events", [], |row| row.get(0))
        .unwrap();
    // When
    let error = store.read(&path).await.unwrap_err().to_string();
    // Then
    assert!(error.contains("store corrupt"));
    let correlation = error
        .split("correlation_id=")
        .nth(1)
        .unwrap()
        .trim_end_matches(')');
    assert!(captured_error_messages()
        .iter()
        .any(|message| message.contains(&stream)
            && message.contains(correlation)
            && message.contains("sequence=1")
            && message.contains("workspace.state_changed")
            && message.contains("hash mismatch")));
}

#[tokio::test]
async fn test_repositoryグループ保存_読み取った版の後に更新があれば競合する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    let (version, _) = store.read_repository_group("/repo").await.unwrap();
    store
        .save_repository_group("/repo", true, version)
        .await
        .unwrap();
    // When
    let result = store.save_repository_group("/repo", false, version).await;
    // Then
    assert!(matches!(
        result,
        Err(releashd::test_support::integration::platform::WorkspaceStateError::VersionConflict)
    ));
    assert!(store.load_repository_group("/repo").await.unwrap());
    assert_eq!(event_count(dir.path()), 1);
}

#[tokio::test]
async fn test_ui状態保存_同じ値の連続保存も毎回事実を追記する() {
    // Given
    let dir = TempDir::new().unwrap();
    let store =
        releashd::test_support::integration::platform::workspace_state_store(dir.path().into());
    let state = make_state();
    // When
    for _ in 0..2 {
        let version = store.read("/repo/wt").await.unwrap().0;
        store
            .save("/repo/wt", state.clone(), version)
            .await
            .unwrap();
    }
    // Then
    assert_eq!(event_count(dir.path()), 2);
    assert_eq!(store.read("/repo/wt").await.unwrap().1, Some(state));
}

#[tokio::test]
async fn test_表示状態load_相対pathとroot外の絶対pathで消失したタブだけ除く() {
    // Given
    let directory = TempDir::new().unwrap();
    let root = directory.path().join("worktree");
    let outside = directory.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let paths = [
        root.join("kept"),
        root.join("removed"),
        outside.join("kept"),
        outside.join("removed"),
    ];
    for path in &paths {
        std::fs::write(path, "file").unwrap();
    }
    let entries = [
        "kept".into(),
        "removed".into(),
        paths[2].to_string_lossy().into_owned(),
        paths[3].to_string_lossy().into_owned(),
    ];
    let mut state = make_state();
    state.tabs.editors = entries
        .iter()
        .map(|path| WorkspaceTabEntry {
            path: path.clone(),
            name: path.clone(),
        })
        .collect();
    state.tabs.active_editor_path = Some(entries[0].clone());
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    store
        .save(
            "worktree",
            state,
            releashd::test_support::integration::platform::StreamVersion::new(0).unwrap(),
        )
        .await
        .unwrap();
    std::fs::remove_file(&paths[1]).unwrap();
    std::fs::remove_file(&paths[3]).unwrap();
    // When
    let loaded = store
        .load("worktree", root.to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        loaded
            .tabs
            .editors
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        [entries[0].as_str(), entries[2].as_str()]
    );
}

#[tokio::test]
async fn test_ui状態gateway_両方の読取と保存で停止の種別を保持する() {
    use releashd::test_support::integration::platform::{
        scope, Deadline, OperationContext, StreamVersion, TechnicalFailureNature,
        WorkspaceStateError,
    };
    use std::sync::Arc;
    // Given
    let directory = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    for cancelled in [false, true] {
        for operation in 0..4 {
            let token = Arc::new(tokio_util::sync::CancellationToken::new());
            let deadline = if cancelled {
                token.cancel();
                None
            } else {
                Some(Deadline::new(std::time::Instant::now()))
            };
            // When
            let result = scope(OperationContext::new(deadline, token), async {
                match operation {
                    0 => store.read("/repo/wt").await.map(|_| ()),
                    1 => store.read_repository_group("/repo").await.map(|_| ()),
                    2 => {
                        store
                            .save("/repo/wt", make_state(), StreamVersion::new(0).unwrap())
                            .await
                    }
                    _ => {
                        store
                            .save_repository_group("/repo", true, StreamVersion::new(0).unwrap())
                            .await
                    }
                }
            })
            .await;
            // Then
            assert!(
                matches!(result, Err(WorkspaceStateError::Technical(error)) if error.nature == if cancelled { TechnicalFailureNature::Cancelled } else { TechnicalFailureNature::TimedOut })
            );
        }
    }
}
