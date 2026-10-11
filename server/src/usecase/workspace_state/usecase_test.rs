use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use parking_lot::Mutex;

struct Store {
    fail: bool,
    readable: bool,
    calls: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl WorkspaceStateRepository for Store {
    async fn migrate_legacy(
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

    async fn load_repository_group(&self, _: &str) -> Result<bool, WorkspaceStateError> {
        Ok(false)
    }
    async fn save_repository_group(
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

    async fn read(
        &self,
        _: &str,
    ) -> Result<
        (
            crate::domain::local_event::StreamVersion,
            Option<WorkspaceState>,
        ),
        WorkspaceStateError,
    > {
        if self.readable {
            Ok((
                crate::domain::local_event::StreamVersion::new(0).unwrap(),
                None,
            ))
        } else {
            Err(WorkspaceStateError::Message("unreadable".into()))
        }
    }
    async fn load(&self, _: &str, _: &str) -> Result<Option<WorkspaceState>, WorkspaceStateError> {
        unreachable!()
    }
    async fn save(
        &self,
        name: &str,
        _: WorkspaceState,
        _: crate::domain::local_event::StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        self.calls.lock().push(format!("save:{name}"));
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }
    async fn states(
        &self,
    ) -> Result<Vec<(String, crate::domain::local_event::StreamVersion)>, WorkspaceStateError> {
        unreachable!()
    }
    async fn invalidate(
        &self,
        _: &str,
        _: crate::domain::local_event::StreamVersion,
    ) -> Result<bool, WorkspaceStateError> {
        unreachable!()
    }
}

#[tokio::test]
async fn test_表示状態保存_保存成功後だけ対象名を通知する() {
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
        let result = save_workspace_state(&store, Some(&publisher), "worktree", state).await;
        // Then
        assert_eq!(*store.calls.lock(), ["save:worktree"]);
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

#[tokio::test]
async fn test_表示状態保存_読取失敗ならsetとsaveを呼ばない() {
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
    let result = save_workspace_state(&store, None, "wt", state).await;
    // Then
    assert!(result.is_err());
    assert!(store.calls.lock().is_empty());
}

#[tokio::test]
async fn test_repository折りたたみ保存_成功したgroupだけ通知する() {
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
        let result = save_repository_group_state(&store, Some(&publisher), "/repo", true).await;
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

#[tokio::test]
async fn test_表示状態保存_不正なpane配置は保存前に拒否する() {
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

#[tokio::test]
async fn test_旧表示状態の移行_登録repositoryを列挙し未解決なら名前の解決を見送る() {
    for fail in [false, true] {
        for unresolved in [false, true] {
            // Given
            let store = Store {
                fail,
                readable: true,
                calls: Default::default(),
            };
            let fake = std::sync::Arc::new(crate::usecase::test_helpers::FakeRepo {
                fail_list_worktrees: unresolved,
                worktrees: vec![crate::domain::repository::Worktree {
                    name: "a".into(),
                    path: "/repo/a".into(),
                    branch: "a".into(),
                    is_main: false,
                    is_locked: false,
                    is_merged: false,
                }],
                ..Default::default()
            });
            let repository = crate::usecase::test_helpers::usecase(fake.clone());
            // When
            let result = migrate_workspace_states(&store, &["/repo".into()], &repository).await;
            // Then
            assert_eq!(result.migration.is_err(), fail);
            assert_eq!(
                result.unresolved_repositories.len(),
                usize::from(unresolved)
            );
            if unresolved {
                assert_eq!(result.unresolved_repositories[0].0, "/repo");
                assert_eq!(
                    result.unresolved_repositories[0].1.to_string(),
                    "worktrees unavailable"
                );
            }
            assert_eq!(*fake.listed_worktree_paths.lock(), ["/repo"]);
            assert_eq!(
                *store.calls.lock(),
                [if unresolved {
                    "migration:[\"/repo\"]:[]"
                } else {
                    "migration:[\"/repo\"]:[\"/repo/a\"]"
                }]
            );
        }
    }
}
