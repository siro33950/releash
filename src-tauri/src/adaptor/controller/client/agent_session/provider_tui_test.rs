use super::*;
use crate::usecase::agent_session::{
    AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecaseError,
};
use crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError;

#[tokio::test(flavor = "current_thread")]
async fn test_provider_availability_controller_blocking操作中もasync_runtimeを占有しない() {
    let heartbeat = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let heartbeat_task = {
        let heartbeat = heartbeat.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            heartbeat.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };

    run_provider_availability_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(())
    })
    .await
    .unwrap();
    heartbeat_task.await.unwrap();

    assert!(heartbeat.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn test_agent_session_controller_domain結果をwire語彙へ変換する() {
    assert_eq!(
        AgentSessionOpenResponse::from(AgentSessionOpenOutcome::Indeterminate),
        AgentSessionOpenResponse::Indeterminate
    );
}

#[test]
fn test_agent_session_controller_provider未選択と未知値を起動前に拒否する() {
    assert_eq!(
        parse_provider("claude", ProviderParseOperation::Start).unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Claude
    );
    assert_eq!(
        parse_provider("codex", ProviderParseOperation::Start).unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Codex
    );
    let missing =
        serde_json::to_value(parse_provider("", ProviderParseOperation::Start).unwrap_err())
            .unwrap();
    let unknown =
        serde_json::to_value(parse_provider("unknown", ProviderParseOperation::Start).unwrap_err())
            .unwrap();
    assert_eq!(missing["code"], "AGENT_SESSION_INVALID_PROVIDER");
    assert_eq!(unknown["code"], "AGENT_SESSION_INVALID_PROVIDER");
}

#[test]
fn test_agent_session_controller_terminal_spawn詳細を既存の利用者向けerrorへ変換する() {
    // Given
    let internal_error = "Failed to spawn shell: Permission denied (os error 13)";

    // When
    let error = launch_error(
        AgentSessionLaunchUsecaseError::TerminalSpawn(
            crate::domain::agent_session::ProviderAgentTerminalSpawnError::PtySpawn {
                error: internal_error.to_string(),
            },
        ),
        AgentSessionLaunchOperation::Start,
    );

    // Then
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "AGENT_SESSION_TERMINAL_UNAVAILABLE");
    assert_eq!(
        value["message"],
        "Releash could not complete the Terminal operation for this AgentSession. Try again."
    );
    assert!(!value.to_string().contains(internal_error));
}

#[test]
fn test_agent_session_controller_対象21codeを利用者向け英語文言へ変換する() {
    // Given
    let cases = [
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::InvalidInput),
            "PROVIDER_AVAILABILITY_INVALID_EXECUTABLE",
            "Enter a Provider executable command name or path.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::ConfigUnavailable),
            "PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE",
            "Releash could not access the Provider executable setting. Try again.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::RefreshUnavailable),
            "PROVIDER_AVAILABILITY_REFRESH_UNAVAILABLE",
            "Releash could not refresh Provider CLI availability. Try again.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::Corrupt),
            "PROVIDER_AVAILABILITY_CORRUPT",
            "Releash could not read Provider CLI availability. Restart Releash and try again.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::Start).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before starting the AgentSession.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::ProviderUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_PROVIDER_UNAVAILABLE",
            "The selected Provider is unavailable. Check its executable and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not start the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict((crate::domain::local_event::CommitBatchError::TreeHeadConflict).into()),
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::StorageUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Releash could not access saved AgentSession data. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::LaunchUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Releash could not complete the Provider operation for this AgentSession. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::TerminalUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Releash could not complete the Terminal operation for this AgentSession. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Corrupt,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CORRUPT",
            "Releash could not continue because the AgentSession data is invalid.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::NotFound),
            "AGENT_SESSION_NOT_FOUND",
            "The AgentSession is no longer available.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::InvalidOperation),
            "AGENT_SESSION_INVALID_OPERATION",
            "This operation is not available for the AgentSession in its current state. Refresh and try again.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::InvalidInput),
            "PROVIDER_HOOK_HEALTH_INVALID_REQUEST",
            "Releash could not load Provider Hook health because the request is invalid.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::StorageUnavailable),
            "PROVIDER_HOOK_HEALTH_STORAGE_UNAVAILABLE",
            "Releash could not load Provider Hook health. Try again.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::Corrupt),
            "PROVIDER_HOOK_HEALTH_CORRUPT",
            "Releash could not load Provider Hook health because its saved data is invalid.",
        ),
    ];

    // When / Then
    for (error, expected_code, expected_message) in cases {
        assert_coded_error(error, expected_code, expected_message);
    }
}

