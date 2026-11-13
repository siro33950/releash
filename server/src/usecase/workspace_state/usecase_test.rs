use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use crate::domain::workspace_state::WorkspaceStateKey;
use parking_lot::Mutex;

#[derive(Default)]
struct Store {
    conflict_returned: tokio::sync::Notify,
    save_failure: Option<crate::domain::failure::TechnicalFailure>,
    fail: bool,
    readable: bool,
    missing_files: Vec<String>,
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
        let conflict = if *remaining > 0 {
            *remaining -= 1;
            true
        } else {
            self.conflict
        };
        if conflict {
            self.conflict_returned.notify_one();
        }
        conflict
    }
}
#[async_trait::async_trait]
impl WorkspaceStateRepository for Store {
    async fn load_repository_group(&self, _: &str) -> Result<bool, WorkspaceStateError> {
        Ok(false)
    }
    async fn read_repository_group(
        &self,
        _: &str,
    ) -> Result<(crate::domain::local_event::StreamVersion, bool), WorkspaceStateError> {
        crate::common::operation_context::check()?;
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
        if let Some(failure) = &self.save_failure {
            return Err(WorkspaceStateError::Technical(failure.clone()));
        }
        if self.fail {
            Err(WorkspaceStateError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failed".into(),
                },
            ))
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
        crate::common::operation_context::check()?;
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
                        .or_else(|| self.saved.lock().last().cloned())
                },
            ))
        } else {
            Err(WorkspaceStateError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "unreadable".into(),
                },
            ))
        }
    }
    async fn filter_missing_files(
        &self,
        state: WorkspaceState,
        _: &str,
    ) -> Result<WorkspaceState, WorkspaceStateError> {
        Ok(
            crate::domain::workspace_state::services::filter_missing_files(state, |path| {
                !self.missing_files.iter().any(|missing| missing == path)
            }),
        )
    }
    async fn save(
        &self,
        name: &str,
        state: WorkspaceState,
        expected: crate::domain::local_event::StreamVersion,
    ) -> Result<crate::domain::local_event::StreamVersion, WorkspaceStateError> {
        assert_eq!(
            expected.value() + 1,
            self.reads.load(std::sync::atomic::Ordering::SeqCst) as i64
        );
        self.calls.lock().push(format!("save:{name}"));
        self.saved.lock().push(state);
        if self.should_conflict() {
            return Err(WorkspaceStateError::VersionConflict);
        }
        if let Some(failure) = &self.save_failure {
            return Err(WorkspaceStateError::Technical(failure.clone()));
        }
        if self.fail {
            Err(WorkspaceStateError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failed".into(),
                },
            ))
        } else {
            Ok(crate::domain::local_event::StreamVersion::new(expected.value() + 1).unwrap())
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
            missing_files: Vec::new(),
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            conflict_returned: Default::default(),
            save_failure: None,
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
            tabs: Some(WorkspaceTabsState { editors: vec![] }),
            layout: Some(WorkspaceLayoutState {
                center_tab: "editor".into(),
                active_view: "git".into(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
            }),
        };
        // When
        let result = save_workspace_state(
            &store,
            Some(&publisher),
            "worktree",
            state.clone(),
            crate::domain::local_event::StreamVersion::zero(),
        )
        .await;
        // Then
        assert_eq!(*store.calls.lock(), ["save:worktree"]);
        assert_eq!(result.is_err(), fail);
        if fail {
            assert!(changes.try_recv().is_err());
        } else {
            let (version, saved) = result.unwrap();
            assert_eq!(version.value(), 1);
            assert_eq!(saved, state);
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
        missing_files: Vec::new(),
        conflict: false,
        conflicts_remaining: Default::default(),
        reads: Default::default(),
        conflict_returned: Default::default(),
        save_failure: None,
        retry_previous: None,
        previous: None,
        saved: Default::default(),
        calls: Default::default(),
    };
    let state = WorkspaceState {
        panes: None,
        version: 1,
        tabs: Some(WorkspaceTabsState { editors: vec![] }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
        }),
    };
    // When
    let result = save_workspace_state(
        &store,
        None,
        "wt",
        state,
        crate::domain::local_event::StreamVersion::zero(),
    )
    .await;
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
            missing_files: Vec::new(),
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            conflict_returned: Default::default(),
            save_failure: None,
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
        missing_files: Vec::new(),
        conflict: false,
        conflicts_remaining: Default::default(),
        reads: Default::default(),
        conflict_returned: Default::default(),
        save_failure: None,
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
                tabs: Some(WorkspaceTabsState { editors: vec![] }),
                layout: Some(WorkspaceLayoutState {
                    center_tab: "editor".into(),
                    active_view: "git".into(),
                    left_nav_collapsed: false,
                    right_collapsed: false,
                    right_bottom_collapsed: false,
                }),
            },
            crate::domain::local_event::StreamVersion::zero(),
        )
    });
    // Then
    assert!(result.is_err());
    assert!(store.calls.lock().is_empty());
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
        tabs: Some(WorkspaceTabsState { editors: vec![] }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
        }),
    }
}
#[tokio::test]
async fn test_表示状態保存_版が進んでいれば省略した配置を補った上で版競合を返し再試行も通知もしない(
) {
    // Given
    let previous = state_with_panes();
    let store = Store {
        readable: true,
        missing_files: Vec::new(),
        conflict: true,
        previous: Some(previous.clone()),
        ..Default::default()
    };
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let mut next = previous.clone();
    next.panes = None;
    next.layout.as_mut().unwrap().left_nav_collapsed = true;
    // When
    let result = save_workspace_state(
        &store,
        Some(&publisher),
        "wt",
        next,
        crate::domain::local_event::StreamVersion::zero(),
    )
    .await;
    // Then
    assert!(matches!(result, Err(WorkspaceStateError::VersionConflict)));
    assert!(changes.try_recv().is_err());
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(store.calls.lock().as_slice(), ["save:wt"]);
    let saved = store.saved.lock();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].panes, previous.panes);
    assert!(saved[0].layout.as_ref().unwrap().left_nav_collapsed);
}

