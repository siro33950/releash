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
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    use crate::usecase::agent_session::AgentSessionLaunchUsecaseError;
    // Given
    let cases = [
        (
            AgentSessionLaunchUsecaseError::ProviderUnavailable,
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::InvalidInput,
            F::InvalidInput,
        ),
        (
            AgentSessionLaunchUsecaseError::Conflict(
                crate::domain::failure::FailureKind::RestartRequired,
            ),
            F::RestartRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::StorageUnavailable,
            F::Temporary,
        ),
        (
            AgentSessionLaunchUsecaseError::LaunchUnavailable,
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::TerminalUnavailable,
            F::StateRequired,
        ),
        (AgentSessionLaunchUsecaseError::Corrupt, F::Corrupt),
        (
            AgentSessionLaunchUsecaseError::TerminalSpawn(
                crate::domain::agent_session::ProviderAgentTerminalSpawnError::OwnerConflict,
            ),
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::TerminalSpawn(
                crate::domain::agent_session::ProviderAgentTerminalSpawnError::PtySpawn {
                    error: "pty".into(),
                },
            ),
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::TerminalSpawn(
                crate::domain::agent_session::ProviderAgentTerminalSpawnError::OtherSpawnFailure {
                    error: "spawn".into(),
                },
            ),
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Temporary),
            F::Temporary,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Expired),
            F::Expired,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Missing),
            F::Missing,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Permission),
            F::Permission,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Capacity),
            F::Capacity,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Internal),
            F::Internal,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Corrupt),
            F::Corrupt,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Cancelled),
            F::Cancelled,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::Unknown),
            F::Unknown,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            AgentSessionLaunchUsecaseError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given / When / Then
    for (source, expected) in [
        (
            AgentSessionUsecaseError::Conflict,
            FailureKind::RestartRequired,
        ),
        (
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            FailureKind::StateRequired,
        ),
    ] {
        assert_eq!(super::map_session_error(source).failure_kind(), expected);
    }
}

#[test]
fn test_実行木登録の失敗_起動エラーへ変換しても元の分類を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given / When / Then
    for kind in [
        FailureKind::Internal,
        FailureKind::Temporary,
        FailureKind::Expired,
        FailureKind::StateRequired,
    ] {
        let error = super::map_execution_tree_registration_error(
            super::StartedExecutionTreeRegistrationError::Store(kind),
        );
        assert_eq!(error.failure_kind(), kind);
    }
}

#[test]
fn test_provider起動準備_停止分類をusecaseまで保持する() {
    use crate::common::operation_context::OperationStopped;
    use crate::domain::failure::ClassifiedFailure;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = super::map_launch_error(
            crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                stopped.into(),
            ),
        );
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
    }
}
