use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use crate::domain::workspace_state::WorkspaceStateKey;
use parking_lot::Mutex;

struct Store {
    fail: bool,
    readable: bool,
    conflict: bool,
    conflicts_remaining: Mutex<usize>,
    reads: std::sync::atomic::AtomicUsize,
    retry_previous: Option<WorkspaceState>,
    previous: Option<WorkspaceState>,
    saved: Mutex<Vec<WorkspaceState>>,
    calls: Mutex<Vec<String>>,
}
impl Store {
    fn should_conflict(&self) -> bool {
        let mut remaining = self.conflicts_remaining.lock();
        if *remaining > 0 {
            *remaining -= 1;
            true
        } else {
            self.conflict
        }
    }
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
    async fn read_repository_group(
        &self,
        _: &str,
    ) -> Result<(crate::domain::local_event::StreamVersion, bool), WorkspaceStateError> {
        let read = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok((
            crate::domain::local_event::StreamVersion::new(read as i64).unwrap(),
            false,
        ))
    }
    async fn save_repository_group(
        &self,
        path: &str,
        collapsed: bool,
        expected: crate::domain::local_event::StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        assert_eq!(
            expected.value() + 1,
            self.reads.load(std::sync::atomic::Ordering::SeqCst) as i64
        );
        self.calls.lock().push(format!("group:{path}:{collapsed}"));
        if self.should_conflict() {
            return Err(WorkspaceStateError::VersionConflict);
        }
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
                crate::domain::local_event::StreamVersion::new(
                    self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as i64,
                )
                .unwrap(),
                if self.saved.lock().is_empty() {
                    self.previous.clone()
                } else {
                    self.retry_previous
                        .clone()
                        .or_else(|| self.previous.clone())
                },
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
        state: WorkspaceState,
        expected: crate::domain::local_event::StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        assert_eq!(
            expected.value() + 1,
            self.reads.load(std::sync::atomic::Ordering::SeqCst) as i64
        );
        self.calls.lock().push(format!("save:{name}"));
        self.saved.lock().push(state);
        if self.should_conflict() {
            return Err(WorkspaceStateError::VersionConflict);
        }
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }
    async fn states(
        &self,
    ) -> Result<
        Vec<(WorkspaceStateKey, crate::domain::local_event::StreamVersion)>,
        WorkspaceStateError,
    > {
        unreachable!()
    }
    async fn invalidate(
        &self,
        _: &WorkspaceStateKey,
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
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            retry_previous: None,
            previous: None,
            saved: Default::default(),
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
        conflict: false,
        conflicts_remaining: Default::default(),
        reads: Default::default(),
        retry_previous: None,
        previous: None,
        saved: Default::default(),
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
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            retry_previous: None,
            previous: None,
            saved: Default::default(),
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
        conflict: false,
        conflicts_remaining: Default::default(),
        reads: Default::default(),
        retry_previous: None,
        previous: None,
        saved: Default::default(),
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
                conflict: false,
                conflicts_remaining: Default::default(),
                reads: Default::default(),
                retry_previous: None,
                previous: None,
                saved: Default::default(),
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

fn state_with_panes() -> WorkspaceState {
    WorkspaceState {
        panes: Some(
            crate::domain::workspace_state::value_objects::pane_layout::PaneLayout::pane(
                "previous".into(),
                vec![],
                None,
            )
            .unwrap(),
        ),
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
    }
}
#[tokio::test]
async fn test_表示状態保存_最新の配置を保持し待ちが期限を越える版競合を返す() {
    for conflict in [false, true] {
        // Given
        let previous = state_with_panes();
        let store = Store {
            fail: false,
            readable: true,
            conflict,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            retry_previous: None,
            previous: Some(previous.clone()),
            saved: Default::default(),
            calls: Default::default(),
        };
        let mut next = previous.clone();
        next.panes = None;
        next.layout.left_nav_collapsed = true;
        // When
        let result = crate::common::operation_context::scope(
            crate::common::operation_context::with_timeout(std::time::Duration::from_millis(10)),
            save_workspace_state(&store, None, "wt", next),
        )
        .await;
        // Then
        assert_eq!(
            matches!(result, Err(WorkspaceStateError::VersionConflict)),
            conflict
        );
        assert_eq!(store.calls.lock().as_slice(), ["save:wt"]);
        let saved = store.saved.lock();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].panes, previous.panes);
        assert!(saved[0].layout.left_nav_collapsed);
    }
}
#[tokio::test]
async fn test_repositoryグループ保存_待ちが期限を越える版競合を返す() {
    // Given
    let store = Store {
        fail: false,
        readable: true,
        conflict: true,
        conflicts_remaining: Default::default(),
        reads: Default::default(),
        retry_previous: None,
        previous: None,
        saved: Default::default(),
        calls: Default::default(),
    };
    // When
    let result = crate::common::operation_context::scope(
        crate::common::operation_context::with_timeout(std::time::Duration::from_millis(10)),
        save_repository_group_state(&store, None, "/repo", true),
    )
    .await;
    // Then
    assert!(matches!(result, Err(WorkspaceStateError::VersionConflict)));
    assert_eq!(store.calls.lock().as_slice(), ["group:/repo:true"]);
}

