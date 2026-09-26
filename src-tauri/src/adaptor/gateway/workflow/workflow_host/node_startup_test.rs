use super::super::test_helpers::Fixture;
use super::*;

#[tokio::test]
async fn test_一時失敗の再起動_保存済み実行を読み直しleafと合成子を再構築する() {
    for nodes in [
        "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        "  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        "  main: {worktree: isolated, fanout: {children: [work]}}\n  work: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
    ] {
        // Given
        let fixture = Fixture::new(0);
        let started = fixture.persist_started(nodes, "/repo").await;
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let gateway = HostNodeStartup {
            host: &fixture.host, app: &fixture.app,
            execution_id: &started.execution_id, worktree_path: "/repo", cancelled,
        };
        let id = &started.node_executions[0].id;
        // When
        let start = gateway.restart(id, AttemptProgress::Continue).await.unwrap().unwrap();
        // Then
        assert_eq!(start.node_execution_id(), id);
        match start {
            NodeStart::Leaf(leaf) => {
                assert_eq!(leaf.node_name, "main");
                assert_eq!(leaf.kind, crate::domain::workflow::entities::workflow_execution::LeafKind::Session);
                assert!(!nodes.contains("children"));
            }
            NodeStart::InjectDelegate(_) => panic!("unexpected delegate injection"),
            NodeStart::PrepareComposite(_) => assert!(nodes.contains("children")),
        }
        fixture.host.abort_workflow_execution(&fixture.app, &started.execution_id, None).await.unwrap();
        assert!(gateway.restart(id, AttemptProgress::Continue).await.unwrap().is_none());
        assert!(fixture.sessions.prepared.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_起動再試行登録_読込の一時失敗を作業列で再試行し停止分類は終了する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::domain::local_event::LocalEventQueryError;
    use crate::usecase::workflow::node_startup::FailedNodeStart;

    for (kind, read_failure, should_start) in [
        (
            crate::domain::failure::Failure::Technical(
                crate::domain::failure::TechnicalFailureNature::Transient,
            ),
            ReadFailure::Query(LocalEventQueryError::QueryBusy),
            true,
        ),
        (
            crate::domain::failure::Failure::Business(
                crate::domain::failure::BusinessFailure::VersionConflict,
            ),
            ReadFailure::Query(LocalEventQueryError::QueryBusy),
            true,
        ),
        (
            crate::domain::failure::Failure::Technical(
                crate::domain::failure::TechnicalFailureNature::Transient,
            ),
            ReadFailure::Sqlite(rusqlite::ffi::SQLITE_IOERR),
            false,
        ),
    ] {
        let fixture = Fixture::new(0);
        let started = fixture
            .persist_started(
                "  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
                "/repo",
            )
            .await;
        let id = &started.node_executions[0].id;
        fixture.store.fail_next_read(read_failure);
        fixture
            .host
            .schedule_startup_retries(
                &fixture.app,
                &started.execution_id,
                "/repo",
                vec![FailedNodeStart {
                    id: id.clone(),
                    kind,
                }],
            )
            .await;
        fixture.wait_startup_retries().await;
        let activated = fixture.sessions.activated.lock().unwrap().clone();
        assert_eq!(activated.len(), usize::from(should_start));
        if should_start {
            assert_eq!(
                activated[0] == *id,
                kind == crate::domain::failure::Failure::Technical(
                    crate::domain::failure::TechnicalFailureNature::Transient
                )
            );
        }
        let observations = fixture.host.queue.failure_query().records(id).await;
        assert!(observations
            .iter()
            .any(|observation| observation.record.kind
                == if should_start {
                    crate::domain::failure::Failure::Technical(
                        crate::domain::failure::TechnicalFailureNature::Transient,
                    )
                } else {
                    crate::domain::failure::Failure::Technical(
                        crate::domain::failure::TechnicalFailureNature::Other,
                    )
                }));
    }
}
