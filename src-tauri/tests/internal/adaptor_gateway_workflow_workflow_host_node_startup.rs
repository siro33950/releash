use crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture;
use releash_lib::test_support::integration::platform::AttemptProgress;
use releash_lib::test_support::integration::workflow::FailedNodeStart;
use releash_lib::test_support::integration::workflow::HostNodeStartup;
use releash_lib::test_support::integration::workflow::NodeStart;
use releash_lib::test_support::integration::workflow::NodeStartupGateway;

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
                assert_eq!(leaf.kind, releash_lib::test_support::integration::workflow::LeafKind::Session);
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
async fn test_起動再試行登録_読込の一時失敗をやり直し停止分類は終了する() {
    use releash_lib::test_support::integration::persistence::ReadFailure;
    use releash_lib::test_support::integration::platform::LocalEventQueryError;

    for (kind, read_failure, should_start) in [
        (
            releash_lib::test_support::integration::platform::Failure::Technical(
                releash_lib::test_support::integration::platform::TechnicalFailureNature::Transient,
            ),
            ReadFailure::Query(LocalEventQueryError::QueryBusy),
            true,
        ),
        (
            releash_lib::test_support::integration::platform::Failure::Business(
                releash_lib::test_support::integration::platform::BusinessFailure::VersionConflict,
            ),
            ReadFailure::Query(LocalEventQueryError::QueryBusy),
            true,
        ),
        (
            releash_lib::test_support::integration::platform::Failure::Technical(
                releash_lib::test_support::integration::platform::TechnicalFailureNature::Transient,
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
                kind == releash_lib::test_support::integration::platform::Failure::Technical(
                    releash_lib::test_support::integration::platform::TechnicalFailureNature::Transient
                )
            );
        }
        let observations =
            releash_lib::test_support::integration::platform::shared_store().records(id);
        assert!(observations
            .iter()
            .all(|observation| observation.record.operation != "workflow_node_start"));
        let facts = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &started.execution_id,
        )
        .await
        .unwrap();
        assert_eq!(
            facts.iter().any(|record| matches!(
                record.fact,
                releash_lib::test_support::integration::workflow::NodeFact::RuntimeFailureObserved(
                    _
                )
            )),
            !should_start
        );
    }
}