#[tokio::test]
async fn test_ui状態保存_両方の保存が版競合後に読み直し成功後だけ通知する() {
    for group in [false, true] {
        // Given
        let previous = state_with_panes();
        let mut latest = previous.clone();
        latest.panes = Some(
            crate::domain::workspace_state::value_objects::pane_layout::PaneLayout::pane(
                "latest".into(),
                vec![],
                None,
            )
            .unwrap(),
        );
        let store = Store {
            fail: false,
            readable: true,
            conflict: false,
            conflicts_remaining: Mutex::new(1),
            reads: Default::default(),
            retry_previous: Some(latest.clone()),
            previous: Some(previous),
            saved: Default::default(),
            calls: Default::default(),
        };
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let mut next = state_with_panes();
        next.panes = None;
        next.layout.left_nav_collapsed = true;
        // When
        if group {
            save_repository_group_state(&store, Some(&publisher), "/repo", true)
                .await
                .unwrap();
        } else {
            save_workspace_state(&store, Some(&publisher), "/repo/wt", next)
                .await
                .unwrap();
        }
        // Then
        assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(store.calls.lock().len(), 2);
        if !group {
            let saved = store.saved.lock();
            assert_eq!(saved[1].panes, latest.panes);
            assert!(saved[1].layout.left_nav_collapsed);
        }
        assert!(changes.try_recv().is_ok());
        assert!(changes.try_recv().is_err());
    }
}

#[tokio::test]
async fn test_ui状態保存_取消済みなら両方とも読み書きしない() {
    for group in [false, true] {
        // Given
        let cancellation = std::sync::Arc::new(tokio_util::sync::CancellationToken::new());
        cancellation.cancel();
        let store = Store {
            fail: false,
            readable: true,
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            retry_previous: None,
            previous: None,
            saved: Default::default(),
            calls: Default::default(),
        };
        // When
        let result = crate::common::operation_context::scope(
            crate::common::operation_context::OperationContext::new(None, cancellation),
            async {
                if group {
                    save_repository_group_state(&store, None, "/repo", true).await
                } else {
                    save_workspace_state(&store, None, "/repo/wt", state_with_panes()).await
                }
            },
        )
        .await;
        // Then
        assert!(
            matches!(result, Err(WorkspaceStateError::Technical(error)) if error.nature == crate::domain::failure::TechnicalFailureNature::Cancelled && error.message == "Operation cancelled")
        );
        assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(store.calls.lock().is_empty());
    }
}

#[tokio::test]
async fn test_ui状態保存_最初の期限切れと競合待ちの取消の種別を保持する() {
    use crate::common::operation_context::{self, Deadline, OperationContext};
    use crate::domain::failure::TechnicalFailureNature;
    // Given
    let expired =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When
    let result = operation_context::scope(
        expired,
        save_with_retry(|| async { panic!("expired operation called") }),
    )
    .await;
    // Then
    assert!(
        matches!(result, Err(WorkspaceStateError::Technical(error)) if error.nature == TechnicalFailureNature::TimedOut)
    );
    // Given
    let cancellation = std::sync::Arc::new(tokio_util::sync::CancellationToken::new());
    let context = OperationContext::new(None, cancellation.clone());
    // When
    let result = operation_context::scope(
        context,
        save_with_retry(|| async {
            cancellation.cancel();
            Err(WorkspaceStateError::VersionConflict)
        }),
    )
    .await;
    // Then
    assert!(
        matches!(result, Err(WorkspaceStateError::Technical(error)) if error.nature == TechnicalFailureNature::Cancelled)
    );
}