#[tokio::test]
async fn test_repositoryグループ保存_競合待ちの取消で版競合を返し通知しない() {
    // Given
    let store = Store {
        readable: true,
        missing_files: Vec::new(),
        conflict: true,
        ..Default::default()
    };
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let cancellation = std::sync::Arc::new(tokio_util::sync::CancellationToken::new());
    // When
    let (result, ()) = tokio::join!(
        crate::common::operation_context::scope(
            crate::common::operation_context::OperationContext::new(None, cancellation.clone()),
            save_repository_group_state(&store, Some(&publisher), "/repo", true),
        ),
        async {
            store.conflict_returned.notified().await;
            cancellation.cancel();
        }
    );
    // Then
    assert!(matches!(result, Err(WorkspaceStateError::VersionConflict)));
    assert!(changes.try_recv().is_err());
    assert_eq!(store.calls.lock().as_slice(), ["group:/repo:true"]);
}

#[tokio::test]
async fn test_repositoryグループ保存_競合後の技術的な期限切れをそのまま返し通知しない() {
    // Given
    let failure = crate::domain::failure::TechnicalFailure {
        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
        message: "save timed out".into(),
    };
    let store = Store {
        readable: true,
        conflicts_remaining: Mutex::new(1),
        save_failure: Some(failure.clone()),
        ..Default::default()
    };
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    // When
    let result = save_repository_group_state(&store, Some(&publisher), "/repo", true).await;
    // Then
    assert!(matches!(result, Err(WorkspaceStateError::Technical(actual)) if actual == failure));
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(store.calls.lock().len(), 2);
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
async fn test_repositoryグループ保存_版競合後に読み直し成功後だけ通知する() {
    // Given
    let store = Store {
        readable: true,
        conflicts_remaining: Mutex::new(1),
        ..Default::default()
    };
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    // When
    save_repository_group_state(&store, Some(&publisher), "/repo", true)
        .await
        .unwrap();
    // Then
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(store.calls.lock().len(), 2);
    assert!(changes.try_recv().is_ok());
    assert!(changes.try_recv().is_err());
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
            missing_files: Vec::new(),
            conflict: false,
            conflicts_remaining: Default::default(),
            reads: Default::default(),
            conflict_returned: Default::default(),
            save_failure: None,
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
                    save_workspace_state(
                        &store,
                        None,
                        "/repo/wt",
                        state_with_panes(),
                        crate::domain::local_event::StreamVersion::zero(),
                    )
                    .await
                    .map(|_| ())
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

fn legacy_state() -> WorkspaceState {
    use crate::domain::workspace_state::value_objects::pane_layout::{
        PaneLayout, PaneTab, PaneTabKind,
    };
    let mut state = state_with_panes();
    state.panes = Some(
        PaneLayout::pane(
            "pane".into(),
            ["resolved", "missing", "failed", "resolved"]
                .into_iter()
                .enumerate()
                .map(|(index, node)| PaneTab {
                    id: index.to_string(),
                    kind: PaneTabKind::Session,
                    node_id: Some(node.into()),
                    agent_session_id: None,
                })
                .collect(),
            Some("0".into()),
        )
        .unwrap(),
    );
    state
}

#[tokio::test]
async fn test_旧会話参照の復元_解決を保存して再照会せず未解決と失敗した参照を保持する() {
    // Given
    let query =
        crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(Vec::new());
    query
        .session_nodes
        .lock()
        .sessions
        .insert("resolved".into(), "agent".into());
    let failure = crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto {
        reference_id: "failed".into(),
        failure: crate::domain::failure::WorkFailure::from_error(
            &crate::domain::workflow::WorkflowError::external("lookup failed"),
        ),
    };
    query.session_nodes.lock().failures.push(failure.clone());
    let original = legacy_state();
    let store = Store {
        readable: true,
        previous: Some(original.clone()),
        ..Default::default()
    };
    // When
    let (version, actual, failures, migration_failure) =
        load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
            .await
            .unwrap();
    // Then
    assert_eq!(version.value(), 1);
    let mut expected = original;
    expected
        .panes
        .as_mut()
        .unwrap()
        .resolve_legacy_sessions(&[("resolved".into(), "agent".into())].into());
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(actual, Some(expected.clone()));
    assert_eq!(failures, vec![failure]);
    assert!(migration_failure.is_none());
    assert_eq!(store.saved.lock().as_slice(), &[expected]);
    let ids = query.resolved_nodes.lock()[0].clone();
    assert_eq!(ids.len(), 3);
    // When
    load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
        .await
        .unwrap();
    // Then
    let calls = query.resolved_nodes.lock();
    assert_eq!(calls.len(), 2);
    assert!(!calls[1].iter().any(|id| id == "resolved"));
    assert_eq!(store.saved.lock().len(), 1);
}

#[tokio::test]
async fn test_旧会話参照の移行_競合後は最新配置へ参照置換だけを適用する() {
    // Given
    let query =
        crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(Vec::new());
    query
        .session_nodes
        .lock()
        .sessions
        .insert("resolved".into(), "agent".into());
    let original = legacy_state();
    let mut latest = original.clone();
    latest.layout.as_mut().unwrap().left_nav_collapsed = true;
    latest.layout.as_mut().unwrap().right_collapsed = true;
    use crate::domain::workspace_state::value_objects::pane_layout::{PaneLayout, SplitAxis};
    latest.panes = Some(
        PaneLayout::split(
            "latest-split".into(),
            SplitAxis::Vertical,
            0.7,
            latest.panes.take().unwrap(),
            PaneLayout::pane("new-pane".into(), Vec::new(), None).unwrap(),
        )
        .unwrap(),
    );
    let store = Store {
        readable: true,
        previous: Some(original),
        retry_previous: Some(latest.clone()),
        conflicts_remaining: Mutex::new(1),
        ..Default::default()
    };
    // When
    let (_, actual, _, migration_failure) =
        load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
            .await
            .unwrap();
    // Then
    latest
        .panes
        .as_mut()
        .unwrap()
        .resolve_legacy_sessions(&[("resolved".into(), "agent".into())].into());
    assert_eq!(actual, Some(latest.clone()));
    assert!(migration_failure.is_none());
    assert_eq!(store.saved.lock().last(), Some(&latest));
    assert_eq!(store.saved.lock().len(), 2);
}

#[tokio::test]
async fn test_旧会話参照の復元_書き戻し失敗でも配置と取得失敗を返し保存失敗を保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    use crate::domain::workspace_state::value_objects::pane_layout::{PaneLayout, SplitAxis};
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        // Given
        let query = crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(
            Vec::new(),
        );
        query
            .session_nodes
            .lock()
            .sessions
            .insert("resolved".into(), "agent".into());
        let resolution_failure =
            crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto {
                reference_id: "failed".into(),
                failure: crate::domain::failure::WorkFailure::from_error(
                    &crate::domain::workflow::WorkflowError::external("lookup failed"),
                ),
            };
        query
            .session_nodes
            .lock()
            .failures
            .push(resolution_failure.clone());
        let mut original = legacy_state();
        original.panes = Some(
            PaneLayout::split(
                "split".into(),
                SplitAxis::Vertical,
                0.7,
                original.panes.take().unwrap(),
                state_with_panes().panes.unwrap(),
            )
            .unwrap(),
        );
        let save_failure = TechnicalFailure {
            nature,
            message: "migration write failed".into(),
        };
        let store = Store {
            readable: true,
            previous: Some(original.clone()),
            save_failure: Some(save_failure.clone()),
            ..Default::default()
        };
        // When
        let (version, actual, failures, migration_failure) =
            load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
                .await
                .unwrap();
        // Then
        assert_eq!(version.value(), 0);
        original
            .panes
            .as_mut()
            .unwrap()
            .resolve_legacy_sessions(&[("resolved".into(), "agent".into())].into());
        assert_eq!(actual, Some(original));
        assert_eq!(failures, vec![resolution_failure]);
        assert_eq!(
            migration_failure,
            Some(WorkspaceStateError::Technical(save_failure))
        );
        assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(store.calls.lock().as_slice(), ["save:/repo"]);
        assert_eq!(query.resolved_nodes.lock().len(), 1);
    }
}

