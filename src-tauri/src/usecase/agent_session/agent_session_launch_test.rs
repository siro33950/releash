use super::{
    wait_for_activation, StandaloneLaunchRequestRegistry, COMPLETED_STANDALONE_LAUNCH_CAPACITY,
};

#[test]
fn test_冪等レジストリ_完了記録が容量を超えると最古のrequest_idを追い出す() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..=COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }

    assert_eq!(registry.recall_completed("request-0"), None);
    assert_eq!(
        registry.recall_completed("request-1"),
        Some(Ok("agent-1".to_string()))
    );
    assert_eq!(
        registry.recall_completed(&format!("request-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")),
        Some(Ok(format!("agent-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")))
    );
}

#[test]
fn test_冪等レジストリ_recall済みrequest_idは追い出し順が更新され残存する() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }
    assert!(registry.recall_completed("request-0").is_some());

    registry.record_completed("request-new".to_string(), Ok("agent-new".to_string()));

    assert_eq!(
        registry.recall_completed("request-0"),
        Some(Ok("agent-0".to_string()))
    );
    assert_eq!(registry.recall_completed("request-1"), None);
    assert_eq!(
        registry.recall_completed("request-new"),
        Some(Ok("agent-new".to_string()))
    );
}

#[tokio::test]
async fn test_workflow_activation待機_sender消失を完了として扱わない() {
    let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
    drop(completion_tx);

    assert!(!wait_for_activation(completion_rx).await);
}

#[tokio::test]
async fn test_workflow_activation待機_true通知を完了として扱う() {
    let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
    completion_tx.send(true).unwrap();

    assert!(wait_for_activation(completion_rx).await);
}

#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;

    // Given / When / Then
    for source in [
        AgentSessionUsecaseError::Conflict,
        AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
            agent_session_id: "owner".into(),
        },
    ] {
        let expected = source.clone().into();
        assert_eq!(
            super::map_session_error(source),
            super::AgentSessionLaunchUsecaseError::Conflict(expected)
        );
    }
}

#[test]
fn test_実行木登録の失敗_起動エラーへ変換しても元の分類を保持する() {
    // Given / When / Then
    for source in [
        crate::domain::local_event::LocalEventQueryError::Internal {
            correlation_id: "id".into(),
        },
        crate::domain::local_event::LocalEventQueryError::QueryBusy,
        crate::domain::local_event::LocalEventQueryError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "timeout".into(),
            },
        ),
        crate::domain::local_event::LocalEventQueryError::IncompatibleStoredEvent {
            correlation_id: "id".into(),
        },
    ] {
        let expected = source.clone().into();
        let error = super::map_execution_tree_registration_error(
            super::StartedExecutionTreeRegistrationError::Store(source.into()),
        );
        assert_eq!(
            error,
            super::AgentSessionLaunchUsecaseError::Store(expected)
        );
    }
}

#[test]
fn test_provider起動準備_停止分類をusecaseまで保持する() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = super::map_launch_error(
            crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                stopped.into(),
            ),
        );
        // Then
        assert!(
            matches!(error, super::AgentSessionLaunchUsecaseError::Technical(ref actual) if *actual == stopped.into())
        );
    }
}
