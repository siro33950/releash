use super::*;

#[tokio::test]
pub async fn test_startup_gc結線_消失候補の未終了実行木をabortしてarchiveする() {
    use crate::adaptor::gateway::workflow::fact_log;
    use crate::adaptor::gateway::workflow::workflow_host::test_helpers::archive_fixture;
    use crate::domain::workflow::{
        ExecutionStatus, ExecutionTreeArchiveRepository, NodeFact, SessionExecutionTreeRootFacts,
    };
    // Given
    let fixture = archive_fixture();
    let repo = tempfile::tempdir().unwrap();
    git2::Repository::init(repo.path()).unwrap();
    let path = repo.path().join("removed-worktree");
    std::fs::create_dir(&path).unwrap();
    let id = "00000000-0000-4000-8000-000000000995";
    let mut facts = SessionExecutionTreeRootFacts::new(
        id,
        path.to_str().unwrap(),
        path.to_str().unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    if let NodeFact::Started(started) = &mut facts.started {
        started.root.as_mut().unwrap().repository_root =
            Some(repo.path().to_string_lossy().into_owned());
    }
    fact_log::append_fact_batch_for_seed(&fixture.store, &facts.into_facts(), 1, id).unwrap();
    let composition = ProductionAppDataComposition::new(
        fixture.directory.path().into(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    );
    // When
    let report = composition
        .run_startup_gc_pass(
            Arc::new(parking_lot::RwLock::new(vec![repo
                .path()
                .to_string_lossy()
                .into_owned()])),
            fixture.store.clone(),
            Arc::new(fixture.runtime),
        )
        .await
        .unwrap();
    // Then
    assert_eq!(report.errors, 0);
    assert!(path.exists());
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
}
