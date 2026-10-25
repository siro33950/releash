use super::*;
use crate::usecase::agent_session::{
    AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecaseError,
};

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
fn test_agent_session_controller_成功応答は状態を持たないunitである() {
    let unit: crate::adaptor::presenter::client::Unit = ().try_into().unwrap();
    let response = crate::adaptor::presenter::client::CommandResult {
        command: Some(
            crate::adaptor::presenter::client::command_result::Command::OpenAgentSession(unit),
        ),
    };
    assert!(matches!(
        response.command,
        Some(crate::adaptor::presenter::client::command_result::Command::OpenAgentSession(_))
    ));
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
        AgentSessionLaunchUsecaseError::Terminal(
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: internal_error.to_string(),
                },
            ),
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
            provider_availability_error(ProviderAvailabilityUsecaseError::Config(crate::domain::agent_session::ProviderExecutableConfigRepositoryError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() }))),
            "PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE",
            "Releash could not access the Provider executable setting. Try again.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::Refresh(crate::domain::agent_session::ProviderExecutableProbeGatewayError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() }))),
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
                AgentSessionLaunchUsecaseError::Launch(crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() })),
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Releash could not complete the Provider operation for this AgentSession. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Terminal(crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() })),
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
                AgentSessionLaunchUsecaseError::Launch(
                    crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                ),
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::Launch(
                crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            )),
        ],
        "AGENT_SESSION_LAUNCH_UNAVAILABLE",
        "Releash could not complete the Provider operation for this AgentSession. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::Terminal(
                    crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                ),
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::Terminal(
                crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            )),
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
    for (error, code) in [
        launch_error(
            AgentSessionLaunchUsecaseError::Launch(
                crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            ),
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::Launch(
            crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ),
        )),
        launch_error(
            AgentSessionLaunchUsecaseError::Terminal(
                crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            ),
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::Terminal(
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ),
        )),
        launch_error(
            AgentSessionLaunchUsecaseError::Conflict(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            AgentSessionLaunchOperation::Start,
        ),
        lifecycle_error(AgentSessionLifecycleUsecaseError::Conflict(
            (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
        )),
    ]
    .into_iter()
    .zip([
        ErrorCode::Unavailable,
        ErrorCode::Unavailable,
        ErrorCode::Unavailable,
        ErrorCode::Unavailable,
        ErrorCode::FailedPrecondition,
        ErrorCode::FailedPrecondition,
    ]) {
        // When / Then
        assert_eq!(error.connect_code(), code);
        assert_eq!(classified_error(error).code, code);
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

use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::presenter::client as wire;
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionTreeLocation, ManagedPtyPresence,
};
use crate::domain::agent_session::services::AgentSessionService;
use crate::domain::agent_session::{
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalInputGateway,
};
use crate::domain::comment::{
    project_thread, Author, ReviewEvent, Target, Thread, ThreadRepository,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::usecase::agent_session::test_helpers::FailingSaveRepository;
use crate::usecase::agent_session::AgentSessionUsecase;
use std::sync::Mutex;

struct SendTerminal {
    presence: ManagedPtyPresence,
    writes: Mutex<Vec<(TerminalSurfaceOwner, String)>>,
}

impl ProviderAgentTerminalGateway for SendTerminal {
    fn spawn(
        &self,
        _: TerminalSurfaceOwner,
        _: &str,
        _: TerminalProcessLaunch,
        _: u16,
        _: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected spawn")
    }
    fn presence(
        &self,
        _: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError> {
        Ok(self.presence)
    }
    fn stop_preserving_checkpoint(
        &self,
        _: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected stop")
    }
    fn delete(&self, _: &TerminalSurfaceOwner) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected delete")
    }
    fn is_current_runtime_generation(
        &self,
        _: &TerminalSurfaceOwner,
        _: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        panic!("unexpected generation check")
    }
}

impl ProviderAgentTerminalInputGateway for SendTerminal {
    fn write_instruction(
        &self,
        owner: &TerminalSurfaceOwner,
        instruction: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.writes
            .lock()
            .unwrap()
            .push((owner.clone(), instruction.into()));
        Ok(())
    }
}

struct SendThreads(Vec<Thread>);
impl ThreadRepository for SendThreads {
    fn find_by_ids(
        &self,
        workspace: &WorkspaceIdentity,
        ids: &[String],
    ) -> Result<Vec<Thread>, crate::domain::comment::Error> {
        ids.iter()
            .map(|id| {
                self.0
                    .iter()
                    .find(|thread| thread.id == *id && thread.worktree_name == workspace.as_str())
                    .cloned()
                    .ok_or_else(|| crate::domain::comment::Error::NotFound(id.clone()))
            })
            .collect()
    }
}

fn send_thread(id: &str, worktree: &str, resolved: bool) -> Thread {
    let mut events = vec![ReviewEvent::ThreadCreated {
        event_id: format!("event-{id}"),
        thread_id: id.into(),
        comment_id: format!("comment-{id}"),
        actor: Author::human(),
        target: Target {
            file_path: Some("src/lib.rs".into()),
            line_number: Some(3),
            end_line: Some(5),
            side: None,
            end_side: None,
        },
        content: format!("本文-{id}"),
        at: 1.0,
    }];
    if resolved {
        events.push(ReviewEvent::ThreadResolved {
            event_id: format!("resolved-{id}"),
            thread_id: id.into(),
            actor: Author::human(),
            outcome: "done".into(),
            summary: "解決済み".into(),
            at: 2.0,
        });
    }
    project_thread(worktree, id, &events).unwrap()
}

fn send_dispatch(
    presence: ManagedPtyPresence,
    missing_session: bool,
    archived: bool,
) -> (ClientCommandDispatch, Arc<SendTerminal>) {
    let mut session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Claude,
        AgentSessionTreeLocation::session_tree_root("session").unwrap(),
    )
    .unwrap();
    if archived {
        session.archive().unwrap();
    }
    let repository = Arc::new(FailingSaveRepository::new(session));
    if missing_session {
        *repository.stored.lock().unwrap() = None;
    }
    let terminal = Arc::new(SendTerminal {
        presence,
        writes: Mutex::new(Vec::new()),
    });
    let sessions = AgentSessionUsecase::new(
        repository,
        Arc::new(SendThreads(vec![
            send_thread("one", "/repo", false),
            send_thread("two", "/repo", true),
            send_thread("outside", "/other", false),
        ])),
        AgentSessionService::new(terminal.clone(), terminal.clone()),
    );
    let mut deps = crate::acceptance_test_support::build_client_dependencies(PathBuf::new());
    deps.agent_session_usecase = Some(Arc::new(sessions));
    let mut dispatch = ClientCommandDispatch::new(deps.daemon.clone());
    crate::adaptor::controller::client::agent_session::register_shared(&mut dispatch, &deps);
    (dispatch, terminal)
}

use std::path::PathBuf;

fn send_request(ids: &[&str]) -> wire::command_request::Command {
    wire::command_request::Command::SendThreadsToSession(wire::SendThreadsToSessionRequest {
        agent_session_id: Some("session".into()),
        thread_ids: Some(
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        ),
    })
}

#[tokio::test]
async fn test_thread送信controller_複数threadを指定順で一回入力してunitを返す() {
    // Given
    let (dispatch, terminal) = send_dispatch(ManagedPtyPresence::Live, false, false);
    // When
    let result = dispatch
        .dispatch(send_request(&["two", "one"]))
        .await
        .unwrap();
    // Then
    assert!(matches!(
        result,
        wire::command_result::Command::SendThreadsToSession(_)
    ));
    let writes = terminal.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].0,
        TerminalSurfaceOwner::Session {
            workspace: WorkspaceIdentity::new("/repo"),
            session_id: "session".into()
        }
    );
    let prompt = &writes[0].1;
    assert!(prompt.contains("src/lib.rs:3-5"));
    assert!(prompt.contains("本文-two"));
    assert!(prompt.contains("本文-one"));
    assert!(prompt.find("Thread ID: two").unwrap() < prompt.find("Thread ID: one").unwrap());
    assert!(prompt.contains("releash review get --session-id \"$RELEASH_SESSION_ID\" two"));
}

