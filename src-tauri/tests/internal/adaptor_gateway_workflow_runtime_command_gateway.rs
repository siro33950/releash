use releashd::test_support::integration::workflow::WorkflowControlPlaneGateway;
use releashd::test_support::integration::workflow::WorkflowRuntimeCommandGateway;
use std::sync::Arc;

#[tokio::test]
async fn test_承認記録読取_実経路で失敗分類を保持する() {
    use releashd::test_support::integration::persistence::ReadFailure;
    use releashd::test_support::integration::transport::classified_error;
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    let gateway =
        WorkflowRuntimeCommandGateway::new_with_driver(fixture.app, Arc::new(fixture.host));
    for (failure, expected) in ReadFailure::cases() {
        gateway
            .test_app()
            .store
            .as_ref()
            .unwrap()
            .fail_next_read(failure);
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
    use releashd::test_support::integration::persistence::NewNodeEventRow;
    // Given
    let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
    fixture
        .app
        .store
        .as_ref()
        .unwrap()
        .append_node_event(
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
        .await
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
        releashd::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::DataLoss
    );
}
