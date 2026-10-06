use super::*;
use crate::usecase::test_helpers::*;
use parking_lot::Mutex;
#[test]
fn test_ブランチ作成を委譲する() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    usecase(fake.clone()).create_branch("/r", "feat").unwrap();
    assert_eq!(*fake.created_branches.lock(), vec!["feat".to_string()]);
}

#[test]
fn test_worktree作成をdtoへ合成する() {
    // Given
    let fake = Arc::new(<FakeRepo as Default>::default());
    let entry = usecase(fake.clone())
        .create_worktree("/r", "feat/issues/1302", true, Some("main"))
        // When
        .unwrap();
    // Then
    assert_eq!(entry.branch, "feat/issues/1302");
    assert_eq!(entry.path, "/r-worktrees/feat-issues-1302");

    // base 指定時は usecase が releash-base を設定する（旧 gateway 内蔵処理の引き上げ）。
    assert_eq!(
        *fake.set_branch_base_override_calls.lock(),
        vec![("feat/issues/1302".to_string(), Some("main".to_string()))]
    );
}

#[test]
fn test_worktree作成_base未指定ではbase設定しない() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    usecase(fake.clone())
        .create_worktree("/r", "feat", true, None)
        .unwrap();
    assert!(fake.set_branch_base_override_calls.lock().is_empty());
}

#[test]
fn test_worktree一覧_pathとbranchとis_mainを写す() {
    // Given
    let fake = Arc::new(FakeRepo {
        worktrees: vec![wt("/wt-feat", "feat", false)],
        ..<FakeRepo as Default>::default()
    });
    // When
    let entries = usecase(fake).list_worktrees("/r").unwrap();
    // Then
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.path, "/wt-feat");
    assert_eq!(e.branch, "feat");
    assert!(!e.is_main);
}

#[test]
fn test_linked_worktreeのreadには開いたrepo_pathを渡す() {
    let fake = Arc::new(FakeRepo {
        worktrees: vec![wt("/linked", "feature", false)],
        ..<FakeRepo as Default>::default()
    });
    let repository = usecase(fake.clone());

    repository.list_worktrees("/linked").unwrap();
    repository.list_working_worktrees("/linked").unwrap();
    repository.list_branches_with_worktree("/linked").unwrap();

    assert_eq!(*fake.listed_worktree_paths.lock(), vec!["/linked"; 3]);
}

#[test]
fn test_repository_root解決に失敗したreadはqueryを呼ばずに失敗する() {
    let fake = Arc::new(FakeRepo {
        worktrees: vec![wt("/linked", "feature", false)],
        fail_main_repo_path: true,
        ..<FakeRepo as Default>::default()
    });
    let repository = usecase(fake.clone());

    assert!(repository.list_worktrees("/linked").is_err());
    assert!(repository.list_working_worktrees("/linked").is_err());
    assert!(repository.list_branches_with_worktree("/linked").is_err());

    assert!(fake.listed_worktree_paths.lock().is_empty());
}

#[test]
fn test_作業worktree一覧_隔離worktreeを除きmainを先頭にブランチ名順で返す() {
    // Given
    let fake = Arc::new(FakeRepo {
        worktrees: vec![
            wt("/main-worktrees/feature-b", "feature-b", false),
            wt(
                "/main-worktrees/.releash-isolated/node-a1",
                "releash/isolated/node-a1",
                false,
            ),
            wt("/main", "main", true),
            wt(
                "/main-worktrees/other-a1",
                "releash/isolated/other-a1",
                false,
            ),
            wt("/main-worktrees/feature-a", "feature-a", false),
        ],
        ..<FakeRepo as Default>::default()
    });
    // When
    let worktrees = usecase(fake).list_working_worktrees("/main").unwrap();
    // Then
    assert_eq!(
        worktrees
            .iter()
            .map(|worktree| worktree.branch.as_str())
            .collect::<Vec<_>>(),
        vec![
            "main",
            "feature-a",
            "feature-b",
            "releash/isolated/other-a1"
        ]
    );
}

