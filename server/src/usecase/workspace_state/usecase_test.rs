use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use parking_lot::Mutex;

struct Store {
    fail: bool,
    readable: bool,
    calls: Mutex<Vec<String>>,
}
impl WorkspaceStateRepository for Store {
    fn migrate_legacy(
        &self,
        repositories: &[String],
        worktrees: &[String],
    ) -> Result<(), WorkspaceStateError> {
        self.calls
            .lock()
            .push(format!("migration:{repositories:?}:{worktrees:?}"));
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }

    fn load_repository_group(&self, _: &str) -> Result<bool, WorkspaceStateError> {
        Ok(false)
    }
    fn save_repository_group(
        &self,
        path: &str,
        collapsed: bool,
    ) -> Result<(), WorkspaceStateError> {
        self.calls.lock().push(format!("group:{path}:{collapsed}"));
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }

    fn check_readable(&self, _: &str) -> Result<(), WorkspaceStateError> {
        if self.readable {
            Ok(())
        } else {
            Err(WorkspaceStateError::Message("unreadable".into()))
        }
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
            readable: true,
            calls: Default::default(),
        };
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let state = WorkspaceState {
            panes: None,
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
fn test_表示状態保存_読取失敗ならsetとsaveを呼ばない() {
    // Given
    let store = Store {
        fail: false,
        readable: false,
        calls: Default::default(),
    };
    let state = WorkspaceState {
        panes: None,
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
    assert!(result.is_err());
    assert!(store.calls.lock().is_empty());
}

#[test]
fn test_repository折りたたみ保存_成功したgroupだけ通知する() {
    for fail in [false, true] {
        // Given
        let store = Store {
            fail,
            readable: true,
            calls: Default::default(),
        };
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        // When
        let result = save_repository_group_state(&store, Some(&publisher), "/repo", true);
        // Then
        assert_eq!(result.is_err(), fail);
        assert_eq!(*store.calls.lock(), ["group:/repo:true"]);
        if fail {
            assert!(changes.try_recv().is_err());
        } else {
            assert_eq!(
                changes.try_recv().unwrap(),
                crate::usecase::state_subscription::StateChangeSource::RepositoryGroupState(
                    "/repo".into()
                )
            );
        }
    }
}

#[test]
fn test_表示状態保存_不正なpane配置は保存前に拒否する() {
    // Given
    let store = Store {
        fail: false,
        readable: true,
        calls: Default::default(),
    };
    // When
    let result = crate::domain::workspace_state::value_objects::pane_layout::PaneLayout::pane(
        "pane".into(),
        vec![],
        Some("missing".into()),
    )
    .map(|panes| {
        save_workspace_state(
            &store,
            None,
            "wt",
            WorkspaceState {
                panes: Some(panes),
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
            },
        )
    });
    // Then
    assert!(result.is_err());
    assert!(store.calls.lock().is_empty());
}

#[test]
fn test_旧表示状態の移行_登録repositoryとworktreeを渡し失敗を返す() {
    for fail in [false, true] {
        // Given
        let store = Store {
            fail,
            readable: true,
            calls: Default::default(),
        };
        // When
        let result = migrate_workspace_states(&store, &["/repo".into()], &["/repo/a".into()]);
        // Then
        assert_eq!(result.is_err(), fail);
        assert_eq!(*store.calls.lock(), ["migration:[\"/repo\"]:[\"/repo/a\"]"]);
    }
}
