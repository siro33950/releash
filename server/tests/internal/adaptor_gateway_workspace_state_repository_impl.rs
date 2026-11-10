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
        tabs: Some(WorkspaceTabsState {
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
        }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "editor".to_string(),
            active_view: "git".to_string(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
        }),
    }
}

#[tokio::test]
async fn test_workspace状態保存_設定未保存のsession配置を再起動後も復元する() {
    // Given
    let directory = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let mut state = WorkspaceState::default();
    state.add_session_tab("session", "node");
    // When
    let (version, _) = store.read("worktree").await.unwrap();
    store
        .save("worktree", state.clone(), version)
        .await
        .unwrap();
    drop(store);
    let restarted = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    let actual = restarted.read("worktree").await.unwrap().1.unwrap();
    // Then
    assert!(actual.tabs.is_none());
    assert!(actual.layout.is_none());
    assert_eq!(actual, state);
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
    let loaded = load(&store, "wt1", worktree_dir.to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.tabs.as_ref().unwrap().editors.len(), 2);
    assert_eq!(loaded.layout.as_ref().unwrap().center_tab, "editor");
}

#[tokio::test]
pub async fn load_nonexistent_returns_none() {
    // Given
    let dir = TempDir::new().unwrap();
    let store = releashd::test_support::integration::platform::workspace_state_store(
        dir.path().to_path_buf(),
    );
    // When
    let loaded = load(&store, "nonexistent", "/tmp").await.unwrap();
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
            .as_ref()
            .unwrap()
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
    one.layout.as_mut().unwrap().left_nav_collapsed = true;
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
    assert!(
        load(&restored, "/repo/a/b", "/repo/a/b")
            .await
            .unwrap()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .left_nav_collapsed
    );
    assert!(
        !load(&restored, "/repo/a_b", "/repo/a_b")
            .await
            .unwrap()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .left_nav_collapsed
    );
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
                },
                PaneTab {
                    agent_session_id: Some("agent-session".into()),
                    node_id: None,
                    id: "session".into(),
                    kind: PaneTabKind::Session,
                },
                PaneTab {
                    agent_session_id: None,
                    node_id: Some("command-node".into()),
                    id: "command".into(),
                    kind: PaneTabKind::Command,
                },
            ],
            Some("session".into()),
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
    react.layout.as_mut().unwrap().left_nav_collapsed = true;
    react.tabs.as_mut().unwrap().editors.clear();
    // When
    save_workspace_state(&store, None, key, react.clone())
        .await
        .unwrap();
    // Then
    let saved = load(&store, key, key).await.unwrap().unwrap();
    assert_eq!(saved.panes, Some(panes.clone()));
    assert_eq!(saved.tabs, react.tabs);
    assert_eq!(saved.layout, react.layout);
    drop(store);
    let restarted = releashd::test_support::integration::platform::workspace_state_store(
        directory.path().into(),
    );
    assert_eq!(load(&restarted, key, key).await.unwrap().unwrap(), saved);
    let mut changed = react.clone();
    changed.panes = Some(PaneLayout::pane("all-closed".into(), vec![], None).unwrap());
    save_workspace_state(&restarted, None, key, changed.clone())
        .await
        .unwrap();
    save_workspace_state(&restarted, None, key, react)
        .await
        .unwrap();
    assert_eq!(
        load(&restarted, key, key).await.unwrap().unwrap().panes,
        changed.panes
    );
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
    next.layout.as_mut().unwrap().left_nav_collapsed = true;
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
        load(&store, path, path)
            .await
            .unwrap()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
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
    winner.layout.as_mut().unwrap().left_nav_collapsed = true;
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
async fn test_ui状態の並行保存_groupは再読取で後続を反映しworkspaceは版競合で後続を捨てる() {
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
        let store = Arc::new(WorkspaceStateStore::with_store(events.clone()));
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
                state.layout.as_mut().unwrap().left_nav_collapsed = true;
                save_workspace_state(second_store.as_ref(), None, "/repo/wt", state.clone()).await
            }
        });
        wait_for_ui_writes(events.as_ref(), 2).await;
        // When
        stall.release();
        append.await.unwrap();
        first.await.unwrap().unwrap();
        let second = second.await.unwrap();
        drop(store);
        let restored = WorkspaceStateStore::with_store(events.clone());
        // Then
        if group {
            second.unwrap();
            assert!(!restored.load_repository_group("/repo").await.unwrap());
            assert_eq!(event_count(dir.path()), 2);
        } else {
            assert!(matches!(
                second,
                Err(releashd::test_support::integration::platform::WorkspaceStateError::VersionConflict)
            ));
            let (version, state) = restored.read("/repo/wt").await.unwrap();
            assert_eq!(version.value(), 1);
            let state = state.unwrap();
            assert!(!state.layout.as_ref().unwrap().left_nav_collapsed);
            assert_eq!(state.panes, Some(panes));
            assert_eq!(event_count(dir.path()), 1);
        }
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
    let store = Arc::new(WorkspaceStateStore::with_store(events.clone()));
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
                load(&store, "/repo/wt", "/repo/wt").await.unwrap_err()
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
    state.tabs.as_mut().unwrap().editors = entries
        .iter()
        .map(|path| WorkspaceTabEntry {
            path: path.clone(),
            name: path.clone(),
        })
        .collect();
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
    let loaded = load(&store, "worktree", root.to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        loaded
            .tabs
            .as_ref()
            .unwrap()
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
                    2 => store
                        .save("/repo/wt", make_state(), StreamVersion::new(0).unwrap())
                        .await
                        .map(|_| ()),
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

pub(crate) async fn load(
    store: &impl WorkspaceStateRepository,
    key: &str,
    root: &str,
) -> Result<
    Option<WorkspaceState>,
    releashd::test_support::integration::platform::WorkspaceStateError,
> {
    match store.read(key).await?.1 {
        Some(state) => store.filter_missing_files(state, root).await.map(Some),
        None => Ok(None),
    }
}