#[test]
fn test_agent_session_controller_操作依存codeを操作ごとの固定文言へ変換する() {
    // Given
    let cases = [
        (
            parse_provider("unknown", ProviderParseOperation::ConfigureProvider)
                .unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a valid Provider.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::Start).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before starting the AgentSession.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::ResumeHistory).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before resuming the AgentSession.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not start the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::ResumeHistory,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not resume the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict((crate::domain::local_event::CommitBatchError::TreeHeadConflict).into()),
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict((crate::domain::local_event::CommitBatchError::TreeHeadConflict).into()),
                AgentSessionLaunchOperation::ResumeHistory,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be resumed because it changed or its Provider session is already in use. Refresh and try again.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::Conflict((crate::domain::local_event::CommitBatchError::TreeHeadConflict).into())),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be updated because it changed or its Provider session is already in use. Refresh and try again.",
        ),
    ];

    // When / Then
    for (error, expected_code, expected_message) in cases {
        assert_coded_error(error, expected_code, expected_message);
    }
}

#[test]
fn test_agent_session_controller_共有codeは全usecase_error経路で同じ文言になる() {
    // Given / When / Then
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::StorageUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::StorageUnavailable),
        ],
        "AGENT_SESSION_STORAGE_UNAVAILABLE",
        "Releash could not access saved AgentSession data. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::LaunchUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::LaunchUnavailable),
        ],
        "AGENT_SESSION_LAUNCH_UNAVAILABLE",
        "Releash could not complete the Provider operation for this AgentSession. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::TerminalUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::TerminalUnavailable),
        ],
        "AGENT_SESSION_TERMINAL_UNAVAILABLE",
        "Releash could not complete the Terminal operation for this AgentSession. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::Corrupt,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::Corrupt),
        ],
        "AGENT_SESSION_CORRUPT",
        "Releash could not continue because the AgentSession data is invalid.",
    );
}

fn assert_coded_errors<const N: usize>(
    errors: [AppError; N],
    expected_code: &str,
    expected_message: &str,
) {
    for error in errors {
        assert_coded_error(error, expected_code, expected_message);
    }
}

fn assert_coded_error(error: AppError, expected_code: &str, expected_message: &str) {
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({
            "code": expected_code,
            "message": expected_message,
        })
    );
}

#[test]
fn test_provider失敗分類_表示コードの生成時に理由を保持する() {
    use super::ProviderTuiCodedError as E;
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode as F;
    // Given
    let cases = [
        (E::ProviderAvailabilityInvalidExecutable, F::InvalidArgument),
        (E::ProviderAvailabilityConfigUnavailable, F::Unavailable),
        (E::ProviderAvailabilityRefreshUnavailable, F::Unavailable),
        (E::ProviderAvailabilityCorrupt, F::DataLoss),
        (
            E::AgentSessionInvalidProvider(super::ProviderParseOperation::Start),
            F::InvalidArgument,
        ),
        (E::AgentSessionProviderUnavailable, F::FailedPrecondition),
        (
            E::AgentSessionInvalidInput(super::AgentSessionLaunchOperation::Start),
            F::InvalidArgument,
        ),
        (
            E::AgentSessionConflict(
                crate::adaptor::presenter::provider_tui::AgentSessionConflictOperation::Start,
            ),
            F::Aborted,
        ),
        (E::AgentSessionStorageUnavailable, F::Unavailable),
        (
            E::AgentSessionLaunchUnavailable(F::FailedPrecondition),
            F::FailedPrecondition,
        ),
        (
            E::AgentSessionTerminalUnavailable(F::FailedPrecondition),
            F::FailedPrecondition,
        ),
        (E::AgentSessionCorrupt, F::DataLoss),
        (E::AgentSessionNotFound, F::NotFound),
        (E::AgentSessionInvalidOperation, F::FailedPrecondition),
        (E::ProviderHookHealthInvalidRequest, F::InvalidArgument),
        (E::ProviderHookHealthStorageUnavailable, F::Unavailable),
        (E::ProviderHookHealthCorrupt, F::DataLoss),
    ];
    for (error, expected) in cases {
        // When
        let error = super::provider_tui_coded_error(error);
        // Then
        assert_eq!(error.connect_code(), expected);
    }
}

#[derive(Clone, Copy, Debug)]
enum HistoryFailurePoint {
    Metadata,
    Ownership,
    Titles,
}

struct FailingHistoryPorts {
    point: HistoryFailurePoint,
    kind: crate::domain::failure::StorageFailure,
}

#[async_trait::async_trait]
impl crate::domain::agent_session::AgentSessionHistoryGateway for FailingHistoryPorts {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        _: usize,
    ) -> Result<
        Vec<crate::domain::agent_session::AgentSessionHistoryMetadata>,
        crate::domain::agent_session::AgentSessionHistoryGatewayError,
    > {
        if matches!(self.point, HistoryFailurePoint::Metadata) {
            return Err(
                crate::domain::agent_session::AgentSessionHistoryGatewayError::Store(
                    self.kind.clone(),
                ),
            );
        }
        Ok(vec![
            crate::domain::agent_session::AgentSessionHistoryMetadata {
                provider,
                provider_session_id: "session".into(),
                worktree_path: worktree_path.into(),
                updated_at_ms: 1,
            },
        ])
    }

    async fn list_session_titles(
        &self,
        _: ProviderKind,
        _: &str,
        _: &[String],
    ) -> Result<
        Vec<crate::domain::agent_session::ProviderSessionTitleEntry>,
        crate::domain::agent_session::AgentSessionHistoryGatewayError,
    > {
        assert!(matches!(self.point, HistoryFailurePoint::Titles));
        Err(crate::domain::agent_session::AgentSessionHistoryGatewayError::Store(self.kind.clone()))
    }
}