#[test]
fn test_ブランチ状態_worktreeの有無を付け隔離worktreeのブランチを含めない() {
    // Given
    let fake = Arc::new(FakeRepo {
        branches: vec![
            Branch::local("feature"),
            Branch::local("main"),
            Branch::local("releash/isolated/retained-a1"),
            Branch::local("releash/isolated/hidden-a1"),
            Branch::remote("origin/main"),
        ],
        worktrees: vec![
            wt("/main", "main", true),
            wt(
                "/main-worktrees/.releash-isolated/hidden-a1",
                "releash/isolated/hidden-a1",
                false,
            ),
            wt("/main-worktrees/detached", "detached-head", false),
        ],
        ..<FakeRepo as Default>::default()
    });
    // When
    let branches = usecase(fake).list_branches_with_worktree("/main").unwrap();
    // Then
    assert_eq!(
        branches,
        vec![
            (Branch::local("feature"), false),
            (Branch::local("main"), true),
            (Branch::local("releash/isolated/retained-a1"), false),
            (Branch::local("detached-head"), true),
        ]
    );
}

#[test]
fn test_worktree一覧_完全な隔離命名の実体を非表示にする() {
    let fake = Arc::new(FakeRepo {
        worktrees: vec![
            wt(
                "/main-worktrees/.releash-isolated/orphan-a1",
                "releash/isolated/orphan-a1",
                false,
            ),
            wt("/main-worktrees/feature", "feature", false),
        ],
        ..<FakeRepo as Default>::default()
    });

    let entries = usecase(fake).list_worktrees("/main").unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].branch, "feature");
}

#[test]
fn test_worktree一覧のpathはunc_prefixを保持して正規化する() {
    let fake = Arc::new(FakeRepo {
        worktrees: vec![wt(r"\\server\share\wt-feat", "feat", false)],
        ..<FakeRepo as Default>::default()
    });

    let entries = usecase(fake).list_worktrees("/r").unwrap();

    assert_eq!(entries[0].path, "//server/share/wt-feat");
}

#[test]
fn test_worktree作成はunc_repo_pathから意味保存でpathを導出する() {
    let fake = Arc::new(<FakeRepo as Default>::default());

    let entry = usecase(fake)
        .create_worktree(r"\\server\share\repo", "feat/issues/1302", true, None)
        .unwrap();

    assert_eq!(entry.path, "//server/share/repo-worktrees/feat-issues-1302");
}

#[test]
fn test_worktree作成エラーをusecaseエラーへ変換する() {
    let fake = Arc::new(FakeRepo {
        fail_create_worktree: true,
        ..<FakeRepo as Default>::default()
    });
    let err = usecase(fake)
        .create_worktree("/r", "feat", true, None)
        .unwrap_err();
    assert_eq!(err.to_string(), "boom");
}

#[test]
fn test_gcを委譲する() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    usecase(fake.clone())
        .prune_stale_branch_bases("/r", &["a".to_string(), "b".to_string()])
        .unwrap();
    assert_eq!(
        *fake.prune_calls.lock(),
        vec![vec!["a".to_string(), "b".to_string()]]
    );
}

#[test]
fn test_config設定系を委譲する() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    let uc = usecase(fake.clone());
    uc.set_releash_base("/r", Some("dev")).unwrap();
    uc.set_branch_base_override("/r", "feat", Some("main"))
        .unwrap();
    assert_eq!(
        *fake.set_releash_base_calls.lock(),
        vec![Some("dev".to_string())]
    );
    assert_eq!(
        *fake.set_branch_base_override_calls.lock(),
        vec![("feat".to_string(), Some("main".to_string()))]
    );
}

