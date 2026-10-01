use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use parking_lot::Mutex;

struct Store {
    fail: bool,
    calls: Mutex<Vec<String>>,
}
impl WorkspaceStateRepository for Store {
    fn check_readable(&self, _: &str) -> Result<(), WorkspaceStateError> {
        Ok(())
    }
    fn load(&self, _: &str, _: &str) -> Result<Option<WorkspaceState>, WorkspaceStateError> {
        unreachable!()
    }
    fn set(&self, name: &str, _: WorkspaceState) {
        self.calls.lock().push(format!("set:{name}"));
    }
    fn save(&self, name: &str) -> Result<(), WorkspaceStateError> {
        self.calls.lock().push(format!("save:{name}"));
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn test_表示状態保存_保存成功後だけ対象名を通知する() {
    for fail in [false, true] {
        // Given
        let store = Store {
            fail,
            calls: Default::default(),
        };
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let state = WorkspaceState {
            version: 1,
            tabs: WorkspaceTabsState {
                editors: vec![],
                active_editor_path: None,
            },
            layout: WorkspaceLayoutState {
                center_tab: "editor".into(),
                active_view: "git".into(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
                right_bottom_active_tab: None,
                selected_diff_file: None,
            },
        };
        // When
        let result = save_workspace_state(&store, Some(&publisher), "worktree", state);
        // Then
        assert_eq!(*store.calls.lock(), ["set:worktree", "save:worktree"]);
        assert_eq!(result.is_err(), fail);
        if fail {
            assert!(changes.try_recv().is_err());
        } else {
            assert_eq!(
                changes.try_recv().unwrap(),
                crate::usecase::state_subscription::StateChangeSource::WorkspaceState(
                    "worktree".into()
                )
            );
        }
    }
}

#[test]
fn test_表示状態保存_保存ファイルなしなら保存できる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::workspace_state::repository_impl::WorkspaceStateStore::new(
        directory.path().into(),
    );
    let path = directory.path().join("workspace_state/wt.json");
    let state = WorkspaceState {
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };

    // When
    let result = save_workspace_state(&store, None, "wt", state);
    // Then
    assert_eq!(result.is_ok(), true);
    assert!(path.is_file());
}

#[test]
fn test_表示状態保存_読める保存ファイルなら保存できる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::workspace_state::repository_impl::WorkspaceStateStore::new(
        directory.path().into(),
    );
    let path = directory.path().join("workspace_state/wt.json");
    let state = WorkspaceState {
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    store.set("wt", state.clone());
    store.save("wt").unwrap();
    // When
    let result = save_workspace_state(&store, None, "wt", state);
    // Then
    assert_eq!(result.is_ok(), true);
    assert!(path.is_file());
}

#[test]
fn test_表示状態保存_読取成功後に壊れたファイルを上書きしない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::workspace_state::repository_impl::WorkspaceStateStore::new(
        directory.path().into(),
    );
    let path = directory.path().join("workspace_state/wt.json");
    let state = WorkspaceState {
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    store.set("wt", state.clone());
    store.save("wt").unwrap();
    store.check_readable("wt").unwrap();
    std::fs::write(&path, "broken").unwrap();
    let before = std::fs::read(&path).ok();
    // When
    let result = save_workspace_state(&store, None, "wt", state);
    // Then
    assert_eq!(result.is_ok(), false);
    assert_eq!(std::fs::read(&path).ok(), before);
}

#[test]
fn test_表示状態保存_読めない保存ファイルを上書きしない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::workspace_state::repository_impl::WorkspaceStateStore::new(
        directory.path().into(),
    );
    let path = directory.path().join("workspace_state/wt.json");
    let state = WorkspaceState {
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    std::fs::create_dir_all(&path).unwrap();
    let before = std::fs::read(&path).ok();
    // When
    let result = save_workspace_state(&store, None, "wt", state);
    // Then
    assert_eq!(result.is_ok(), false);
    assert_eq!(std::fs::read(&path).ok(), before);
}