#[tokio::test]
async fn test_配置復元_配置自体の取得失敗は成功として扱わない() {
    // Given
    let store = Store::default();
    let query =
        crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(Vec::new());
    // When
    let result = load_workspace_state(&store, query.as_ref(), "/repo", "/repo").await;
    // Then
    assert!(matches!(result, Err(WorkspaceStateError::Technical(_))));
    assert!(query.resolved_nodes.lock().is_empty());
}

#[tokio::test]
async fn test_配置復元_参照置換を保存してから返却値だけの欠損ファイルを除く() {
    // Given
    let query =
        crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(Vec::new());
    query
        .session_nodes
        .lock()
        .sessions
        .insert("resolved".into(), "agent".into());
    let mut original = legacy_state();
    original.tabs = Some(crate::domain::workspace_state::value_objects::WorkspaceTabsState {
        editors: vec![crate::domain::workspace_state::value_objects::workspace_tabs_state::WorkspaceTabEntry {
            path: "missing".into(),
            name: "missing".into(),
        }],
    });
    let store = Store {
        readable: true,
        previous: Some(original),
        missing_files: vec!["missing".into()],
        ..Default::default()
    };
    // When
    let (_, actual, _, _) = load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
        .await
        .unwrap();
    // Then
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    let saved = store.saved.lock();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tabs.as_ref().unwrap().editors[0].path, "missing");
    let actual = actual.unwrap();
    assert!(actual.tabs.as_ref().unwrap().editors.is_empty());
    assert_eq!(actual.panes, saved[0].panes);
}