#[test]
fn test_起動worktree_一件だけのときpathと表示名を返す() {
    for count in [0, 1, 2] {
        let fake = Arc::new(FakeRepo {
            worktrees: (0..count)
                .map(|index| Worktree {
                    name: format!("wt{index}"),
                    path: format!("/main/wt{index}"),
                    branch: format!("feature{index}"),
                    is_main: index == 0,
                    is_locked: false,
                    is_merged: false,
                })
                .collect(),
            ..Default::default()
        });
        let result = usecase(fake).startup_worktree().unwrap();
        if count == 1 {
            let worktree = result.unwrap();
            assert_eq!(worktree.path, "/main/wt0");
            assert_eq!(worktree.branch, "feature0");
            assert_eq!(worktree.repository_name, "main");
        } else {
            assert!(result.is_none());
        }
    }
}

#[tokio::test]
pub async fn test_worktree削除_対応ブランチのbaseを後始末する() {
    // remove が返したブランチ名で releash-base を best-effort 削除する。
    let fake = Arc::new(FakeRepo {
        removed_branch: Some("feat".to_string()),
        ..<FakeRepo as Default>::default()
    });
    usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/r", "/wt", false)
        .await
        .unwrap();
    fake.wait_for_deletion("/wt").await;
    assert_eq!(
        *fake.set_branch_base_override_calls.lock(),
        vec![("feat".to_string(), None)]
    );
}
#[tokio::test]
pub async fn test_削除中worktree_管理情報が消えても対象だけ残しguard終了で消す() {
    // Given
    let fake = Arc::new(<FakeRepo as Default>::default());
    let repository = usecase(fake.clone());
    let mut deletion = fake
        .operations
        .delete("/main-worktrees/feature")
        .await
        .unwrap();
    deletion
        .accept(
            crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                repository_root: "/main".into(),
                path: "/main-worktrees/feature".into(),
                branch: Some("feature".into()),
            },
        )
        .unwrap();
    for path in [
        Some("/main-worktrees/feature"),
        Some("/alias/feature"),
        None,
    ] {
        let scanned = std::iter::once(wt("/main", "main", true))
            .chain(path.map(|path| wt(path, "feature", false)))
            .collect();
        // When
        let rows = repository.with_deleting_worktrees("/main", scanned);
        // Then
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].1);
        assert!(rows[1].1);
        assert_eq!(rows[1].0.branch, "feature");
        assert_eq!(rows[1].0.path, path.unwrap_or("/main-worktrees/feature"));
    }
    drop(deletion);
    let rows = repository.with_deleting_worktrees("/main", vec![wt("/main", "main", true)]);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].1);
    assert!(fake.operations.mutate("/main-worktrees/feature").is_ok());
}
#[tokio::test]
pub async fn test_削除中worktree_ブランチ名が不明でもパスごとに対象を残す() {
    // Given
    let fake = Arc::new(<FakeRepo as Default>::default());
    let repository = usecase(fake.clone());
    let mut deletions = Vec::new();
    for path in ["/main-worktrees/one", "/main-worktrees/two"] {
        let mut deletion = fake.operations.delete(path).await.unwrap();
        deletion
            .accept(
                crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                    repository_root: "/main".into(),
                    path: path.into(),
                    branch: None,
                },
            )
            .unwrap();
        deletions.push(deletion);
    }
    // When
    let rows = repository.with_deleting_worktrees(
        "/main",
        vec![
            wt("/main-worktrees/other", "unknown", false),
            wt("/main-worktrees/one", "feature", false),
        ],
    );
    // Then
    assert_eq!(rows.len(), 3);
    assert!(!rows[0].1);
    assert_eq!(rows[1].0.branch, "feature");
    assert!(rows[1].1);
    assert_eq!(rows[2].0.branch, "/main-worktrees/two");
    assert_eq!(rows[2].0.path, "/main-worktrees/two");
    assert!(rows[2].1);
    drop(deletions);
    assert!(deleting_rows(&repository).is_empty());
}
#[tokio::test]
pub async fn test_worktree削除_base後始末の失敗を返し削除中を解いて通知する() {
    let fake = Arc::new(FakeRepo {
        removed_branch: Some("feature".into()),
        fail_cleanup: true,
        ..Default::default()
    });
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let repository = usecase(fake.clone()).with_state_publisher(subscriptions);
    let error = repository
        .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "cleanup failed");
    assert!(deleting_rows(&repository).is_empty());
    assert!(fake.operations.mutate("/wt").is_ok());
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::target::StateChangeSource::Repository(vec![
            "/repo".into()
        ])
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::target::StateChangeSource::Repository(vec![
            "/repo".into()
        ])
    );
}
#[tokio::test]
pub async fn test_worktree削除を委譲する() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/r", "/wt", false)
        .await
        .unwrap();
    fake.wait_for_deletion("/wt").await;
    assert_eq!(
        *fake.removed_worktrees.lock(),
        vec![("/wt".to_string(), false)]
    );
}
#[tokio::test]
pub async fn test_worktree削除_紐づくterminal_surfaceを先に停止する() {
    let fake = Arc::new(<FakeRepo as Default>::default());
    usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/r", "/wt", false)
        .await
        .unwrap();
    fake.wait_for_deletion("/wt").await;
    // worktree 本体の削除（removed 0 件時点）より前に停止が呼ばれる。
    assert_eq!(
        *fake.killed_worktree_terminals.lock(),
        vec![("/wt".to_string(), 0)]
    );
    assert_eq!(
        *fake.removed_worktrees.lock(),
        vec![("/wt".to_string(), false)]
    );
}
#[tokio::test]
pub async fn test_worktree削除_削除失敗を返しterminal停止は実行する() {
    let fake = Arc::new(FakeRepo {
        fail_remove_worktree: true,
        ..<FakeRepo as Default>::default()
    });
    usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/r", "/wt", false)
        .await
        .unwrap_err();
    fake.wait_for_deletion("/wt").await;
    assert_eq!(
        *fake.killed_worktree_terminals.lock(),
        vec![("/wt".to_string(), 0)]
    );
    // 削除に失敗した場合は releash-base の後始末を行わない。
    assert!(fake.set_branch_base_override_calls.lock().is_empty());
}
#[tokio::test]
pub async fn test_worktree削除_archive失敗ではterminalとフォルダを削除しない() {
    // Given
    let fake = Arc::new(FakeRepo {
        fail_archive: true,
        ..Default::default()
    });
    // When
    let error = usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
        .await
        .unwrap_err();
    // Then
    assert_eq!(error.to_string(), "archive failed");
    assert!(fake.removed_worktrees.lock().is_empty());
    assert!(fake.killed_worktree_terminals.lock().is_empty());
}
#[tokio::test]
pub async fn test_worktree削除_archiveがフォルダ削除に先行する() {
    // Given
    let fake = Arc::new(<FakeRepo as Default>::default());
    // When
    usecase(fake.clone())
        .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
        .await
        .unwrap();
    fake.wait_for_deletion("/wt").await;
    // Then
    assert_eq!(
        *fake.archived_worktrees.lock(),
        vec![("/wt".to_string(), 0)]
    );
    assert_eq!(fake.removed_worktrees.lock().len(), 1);
}
#[tokio::test]
pub async fn test_worktree削除_所属不一致とlockedとdirtyではarchiveしない() {
    for (invalid, locked, dirty) in [(true, false, 0), (false, true, 0), (false, false, 1)] {
        // Given
        let mut worktree = wt("/wt", "feature", false);
        worktree.is_locked = locked;
        let fake = Arc::new(FakeRepo {
            fail_validate_removal: invalid,
            dirty,
            worktrees: vec![worktree],
            ..Default::default()
        });
        // When
        let result = usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await;
        // Then
        assert!(result.is_err());
        assert!(fake.archived_worktrees.lock().is_empty());
        assert!(fake.killed_worktree_terminals.lock().is_empty());
        assert!(fake.removed_worktrees.lock().is_empty());
    }
}
#[tokio::test]
pub async fn test_worktree削除_受理後も削除終了まで一覧と排他を保つ() {
    for fail in [false, true] {
        // Given
        let (release, blocked) = std::sync::mpsc::channel();
        let fake = Arc::new(FakeRepo {
            current_branch: "feature".into(),
            removed_branch: Some("feature".into()),
            fail_remove_worktree: fail,
            remove_continue: Some(Mutex::new(blocked)),
            ..Default::default()
        });
        let repository = usecase(fake.clone());
        // When
        let deletion = tokio::spawn({
            let repository = repository.clone();
            let fake = fake.clone();
            async move {
                repository
                    .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                    .await
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fake.remove_started.notified(),
        )
        .await
        .unwrap();
        // Then
        assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
        assert!(fake.operations.mutate("/wt").is_err());
        assert!(fake.operations.mutate("/other").is_ok());
        assert!(repository.get_repository_status_scan("/wt").is_ok());
        assert!(repository
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .is_err());
        let rows = deleting_rows(&repository);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.branch, "feature");
        assert_eq!(rows[0].0.path, "/wt");
        assert!(rows[0].1);
        assert!(fake.removed_worktrees.lock().is_empty());
        assert!(fake.set_branch_base_override_calls.lock().is_empty());
        assert!(!deletion.is_finished());
        release.send(()).unwrap();
        assert_eq!(deletion.await.unwrap().is_err(), fail);
        fake.wait_for_deletion("/wt").await;
        assert!(deleting_rows(&repository).is_empty());
        assert_eq!(fake.removed_worktrees.lock().len(), usize::from(!fail));
        assert_eq!(
            fake.set_branch_base_override_calls.lock().len(),
            usize::from(!fail)
        );
    }
}
#[tokio::test]
pub async fn test_worktree削除_base設定の後始末が終わるまで一覧と排他を保つ() {
    // Given
    let (release, blocked) = std::sync::mpsc::channel();
    let fake = Arc::new(FakeRepo {
        current_branch: "feature".into(),
        removed_branch: Some("feature".into()),
        cleanup_continue: Some(Mutex::new(blocked)),
        ..Default::default()
    });
    let repository = usecase(fake.clone());
    // When
    let deletion = tokio::spawn({
        let repository = repository.clone();
        let fake = fake.clone();
        async move {
            repository
                .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                .await
        }
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fake.cleanup_started.notified(),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(fake.removed_worktrees.lock().len(), 1);
    let rows = deleting_rows(&repository);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1);
    assert!(fake.operations.mutate("/wt").is_err());
    assert!(repository.get_repository_status_scan("/wt").is_ok());
    assert!(!deletion.is_finished());
    release.send(()).unwrap();
    deletion.await.unwrap().unwrap();
    fake.wait_for_deletion("/wt").await;
    assert!(deleting_rows(&repository).is_empty());
}
#[tokio::test]
pub async fn test_worktree削除_archive完了前には受理も背景削除も一覧追加もしない() {
    // Given
    let fake = Arc::new(FakeRepo {
        archive_continue: Some(tokio::sync::Notify::new()),
        ..Default::default()
    });
    let repository = usecase(fake.clone());
    let removal = repository.remove_worktree(fake.as_ref(), "/repo", "/wt", false);
    tokio::pin!(removal);
    // When
    assert!(futures_util::poll!(&mut removal).is_pending());
    // Then
    assert!(fake.removed_worktrees.lock().is_empty());
    assert!(fake.killed_worktree_terminals.lock().is_empty());
    assert!(deleting_rows(&repository).is_empty());
    fake.archive_continue.as_ref().unwrap().notify_one();
    removal.await.unwrap();
    fake.wait_for_deletion("/wt").await;
    assert_eq!(fake.removed_worktrees.lock().len(), 1);
}
#[tokio::test]
pub async fn test_worktree削除_ブランチ取得の停止を保持し後続操作へ進まない() {
    use crate::common::operation_context::OperationStopped;
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        for force in [false, true] {
            // Given
            let fake = Arc::new(FakeRepo {
                stop_current_branch: Some(stopped),
                ..Default::default()
            });
            let publisher = crate::test_support::state_subscription::test_subscriptions();
            let mut changes = crate::test_support::state_subscription::changes(&publisher);
            let repository = usecase(fake.clone()).with_state_publisher(publisher);
            // When
            let error = repository
                .remove_worktree(fake.as_ref(), "/repo", "/wt", force)
                .await
                .unwrap_err();
            // Then
            assert!(
                matches!(error, UsecaseError::Repository(crate::domain::repository::error::RepositoryError::Technical(ref actual)) if *actual == stopped.into())
            );
            assert!(fake.archived_worktrees.lock().is_empty());
            assert!(fake.killed_worktree_terminals.lock().is_empty());
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.set_branch_base_override_calls.lock().is_empty());
            assert!(changes.try_recv().is_err());
            assert!(deleting_rows(&repository).is_empty());
            assert!(fake.operations.mutate("/wt").is_ok());
        }
    }
}
#[tokio::test]
pub async fn test_worktree削除_ブランチ名を取得できなくてもarchive後に受理する() {
    for force in [false, true] {
        // Given
        let (release, blocked) = std::sync::mpsc::channel();
        let fake = Arc::new(FakeRepo {
            fail_current_branch: true,
            remove_continue: Some(Mutex::new(blocked)),
            ..Default::default()
        });
        let repository = usecase(fake.clone());
        // When
        let deletion = tokio::spawn({
            let repository = repository.clone();
            let fake = fake.clone();
            async move {
                repository
                    .remove_worktree(fake.as_ref(), "/repo", "/wt", force)
                    .await
            }
        });
        fake.remove_started.notified().await;
        // Then
        assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
        assert_eq!(
            *fake.killed_worktree_terminals.lock(),
            vec![("/wt".into(), 0)]
        );
        assert!(fake.removed_worktrees.lock().is_empty());
        assert!(fake.operations.mutate("/wt").is_err());
        let rows = deleting_rows(&repository);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.branch, "/wt");
        assert_eq!(rows[0].0.path, "/wt");
        assert!(rows[0].1);
        assert!(!deletion.is_finished());
        release.send(()).unwrap();
        deletion.await.unwrap().unwrap();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(*fake.removed_worktrees.lock(), vec![("/wt".into(), force)]);
        assert!(deleting_rows(&repository).is_empty());
    }
}
#[tokio::test]
pub async fn test_worktree削除_リポジトリ識別情報を取得できないときarchive前に拒否する() {
    // Given
    let fake = Arc::new(FakeRepo {
        fail_main_repo_path: true,
        ..Default::default()
    });
    let repository = usecase(fake.clone());
    // When
    assert!(repository
        .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
        .await
        .is_err());
    // Then
    assert!(fake.archived_worktrees.lock().is_empty());
    assert!(fake.removed_worktrees.lock().is_empty());
    assert!(fake.operations.mutate("/wt").is_ok());
}
#[tokio::test]
pub async fn test_repository更新_操作成功後だけ購読へ通知する() {
    use crate::usecase::state_subscription::target::StateChangeSource;
    // Given
    let fake = Arc::new(<FakeRepo as Default>::default());
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let uc = usecase(fake.clone()).with_state_publisher(publisher.clone());
    // When / Then
    uc.create_worktree("/repo", "feature", true, None).unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Repository(vec!["/repo".into()])
    );
    uc.set_releash_base("/repo", Some("main")).unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Repository(vec!["/repo".into()])
    );
    uc.set_branch_base_override("/repo", "feature", Some("main"))
        .unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Repository(vec!["/repo".into()])
    );
    uc.remove_worktree(fake.as_ref(), "/repo", "/wt", false)
        .await
        .unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Repository(vec!["/repo".into()])
    );
    fake.wait_for_deletion("/wt").await;
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Repository(vec!["/repo".into()])
    );
    let failed = Arc::new(FakeRepo {
        fail_create_worktree: true,
        fail_validate_removal: true,
        ..Default::default()
    });
    let failed_uc = usecase(failed.clone()).with_state_publisher(publisher);
    assert!(failed_uc
        .create_worktree("/repo", "feature", true, None)
        .is_err());
    assert!(failed_uc
        .remove_worktree(failed.as_ref(), "/repo", "/wt", false)
        .await
        .is_err());
    assert!(changes.try_recv().is_err());
}