#[tokio::test]
async fn test_thread送信controller_拒否した要求は何も入力しない() {
    use connectrpc::ErrorCode as C;
    // Given
    for (presence, missing, archived, ids, expected) in [
        (
            ManagedPtyPresence::ConfirmedAbsent,
            false,
            false,
            vec!["one"],
            C::FailedPrecondition,
        ),
        (
            ManagedPtyPresence::Unknown,
            false,
            false,
            vec!["one"],
            C::FailedPrecondition,
        ),
        (
            ManagedPtyPresence::Live,
            false,
            true,
            vec!["one"],
            C::FailedPrecondition,
        ),
        (
            ManagedPtyPresence::Live,
            true,
            false,
            vec!["one"],
            C::NotFound,
        ),
        (
            ManagedPtyPresence::Live,
            false,
            false,
            vec!["one", "outside"],
            C::NotFound,
        ),
        (
            ManagedPtyPresence::Live,
            false,
            false,
            vec!["one", "missing"],
            C::NotFound,
        ),
    ] {
        let (dispatch, terminal) = send_dispatch(presence, missing, archived);
        // When
        let error = dispatch.dispatch(send_request(&ids)).await.unwrap_err();
        // Then
        assert_eq!(
            crate::adaptor::presenter::connect::command_error(error).code,
            expected
        );
        assert!(terminal.writes.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_thread送信controller_必須fieldが無い要求を拒否する() {
    // Given
    let (dispatch, terminal) = send_dispatch(ManagedPtyPresence::Live, false, false);
    for request in [
        wire::SendThreadsToSessionRequest {
            agent_session_id: None,
            thread_ids: Some(vec!["one".to_string()].try_into().unwrap()),
        },
        wire::SendThreadsToSessionRequest {
            agent_session_id: Some("session".into()),
            thread_ids: None,
        },
        wire::SendThreadsToSessionRequest {
            agent_session_id: Some("session".into()),
            thread_ids: Some(Vec::<String>::new().try_into().unwrap()),
        },
    ] {
        // When
        let error = dispatch
            .dispatch(wire::command_request::Command::SendThreadsToSession(
                request,
            ))
            .await
            .unwrap_err();
        // Then
        assert_eq!(
            crate::adaptor::presenter::connect::command_error(error).code,
            connectrpc::ErrorCode::InvalidArgument
        );
        assert!(terminal.writes.lock().unwrap().is_empty());
    }
}
