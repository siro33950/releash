use super::*;

#[tokio::test]
async fn test_承認記録読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::protocol::connect::classified_error;
    // Given
    let fixture = super::super::workflow_host::test_helpers::Fixture::new(0);
    let gateway =
        WorkflowRuntimeCommandGateway::new_with_driver(fixture.app, Arc::new(fixture.host));
    for (failure, expected) in ReadFailure::cases() {
        gateway.app.store.as_ref().unwrap().fail_next_read(failure);
        // When
        let error = gateway
            .approval_persisted("tree", "main", None)
            .await
            .unwrap_err();
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}

#[tokio::test]
async fn test_承認記録読取_保存された事実の破損をdata_lossとして返す() {
    use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
    // Given
    let fixture = super::super::workflow_host::test_helpers::Fixture::new(0);
    fixture
        .app
        .store
        .as_ref()
        .unwrap()
        .append_node_event_blocking(
            NewNodeEventRow {
                tree_id: "tree".into(),
                node_execution_id: "node".into(),
                parent_id: None,
                node_name: "main".into(),
                kind: "session".into(),
                attempt: 1,
                event_type: "approval_granted".into(),
                session_id: None,
                detail: "{".into(),
            },
            Some(1000),
        )
        .unwrap();
    let gateway =
        WorkflowRuntimeCommandGateway::new_with_driver(fixture.app, Arc::new(fixture.host));
    // When
    let error = gateway
        .approval_persisted("tree", "main", None)
        .await
        .unwrap_err();
    // Then
    assert_eq!(
        crate::adaptor::protocol::connect::classified_error(error).code,
        connectrpc::ErrorCode::DataLoss
    );
}