#[async_trait::async_trait]
impl crate::domain::agent_session::AgentSessionOwnershipQuery for FailingHistoryPorts {
    async fn is_owned(
        &self,
        _: ProviderKind,
        _: &str,
    ) -> Result<bool, crate::domain::agent_session::AgentSessionHistoryGatewayError> {
        if matches!(self.point, HistoryFailurePoint::Ownership) {
            return Err(
                crate::domain::agent_session::AgentSessionHistoryGatewayError::Store(
                    self.kind.clone(),
                ),
            );
        }
        Ok(false)
    }
}

#[tokio::test]
async fn test_履歴失敗分類_全gateway経路から購読を通じconnectへ保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode as C;
    // Given
    for point in [
        HistoryFailurePoint::Metadata,
        HistoryFailurePoint::Ownership,
        HistoryFailurePoint::Titles,
    ] {
        for (kind, expected) in [
            (
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "timeout".into(),
                }
                .into(),
                C::DeadlineExceeded,
            ),
            (
                crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                }
                .into(),
                C::DataLoss,
            ),
            (
                crate::domain::local_event::CommitBatchError::QueueBusy.into(),
                C::Unavailable,
            ),
            (
                crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into(),
                C::Aborted,
            ),
            (
                crate::domain::local_event::CommitBatchError::PayloadConflict.into(),
                C::FailedPrecondition,
            ),
        ] {
            let ports = Arc::new(FailingHistoryPorts { point, kind });
            let query = Arc::new(
                crate::adaptor::gateway::agent_session::LocalAgentSessionHistoryQueryService::new(
                    ports.clone(),
                    ports,
                ),
            );
            let usecase =
                Arc::new(crate::usecase::agent_session::AgentSessionHistoryReadUsecase::new(query));
            // When
            let error = usecase
                .list(crate::usecase::agent_session::AgentSessionHistoryRequest {
                    worktree_path: "/repo".into(),
                    visible_count: 1,
                })
                .await
                .unwrap_err();
            // Then
            assert_eq!(error.connect_code(), expected, "{point:?}");
            let message = format!("{error:?}");
            let connect = crate::adaptor::presenter::connect::classified_error(
                crate::usecase::state_subscription::StateReadError {
                    message: message.clone(),
                    source: error.into(),
                },
            );
            assert_eq!(connect.code, expected, "{point:?}");
            assert_eq!(connect.message, Some(message));
        }
    }
}

#[test]
fn test_provider操作失敗_各経路の分類がconnectまで一致する() {
    use crate::adaptor::presenter::connect::classified_error;
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode;
    // Given
    for error in [
        launch_error(
            AgentSessionLaunchUsecaseError::LaunchUnavailable,
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::LaunchUnavailable),
        launch_error(
            AgentSessionLaunchUsecaseError::TerminalUnavailable,
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::TerminalUnavailable),
        launch_error(
            AgentSessionLaunchUsecaseError::Conflict(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::Conflict(
            (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
        )),
    ] {
        // When / Then
        assert_eq!(error.connect_code(), ErrorCode::FailedPrecondition);
        assert_eq!(
            classified_error(error).code,
            connectrpc::ErrorCode::FailedPrecondition
        );
    }
}

#[test]
fn test_workflow失敗_session経由でも非storeの原因表示と分類を保持する() {
    use crate::adaptor::presenter::connect::classified_error;
    use crate::domain::workflow::WorkflowError as W;
    // Given
    for (source, expected) in [
        (
            W::Validation("input".into()),
            connectrpc::ErrorCode::InvalidArgument,
        ),
        (
            W::UnauthorizedApprovalTarget("target".into()),
            connectrpc::ErrorCode::PermissionDenied,
        ),
        (
            W::IncompatibleStoredEvent("version".into()),
            connectrpc::ErrorCode::FailedPrecondition,
        ),
        (
            W::External("external".into()),
            connectrpc::ErrorCode::Internal,
        ),
    ] {
        let expected_message = source.to_string();
        // When
        let error = lifecycle_error(AgentSessionLifecycleUsecaseError::Workflow(source));
        // Then
        assert_eq!(error.to_string(), expected_message);
        assert!(!error.to_string().contains("Storage failure"));
        assert_eq!(classified_error(error).code, expected);
    }
}

#[test]
fn test_provider起動準備_停止分類をrpcまで保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = launch_error(
            AgentSessionLaunchUsecaseError::Technical(stopped.into()),
            AgentSessionLaunchOperation::Start,
        );
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
    }
}
