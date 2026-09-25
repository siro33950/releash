use super::*;
use crate::adaptor::gateway::local_event_store::LocalEventStoreConfig;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{NodeFactMeta, SessionExecutionTreeRootFacts};

fn fixture() -> (
    tempfile::TempDir,
    Arc<LocalEventStore>,
    ExecutionTreeArchiveFactRepository,
    NodeFactMeta,
) {
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let facts = SessionExecutionTreeRootFacts::new(
        "00000000-0000-4000-8000-000000000001",
        "/repo",
        "/repo",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    let meta = facts.meta.clone();
    fact_log::append_fact_batch_for_seed(&store, &facts.into_facts(), 1, "seed-archive").unwrap();
    let repository = ExecutionTreeArchiveFactRepository::new(store.clone(), directory.path());
    (directory, store, repository, meta)
}

#[tokio::test]
async fn test_実行木archive_終了前は拒否して終了後の理由と時刻を事実から読む() {
    // Given
    let (directory, store, repository, meta) = fixture();
    let id = ExecutionTreeId::new(meta.tree_id.clone()).unwrap();
    // When / Then
    assert!(repository.archive(&id, 123.456789, "manual").await.is_err());
    assert!(repository
        .archive_snapshot_for(&[meta.tree_id.clone()])
        .await
        .unwrap()
        .records
        .is_empty());
    fact_log::append_single_fact(
        &store,
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    repository
        .archive(&id, 123.456789, "worktree_removed")
        .await
        .unwrap();
    let records = repository
        .archive_snapshot_for(&[meta.tree_id.clone()])
        .await
        .unwrap()
        .records;
    assert_eq!(
        records,
        vec![ExecutionTreeArchiveRecord {
            execution_id: meta.tree_id.clone(),
            archived_at: 123.456789,
            archive_reason: "worktree_removed".into(),
        }]
    );
    repository.archive(&id, 200.0, "manual").await.unwrap();
    assert_eq!(
        repository
            .archive_snapshot_for(&[meta.tree_id])
            .await
            .unwrap()
            .records,
        records
    );
    assert!(!directory
        .path()
        .join("workflow_execution_archives.json")
        .exists());
}

#[tokio::test]
async fn test_実行木restore_終了状態を保ちsessionはpausedになる() {
    // Given
    let (_directory, store, repository, meta) = fixture();
    let id = ExecutionTreeId::new(meta.tree_id.clone()).unwrap();
    fact_log::append_single_fact(
        &store,
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    repository.archive(&id, 3.0, "manual").await.unwrap();
    // When
    repository.restore(&id, 4.0).await.unwrap();
    // Then
    assert!(repository
        .archive_snapshot_for(&[meta.tree_id.clone()])
        .await
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        repository.target(&meta.tree_id).await.unwrap().status,
        crate::domain::workflow::ExecutionStatus::Aborted
    );
    let records = fact_log::read_tree_records(&store, &meta.tree_id)
        .await
        .unwrap();
    let session = crate::domain::workflow::services::fact_replay::derive_session_facts(
        &records,
        &meta.node_execution_id,
        "00000000-0000-4000-8000-000000000001",
    );
    assert!(!session.archived);
    assert!(session.exited);
    assert!(!records
        .iter()
        .any(|record| matches!(record.fact, NodeFact::ResumeRequested)));
}

#[tokio::test]
async fn test_完了済み旧archive_元の時刻を保って移行し次回候補から外す() {
    // Given
    let (_directory, store, repository, meta) = fixture();
    let id = ExecutionTreeId::new(meta.tree_id.clone()).unwrap();
    for (kind, timestamp) in [
        ("submit_received", 1000),
        ("stop_received", 2000),
        ("archive_requested", 42000),
    ] {
        let mut pending = fact_log::pending_single_fact(
            &meta,
            &NodeFact::AbortRequested(Default::default()),
            timestamp,
        )
        .unwrap();
        pending.row.event_type = kind.into();
        pending.row.detail = "{}".into();
        fact_log::append_pending_rows(&store, vec![pending])
            .await
            .unwrap();
    }
    assert_eq!(
        repository.target(id.as_str()).await.unwrap().status,
        crate::domain::workflow::ExecutionStatus::Completed
    );
    assert_eq!(
        repository
            .legacy_session_archive_page(None)
            .await
            .unwrap()
            .len(),
        1
    );
    // When
    repository
        .archive(&id, 100.0, "worktree_removed")
        .await
        .unwrap();
    // Then
    let snapshot = repository
        .archive_snapshot_for(&[id.to_string()])
        .await
        .unwrap();
    assert_eq!(snapshot.records[0].archived_at, 42.0);
    assert_eq!(snapshot.records[0].archive_reason, "manual");
    assert!(repository
        .legacy_session_archive_page(None)
        .await
        .unwrap()
        .is_empty());
    let facts = fact_log::read_tree_records(&store, id.as_str())
        .await
        .unwrap();
    assert!(!facts
        .iter()
        .any(|record| matches!(record.fact, NodeFact::AbortRequested(_))));
    repository.archive(&id, 200.0, "manual").await.unwrap();
    assert_eq!(
        fact_log::read_tree_records(&store, id.as_str())
            .await
            .unwrap(),
        facts
    );
}

#[test]
fn test_旧archive記録_理由と時刻を保って読み移行完了後だけ削除する() {
    // Given
    let (directory, _, repository, meta) = fixture();
    let path = directory.path().join("workflow_execution_archives.json");
    std::fs::write(
        &path,
        serde_json::json!({"executions": {
            meta.tree_id.clone(): {"archivedAt": 15.125, "archiveReason": "worktree_removed"},
            "restored": {"restoredAt": 20.0}
        }})
        .to_string(),
    )
    .unwrap();
    // When
    let records = repository.legacy_archives().unwrap();
    // Then
    assert_eq!(
        records,
        vec![ExecutionTreeArchiveRecord {
            execution_id: meta.tree_id,
            archived_at: 15.125,
            archive_reason: "worktree_removed".into()
        }]
    );
    assert!(path.exists());
    repository.finish_legacy_migration().unwrap();
    assert!(!path.exists());
    assert!(repository.legacy_archives().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木restore_archiveされていない実行に終了事実を追加しない() {
    let (_directory, store, repository, meta) = fixture();
    let before = fact_log::read_tree_records(&store, &meta.tree_id)
        .await
        .unwrap();
    repository
        .restore(&ExecutionTreeId::new(meta.tree_id.clone()).unwrap(), 2.0)
        .await
        .unwrap();
    assert_eq!(
        fact_log::read_tree_records(&store, &meta.tree_id)
            .await
            .unwrap(),
        before
    );
}

#[test]
fn test_旧archive記録_従来許可していた空objectを受理し破損jsonは保持する() {
    let (directory, _, repository, _) = fixture();
    let path = directory.path().join("workflow_execution_archives.json");
    std::fs::write(&path, "{}").unwrap();
    assert!(repository.legacy_archives().unwrap().is_empty());
    std::fs::write(&path, "invalid json").unwrap();
    assert!(repository.legacy_archives().is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid json");
}

#[tokio::test]
async fn test_実行木archive対象_消失pathでも末尾スラッシュを正規化する() {
    // Given
    let (_directory, _store, repository, meta) = fixture();
    // When / Then
    assert_eq!(
        repository
            .worktree_target_page("/repo/", None)
            .await
            .unwrap()[0]
            .execution_id,
        meta.tree_id
    );
    assert!(repository
        .worktree_target_page("/repo-other", None)
        .await
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn test_実行木archive対象_git削除と同じ実体のpathとworkspace表記を照合する() {
    // Given
    let (directory, store, repository, _) = fixture();
    let worktree = directory.path().join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(&worktree, &alias).unwrap();
    for (id, workspace, path) in [
        (
            "session-path",
            "/workspace".to_string(),
            worktree.to_string_lossy().into_owned(),
        ),
        (
            "session-workspace",
            worktree.to_string_lossy().into_owned(),
            "/isolated".to_string(),
        ),
        (
            "session-alias",
            "/workspace".to_string(),
            alias.to_string_lossy().into_owned(),
        ),
    ] {
        let facts =
            SessionExecutionTreeRootFacts::new(id, workspace, path, ProviderKind::Codex, None)
                .unwrap();
        fact_log::append_fact_batch_for_seed(&store, &facts.into_facts(), 1, id).unwrap();
    }
    // When / Then
    for input in [
        format!("{}/", worktree.display()),
        format!("{}/./", alias.display()),
    ] {
        let mut ids = repository
            .worktree_target_page(&input, None)
            .await
            .unwrap()
            .into_iter()
            .map(|target| target.execution_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["session-alias", "session-path", "session-workspace"]);
    }
    let invalid = directory.path().join("symlink-loop");
    std::os::unix::fs::symlink(&invalid, &invalid).unwrap();
    assert!(repository
        .worktree_target_page(invalid.to_str().unwrap(), None)
        .await
        .is_err());
}

#[tokio::test]
async fn test_archive候補_履歴や定義をfoldせずページングしgcではarchive済みを除外する() {
    // Given
    let (_directory, store, repository, meta) = fixture();
    for index in 0..260 {
        let id = format!("tree-{index:03}");
        let facts =
            SessionExecutionTreeRootFacts::new(&id, "/repo", "/repo", ProviderKind::Codex, None)
                .unwrap();
        let meta = facts.meta.clone();
        let mut rows = facts
            .into_facts()
            .iter()
            .map(|(meta, fact)| fact_log::pending_single_fact(meta, fact, 1).unwrap())
            .collect::<Vec<_>>();
        rows[0].row.detail = rows[0]
            .row
            .detail
            .replace("\"definition\":{", "\"unreadableDefinition\":{");
        let mut corrupt =
            fact_log::pending_single_fact(&meta, &NodeFact::AbortRequested(Default::default()), 2)
                .unwrap();
        corrupt.row.event_type = "process_exited".into();
        corrupt.row.detail = "broken history".into();
        rows.push(corrupt);
        fact_log::append_pending_rows(&store, rows).await.unwrap();
    }
    fact_log::append_single_fact(
        &store,
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    repository
        .archive(&ExecutionTreeId::new(&meta.tree_id).unwrap(), 3.0, "manual")
        .await
        .unwrap();
    // When
    let mut ids = Vec::new();
    let mut after = None;
    loop {
        let page = repository.candidate_page(after.as_deref()).await.unwrap();
        assert!(page.len() <= 128);
        let Some(last) = page.last() else { break };
        after = Some(last.execution_id.clone());
        ids.extend(page.into_iter().map(|target| target.execution_id));
    }
    // Then
    assert_eq!(ids.len(), 260);
    assert!(!ids.contains(&meta.tree_id));
    let mut count = 0;
    let mut after = None;
    loop {
        let page = repository
            .worktree_target_page("/repo", after.as_deref())
            .await
            .unwrap();
        assert!(page.len() <= 128);
        let Some(last) = page.last() else { break };
        after = Some(last.execution_id.clone());
        count += page.len();
    }
    assert_eq!(count, 261);
}

#[cfg(unix)]
#[tokio::test]
async fn test_worktreearchive候補_対象外の壊れたpathを解決せずページ内の対象だけを返す() {
    // Given
    let (directory, store, repository, meta) = fixture();
    let broken = directory.path().join("broken");
    std::os::unix::fs::symlink(&broken, &broken).unwrap();
    for index in 0..260 {
        let id = format!("other-{index:03}");
        let facts = SessionExecutionTreeRootFacts::new(
            &id,
            broken.to_str().unwrap(),
            broken.to_str().unwrap(),
            ProviderKind::Codex,
            None,
        )
        .unwrap();
        fact_log::append_fact_batch_for_seed(&store, &facts.into_facts(), 1, &id).unwrap();
    }
    // When / Then
    let page = repository
        .worktree_target_page("/repo", None)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].execution_id, meta.tree_id);
    assert!(repository
        .worktree_target_page("/repo", Some(&meta.tree_id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_repository所属の記録_追記を繰り返さず再読込後も同じ所属を返す() {
    // Given
    let (directory, store, repository, meta) = fixture();
    let before = fact_log::read_tree_records(&store, &meta.tree_id)
        .await
        .unwrap();
    // When
    repository
        .record_repository_root(&meta.tree_id, "/owner", 2.0)
        .await
        .unwrap();
    repository
        .record_repository_root(&meta.tree_id, "/owner", 3.0)
        .await
        .unwrap();
    let reader = ExecutionTreeArchiveFactRepository::from_backend(FactLogReadBackend::ReadOnly(
        crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
            directory.path(),
        )
        .unwrap(),
    ));
    // Then
    let after = fact_log::read_tree_records(&store, &meta.tree_id)
        .await
        .unwrap();
    assert_eq!(&after[..before.len()], before);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(
        after.last().unwrap().fact,
        NodeFact::RepositoryRootObserved("/owner".into())
    );
    assert_eq!(
        reader.candidate_page(None).await.unwrap()[0]
            .repository_root
            .as_deref(),
        Some("/owner")
    );
    assert_eq!(
        reader
            .location(&meta.tree_id)
            .await
            .unwrap()
            .repository_root
            .as_deref(),
        Some("/owner")
    );
    assert_eq!(
        reader
            .target(&meta.tree_id)
            .await
            .unwrap()
            .repository_root
            .as_deref(),
        Some("/owner")
    );
    assert!(repository
        .record_repository_root(&meta.tree_id, "/other", 4.0)
        .await
        .is_err());
    assert!(repository
        .record_repository_root("missing", "/owner", 4.0)
        .await
        .is_err());
    assert_eq!(
        fact_log::read_tree_records(&store, &meta.tree_id)
            .await
            .unwrap(),
        after
    );
}

#[tokio::test]
async fn test_repository所属の記録_読取専用では保存失敗を返す() {
    // Given
    let (directory, _store, _repository, meta) = fixture();
    let reader = ExecutionTreeArchiveFactRepository::from_backend(FactLogReadBackend::ReadOnly(
        crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
            directory.path(),
        )
        .unwrap(),
    ));
    // When / Then
    assert!(reader
        .record_repository_root(&meta.tree_id, "/owner", 2.0)
        .await
        .is_err());
    assert!(reader
        .location(&meta.tree_id)
        .await
        .unwrap()
        .repository_root
        .is_none());
}

#[tokio::test]
async fn test_repository所属の復元_フォルダ消失済みでも旧隔離worktreeの事実を参照する() {
    // Given
    let (_directory, store, repository, meta) = fixture();
    let mut pending =
        fact_log::pending_single_fact(&meta, &NodeFact::AbortRequested(Default::default()), 2)
            .unwrap();
    pending.row.event_type = "isolated_worktree_created".into();
    pending.row.parent_id = Some(meta.node_execution_id.clone());
    pending.row.node_execution_id = "isolated-child".into();
    pending.row.detail = serde_json::json!({
        "repositoryRoot": "/owner",
        "worktreePath": "/owner-worktrees/.releash-isolated/child",
        "branch": "child"
    })
    .to_string();
    fact_log::append_pending_rows(&store, vec![pending])
        .await
        .unwrap();
    // When / Then
    assert_eq!(
        repository.candidate_page(None).await.unwrap()[0]
            .repository_root
            .as_deref(),
        Some("/owner")
    );
    repository
        .record_repository_root(&meta.tree_id, "/owner", 3.0)
        .await
        .unwrap();
    assert_eq!(
        repository
            .target(&meta.tree_id)
            .await
            .unwrap()
            .repository_root
            .as_deref(),
        Some("/owner")
    );
}

#[tokio::test]
async fn test_archive読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::protocol::connect::classified_error;
    // Given
    let (_directory, store, repository, meta) = fixture();
    let id = ExecutionTreeId::new(meta.tree_id.clone()).unwrap();
    for (failure, expected) in ReadFailure::cases() {
        for operation in 0..6 {
            store.fail_next_read(failure.clone());
            // When
            let result = match operation {
                0 => repository.candidate_page(None).await.map(|_| ()),
                1 => repository.location(&meta.tree_id).await.map(|_| ()),
                2 => repository
                    .legacy_session_archive_page(None)
                    .await
                    .map(|_| ()),
                3 => repository
                    .archive_snapshot_for(&[meta.tree_id.clone()])
                    .await
                    .map(|_| ()),
                4 => repository.archive(&id, 2.0, "manual").await,
                _ => {
                    repository
                        .append(&meta.tree_id, NodeFact::RestoreRequested, 2.0)
                        .await
                }
            };
            // Then
            assert_eq!(
                classified_error(result.unwrap_err()).code,
                expected,
                "operation {operation}"
            );
        }
    }
}

#[tokio::test]
async fn test_archive候補_repo補完の期限と取消を保持し次のpathへ進まない() {
    use crate::common::operation_context::{
        Cancellation, Deadline, OperationContext, OperationStopped,
    };
    use crate::domain::failure::ClassifiedFailure;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    struct CountCancelled(AtomicUsize);
    impl Cancellation for CountCancelled {
        fn is_cancelled(&self) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst);
            true
        }
    }
    for expire in [false, true] {
        // Given: store読取の待機中にcontextを切り替え、repo補完で初めて停止させる。
        let (_directory, store, repository, _) = fixture();
        let mut blockers = Vec::new();
        let mut releases = Vec::new();
        for _ in 0..crate::adaptor::gateway::local_event_store::reader::READER_POOL_SIZE {
            let store = store.clone();
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            blockers.push(tokio::spawn(async move {
                store
                    .submit_query(move |_| {
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(2)).unwrap();
                        Ok(())
                    })
                    .await
                    .unwrap();
            }));
            ready.await.unwrap();
            releases.push(release);
        }
        let mut page = Box::pin(repository.candidate_page(None));
        assert!(futures_util::poll!(&mut page).is_pending());
        for release in releases {
            release.send(()).unwrap();
        }
        for blocker in blockers {
            blocker.await.unwrap();
        }
        let cancellation = Arc::new(CountCancelled(AtomicUsize::new(0)));
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now())),
            cancellation.clone(),
        );
        // When
        let error = crate::common::operation_context::scope(context, page)
            .await
            .unwrap_err();
        // Then
        let expected = if expire {
            OperationStopped::Expired
        } else {
            OperationStopped::Cancelled
        };
        assert!(
            matches!(error, WorkflowError::Technical(ref stopped) if *stopped == expected.into())
        );
        assert_eq!(error.failure_kind(), expected.failure_kind());
        assert_eq!(cancellation.0.load(Ordering::SeqCst), usize::from(!expire));
    }
}