#[tokio::test]
async fn test_配置復元_保存値なしと移行不要の配置は一度だけ読み保存しない() {
    // Given
    for previous in [None, Some(state_with_panes())] {
        let query = crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(
            Vec::new(),
        );
        let store = Store {
            readable: true,
            previous: previous.clone(),
            ..Default::default()
        };
        // When
        let (version, actual, failures, migration_failure) =
            load_workspace_state(&store, query.as_ref(), "/repo", "/repo")
                .await
                .unwrap();
        // Then
        assert_eq!(version.value(), 0);
        assert_eq!(actual, Some(previous.unwrap_or_default()));
        assert!(failures.is_empty());
        assert!(migration_failure.is_none());
        assert!(store.saved.lock().is_empty());
        assert!(query.resolved_nodes.lock().is_empty());
        assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn test_sessionタブ保存_競合後の最新配置へ追加して一度だけ通知する() {
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    // Given
    let mut latest = state_with_panes();
    latest.add_session_tab("other", "other-node", None);
    let store = std::sync::Arc::new(Store {
        readable: true,
        previous: Some(state_with_panes()),
        retry_previous: Some(latest.clone()),
        conflicts_remaining: Mutex::new(1),
        ..Default::default()
    });
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let usecase = super::super::session_tabs::SessionTabsUsecase::new(
        store.clone(),
        crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        publisher,
    );
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionTreeLocation::workflow_node("workflow", "node").unwrap(),
    )
    .unwrap();
    // When
    usecase.add_session(&session, None).await.unwrap();
    // Then
    latest.add_session_tab("session", "node", None);
    let saved = store.saved.lock();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved.last(), Some(&latest));
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::WorkspaceState("/repo".into())
    );
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
async fn test_sessionタブ保存_技術的失敗は再試行せず通知しない() {
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    // Given
    let failure = TechnicalFailure {
        nature: TechnicalFailureNature::Transient,
        message: "save failure".into(),
    };
    let store = std::sync::Arc::new(Store {
        readable: true,
        save_failure: Some(failure.clone()),
        ..Default::default()
    });
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let usecase = super::super::session_tabs::SessionTabsUsecase::new(
        store.clone(),
        crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        publisher,
    );
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionTreeLocation::workflow_node("workflow", "node").unwrap(),
    )
    .unwrap();
    // When / Then
    assert_eq!(
        usecase.add_session(&session, None).await,
        Err(WorkspaceStateError::Technical(failure))
    );
    assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(store.saved.lock().len(), 1);
    assert!(changes.try_recv().is_err());
}
