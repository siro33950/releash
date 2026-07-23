use std::sync::Arc;

use crate::adaptor::controller::agent_session_operation_wiring::{
    CanonicalSendCommandV1, CanonicalSendTargetV1, LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
};
use crate::adaptor::protocol::agent_session_v1::{
    checked_pending_recovery_page, decode_nonnegative_i64_decimal, decode_nonnegative_u64_decimal,
    decode_positive_i64_decimal, GetSessionResponseDtoV1, InitSessionsResponseDtoV1,
    OperationApplicationErrorDtoV1, PendingCallerAttemptPageDtoV1, PendingPartitionDtoV1,
    PendingRecoveryPageDtoV1, PendingRecoveryQueryErrorDtoV1,
    PendingRecoverySnapshotQueryErrorDtoV1, RecoveryActionCommandErrorDtoV1,
    RecoveryActionLookupErrorDtoV1, RecoveryActionOutcomeDtoV1, RecoveryActionRequestDtoV1,
    RecoveryActionStatusDtoV1, SafeOperationFailureDtoV1, SendCommandErrorDtoV1,
    SendCommandOutcomeDtoV1, SendLookupErrorDtoV1, SendOperationViewDtoV1,
    SessionLifecycleActionDtoV1, SessionLifecycleCommandErrorDtoV1,
    SessionLifecycleCommandResultDtoV1, SessionLifecycleLookupErrorDtoV1,
    SessionLifecycleRejectionDtoV1, SessionLifecycleRequestDtoV1, SessionPageDtoV1,
    StopCommandErrorDtoV1, StopCommandOutcomeDtoV1, StopLookupErrorDtoV1,
    StopOperationReceiptDtoV1, StopOperationRequestDtoV1, StopOperationStateDtoV1,
};
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::other::error::AppError;
use crate::usecase::agent_session::session::{
    AgentTaskListReport, AgentThreadSearchMatch, PageCursor, SessionSearchResult, SessionStore,
    DEFAULT_SESSION_PAGE_LIMIT,
};

pub(super) const TAURI_OPERATION_PRINCIPAL: &str = LOCAL_INSTALLATION_OPERATION_PRINCIPAL;

fn presentation_correlation(context: &str, detail: &str) -> String {
    match crate::adaptor::presenter::application_lifecycle::presentation_error(context, detail) {
        OperationApplicationErrorDtoV1::Internal { correlation_id } => correlation_id,
        _ => unreachable!("presentation errors are always Internal"),
    }
}

fn common_error_correlation(context: &str, error: &OperationApplicationErrorDtoV1) -> String {
    match error {
        OperationApplicationErrorDtoV1::Internal { correlation_id } => correlation_id.clone(),
        OperationApplicationErrorDtoV1::StorageUnavailable { failure } => {
            failure.correlation_id.clone()
        }
        other => presentation_correlation(context, &format!("{other:?}")),
    }
}

fn send_command_common(error: OperationApplicationErrorDtoV1) -> SendCommandErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::InvalidRequest => SendCommandErrorDtoV1::InvalidRequest,
        OperationApplicationErrorDtoV1::PayloadConflict => SendCommandErrorDtoV1::PayloadConflict,
        OperationApplicationErrorDtoV1::NotFound => SendCommandErrorDtoV1::NotFound,
        OperationApplicationErrorDtoV1::CapacityExceeded => SendCommandErrorDtoV1::CapacityExceeded,
        OperationApplicationErrorDtoV1::FeedbackCapacityExceeded => {
            SendCommandErrorDtoV1::FeedbackCapacityExceeded
        }
        OperationApplicationErrorDtoV1::MigrationInProgress => {
            SendCommandErrorDtoV1::MigrationInProgress
        }
        OperationApplicationErrorDtoV1::ShutdownInProgress => {
            SendCommandErrorDtoV1::ShutdownInProgress
        }
        OperationApplicationErrorDtoV1::ResponseTooLarge => SendCommandErrorDtoV1::ResponseTooLarge,
        error => SendCommandErrorDtoV1::Internal {
            correlation_id: common_error_correlation("send_command", &error),
        },
    }
}

fn stop_command_common(error: OperationApplicationErrorDtoV1) -> StopCommandErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::InvalidRequest => StopCommandErrorDtoV1::InvalidRequest,
        OperationApplicationErrorDtoV1::PayloadConflict => StopCommandErrorDtoV1::PayloadConflict,
        OperationApplicationErrorDtoV1::FeedbackCapacityExceeded => {
            StopCommandErrorDtoV1::FeedbackCapacityExceeded
        }
        OperationApplicationErrorDtoV1::MigrationInProgress => {
            StopCommandErrorDtoV1::MigrationInProgress
        }
        OperationApplicationErrorDtoV1::ShutdownInProgress => {
            StopCommandErrorDtoV1::ShutdownInProgress
        }
        error => StopCommandErrorDtoV1::Internal {
            correlation_id: common_error_correlation("stop_command", &error),
        },
    }
}

fn lifecycle_command_common(
    error: OperationApplicationErrorDtoV1,
) -> SessionLifecycleCommandErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::InvalidRequest => {
            SessionLifecycleCommandErrorDtoV1::InvalidRequest
        }
        OperationApplicationErrorDtoV1::PayloadConflict => {
            SessionLifecycleCommandErrorDtoV1::PayloadConflict
        }
        OperationApplicationErrorDtoV1::FeedbackCapacityExceeded => {
            SessionLifecycleCommandErrorDtoV1::FeedbackCapacityExceeded
        }
        OperationApplicationErrorDtoV1::MigrationInProgress => {
            SessionLifecycleCommandErrorDtoV1::MigrationInProgress
        }
        OperationApplicationErrorDtoV1::ShutdownInProgress => {
            SessionLifecycleCommandErrorDtoV1::ShutdownInProgress
        }
        error => SessionLifecycleCommandErrorDtoV1::Internal {
            correlation_id: common_error_correlation("session_lifecycle_command", &error),
        },
    }
}

fn recovery_command_common(
    error: OperationApplicationErrorDtoV1,
) -> RecoveryActionCommandErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::InvalidRequest => {
            RecoveryActionCommandErrorDtoV1::InvalidRequest
        }
        OperationApplicationErrorDtoV1::MigrationInProgress => {
            RecoveryActionCommandErrorDtoV1::MigrationInProgress
        }
        OperationApplicationErrorDtoV1::ShutdownInProgress => {
            RecoveryActionCommandErrorDtoV1::ShutdownInProgress
        }
        OperationApplicationErrorDtoV1::StorageUnavailable { failure } => {
            RecoveryActionCommandErrorDtoV1::StorageUnavailable { failure }
        }
        error => RecoveryActionCommandErrorDtoV1::Internal {
            correlation_id: common_error_correlation("recovery_action_command", &error),
        },
    }
}

pub(super) async fn ensure_mutation_admission(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
) -> Result<(), OperationApplicationErrorDtoV1> {
    use crate::domain::local_event::LocalEventTransactionRepository as _;
    if !store.normal_admission_ready() {
        return Err(OperationApplicationErrorDtoV1::MigrationInProgress);
    }
    let current = store
        .query(crate::domain::local_event::LocalEventQuery::CurrentShutdown)
        .await
        .map_err(crate::adaptor::controller::command::application_lifecycle::query_error)?;
    match current {
        crate::domain::local_event::LocalEventQueryResult::CurrentShutdown(Some(plan))
            if matches!(
                plan.phase,
                crate::domain::local_event::ApplicationShutdownPhase::Failed
                    | crate::domain::local_event::ApplicationShutdownPhase::Cancelled
                    | crate::domain::local_event::ApplicationShutdownPhase::Completed
            ) =>
        {
            Ok(())
        }
        crate::domain::local_event::LocalEventQueryResult::CurrentShutdown(Some(_)) => {
            Err(OperationApplicationErrorDtoV1::ShutdownInProgress)
        }
        crate::domain::local_event::LocalEventQueryResult::CurrentShutdown(None) => Ok(()),
        _ => Err(OperationApplicationErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "agent_session_mutation_admission",
                "current shutdown query returned the wrong shape",
            ),
        }),
    }
}

pub(super) async fn ensure_mutation_admission_message(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
) -> Result<(), String> {
    ensure_mutation_admission(store)
        .await
        .map_err(|error| match error {
            OperationApplicationErrorDtoV1::MigrationInProgress => {
                "MigrationInProgress".to_string()
            }
            OperationApplicationErrorDtoV1::ShutdownInProgress => "ShutdownInProgress".to_string(),
            OperationApplicationErrorDtoV1::StorageUnavailable { .. } => {
                "StorageUnavailable".to_string()
            }
            OperationApplicationErrorDtoV1::Internal { correlation_id } => {
                format!("Internal:{correlation_id}")
            }
            _ => "MutationAdmissionRejected".to_string(),
        })
}

pub(super) fn normalize_mutation_error(message: String) -> String {
    if message.contains("Application shutdown is in progress")
        || message.contains("PreviousShutdownReconciliationRequired")
    {
        "ShutdownInProgress".to_string()
    } else if message.contains("migration is in progress") {
        "MigrationInProgress".to_string()
    } else {
        message
    }
}

pub(super) fn caller_journal_failure(
    label: &str,
) -> crate::domain::local_event::SafeOperationFailure {
    crate::domain::local_event::SafeOperationFailure::new(
        crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
        true,
        label,
        presentation_correlation("caller_journal", label),
    )
}

pub(super) fn caller_journal_application_error(
    error: crate::usecase::agent_session::operation::CallerJournalError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::CallerJournalError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::RejectedBeforeCommit | E::OutcomeUnknown => OperationApplicationErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "caller_journal",
                "caller journal result requires reconciliation",
            ),
        },
    }
}

fn send_operation_error(
    error: crate::usecase::agent_session::operation::SendAgentMessageError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::SendAgentMessageError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::CapacityExceeded => OperationApplicationErrorDtoV1::CapacityExceeded,
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
    }
}

fn feedback_operation_error(
    error: crate::usecase::agent_session::feedback::FeedbackError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::feedback::FeedbackError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::CapacityExceeded => OperationApplicationErrorDtoV1::FeedbackCapacityExceeded,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => OperationApplicationErrorDtoV1::ResponseTooLarge,
        E::OutcomeUnknown { feedback_id } => OperationApplicationErrorDtoV1::OutcomeUnknown {
            operation_id: feedback_id,
        },
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        E::NotFound | E::RevisionConflict { .. } | E::CursorMismatch | E::CursorExpired => {
            OperationApplicationErrorDtoV1::Internal {
                correlation_id: presentation_correlation(
                    "session_feedback_attempt",
                    "feedback attempt state changed unexpectedly",
                ),
            }
        }
    }
}

fn session_feedback_load_error(
    error: crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadError as E;
    match error {
        E::Feedback(error) => feedback_operation_error(error),
        E::LoadFailed { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
    }
}

/// Tauri invoke 境界で permission_mode を検証し、検証済み抽象モードを返す。
/// 欠落（None）は空文字相当として扱い、対象外値とともに [`crate::domain::agent_session::InvalidPermissionMode`]
/// で拒否する。command 経路と単体テスト経路の両方で同じ拒否ロジックを共有する（Spec issues-947）。
fn validate_invoke_permission_mode(
    permission_mode: Option<String>,
) -> Result<crate::domain::agent_session::PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_default();
    crate::domain::agent_session::PermissionMode::parse(&permission_value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_session(
    usecase: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase>,
    >,
    session_id: String,
    attempt_id: String,
) -> Result<Option<GetSessionResponseDtoV1>, OperationApplicationErrorDtoV1> {
    dispatch_feedback_supervised_session_load(usecase.inner().as_ref(), &session_id, &attempt_id)
        .await
}

async fn dispatch_feedback_supervised_session_load(
    usecase: &crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase,
    session_id: &str,
    attempt_id: &str,
) -> Result<Option<GetSessionResponseDtoV1>, OperationApplicationErrorDtoV1> {
    usecase
        .get_session(session_id, attempt_id)
        .await
        .map(|response| response.map(Into::into))
        .map_err(session_feedback_load_error)
}

#[tauri::command]
pub async fn get_session_page(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<Option<SessionPageDtoV1>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let cursor = cursor
        .as_deref()
        .map(|value| {
            decode_nonnegative_u64_decimal(value)
                .map(PageCursor)
                .ok_or_else(|| "Invalid session page cursor".to_string())
        })
        .transpose()?;
    state
        .get_session_page(
            &data_dir,
            &session_id,
            cursor,
            limit.unwrap_or(DEFAULT_SESSION_PAGE_LIMIT),
        )
        .map(|page| page.map(Into::into))
}

#[tauri::command]
pub fn plan_agent_chat_eviction(
    request: crate::usecase::agent_session::session::AgentChatEvictionPlanRequest,
) -> Result<crate::usecase::agent_session::session::AgentChatEvictionPlan, String> {
    Ok(crate::usecase::agent_session::session::plan_agent_chat_eviction(request))
}

#[tauri::command]
pub async fn get_session_attachment(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    attachment_id: String,
) -> Result<Option<crate::usecase::agent_session::session::ImageAttachment>, String> {
    let data_dir = resolve_data_dir(&app)?;
    Ok(state
        .get_session_attachment(&data_dir, &session_id, &attachment_id)?
        .map(
            |attachment| crate::usecase::agent_session::session::ImageAttachment {
                data: attachment.data,
                media_type: attachment.media_type,
            },
        ))
}

#[tauri::command]
pub async fn get_session_tool_output(
    state: tauri::State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    tool_output_id: String,
) -> Result<Option<crate::usecase::agent_session::session::SessionToolOutput>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.get_session_tool_output(&data_dir, &session_id, &tool_output_id)
}

#[tauri::command]
pub async fn search_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    worktree_path: String,
    query: String,
    include_workflow: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<SessionSearchResult>, String> {
    let data_dir = resolve_data_dir(&app)?;
    crate::usecase::agent_session::session::search_agent_sessions(
        session_store.inner(),
        &data_dir,
        &worktree_path,
        &query,
        include_workflow.unwrap_or(false),
        limit.unwrap_or(20),
    )
}

#[tauri::command]
pub async fn search_agent_session_messages(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    session_id: String,
    query: String,
) -> Result<Vec<AgentThreadSearchMatch>, String> {
    let data_dir = resolve_data_dir(&app)?;
    crate::usecase::agent_session::session::search_agent_session_messages(
        session_store.inner(),
        &data_dir,
        &session_id,
        &query,
    )
}

fn stop_error(
    error: crate::usecase::agent_session::operation::StopOperationError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::StopOperationError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::CapacityExceeded => OperationApplicationErrorDtoV1::CapacityExceeded,
        E::StaleTarget => OperationApplicationErrorDtoV1::StaleTarget,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
    }
}

fn stop_lookup_error(
    error: crate::usecase::agent_session::operation::StopOperationError,
) -> StopLookupErrorDtoV1 {
    use crate::usecase::agent_session::operation::StopOperationError as E;
    match error {
        E::InvalidRequest => StopLookupErrorDtoV1::InvalidRequest,
        E::NotFound => StopLookupErrorDtoV1::NotFound,
        E::QueryBusy => StopLookupErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => StopLookupErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => StopLookupErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => StopLookupErrorDtoV1::Internal { correlation_id },
        other => StopLookupErrorDtoV1::Internal {
            correlation_id: presentation_correlation("stop_lookup", &format!("{other:?}")),
        },
    }
}

#[tauri::command]
pub async fn stop_agent_session(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::StopOperationUsecase>>,
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    request: StopOperationRequestDtoV1,
) -> Result<StopCommandOutcomeDtoV1, StopCommandErrorDtoV1> {
    let expected_session_revision =
        decode_nonnegative_u64_decimal(&request.expected_session_revision)
            .ok_or(StopCommandErrorDtoV1::InvalidRequest)?;
    let turn_id = decode_positive_i64_decimal(&request.turn_id)
        .ok_or(StopCommandErrorDtoV1::InvalidRequest)?
        .to_string();
    let exact_command =
        serde_json::to_vec(&request).map_err(|_| StopCommandErrorDtoV1::InvalidRequest)?;
    // Resolve the immutable caller identity before checking admission. A
    // previously accepted Stop remains replayable after shutdown/migration
    // closes admission, while an unreadable lookup is outcome-unknown rather
    // than proof that a new mutation is safe.
    let replaying_existing = match usecase
        .get_operation(TAURI_OPERATION_PRINCIPAL, &request.request_id)
        .await
    {
        Ok(_) => true,
        Err(crate::usecase::agent_session::operation::StopOperationError::NotFound) => {
            ensure_mutation_admission(store.inner().as_ref())
                .await
                .map_err(stop_command_common)?;
            false
        }
        Err(crate::usecase::agent_session::operation::StopOperationError::InvalidRequest) => {
            return Err(StopCommandErrorDtoV1::InvalidRequest);
        }
        Err(_) => {
            return Ok(StopCommandOutcomeDtoV1::OutcomeUnknown {
                request_id: request.request_id,
            });
        }
    };
    if !replaying_existing {
        match journal
            .record_attempt_scoped(
                TAURI_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::Stop,
                &request.request_id,
                &exact_command,
                Some(&request.session_id),
            )
            .await
        {
            Ok(_) => {}
            Err(crate::usecase::agent_session::operation::CallerJournalError::OutcomeUnknown) => {
                return Ok(StopCommandOutcomeDtoV1::OutcomeUnknown {
                    request_id: request.request_id,
                });
            }
            Err(
                crate::usecase::agent_session::operation::CallerJournalError::RejectedBeforeCommit,
            ) => {
                return Ok(StopCommandOutcomeDtoV1::RejectedBeforeCommit {
                    failure: SafeOperationFailureDtoV1::from(caller_journal_failure(
                        "The local caller attempt could not be saved.",
                    )),
                });
            }
            Err(error) => return Err(stop_command_common(caller_journal_application_error(error))),
        }
    }
    let request_id = request.request_id.clone();
    let outcome = usecase
        .request(
            crate::usecase::agent_session::operation::StopOperationRequest {
                principal: TAURI_OPERATION_PRINCIPAL.to_string(),
                request_id: request.request_id,
                session_id: request.session_id,
                turn_id,
                expected_session_revision,
            },
        )
        .await
        .map_err(|error| stop_command_common(stop_error(error)))?;
    if !matches!(
        &outcome,
        crate::usecase::agent_session::operation::StopCommandOutcome::OutcomeUnknown { .. }
    ) {
        let accepted = matches!(
            &outcome,
            crate::usecase::agent_session::operation::StopCommandOutcome::Accepted { .. }
        );
        if let Err(error) = journal
            .resolve_attempt_if_present(
                TAURI_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::Stop,
                &request_id,
                &exact_command,
                accepted,
            )
            .await
        {
            log::warn!("caller Stop journal clear requires reconciliation: {error:?}");
        }
    }
    Ok(outcome.into())
}

#[tauri::command]
pub async fn get_stop_operation(
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::StopOperationUsecase>>,
    operation_id: String,
) -> Result<(StopOperationReceiptDtoV1, StopOperationStateDtoV1), StopLookupErrorDtoV1> {
    usecase
        .get_operation(TAURI_OPERATION_PRINCIPAL, &operation_id)
        .await
        .map(|(receipt, state)| (receipt.into(), state.into()))
        .map_err(stop_lookup_error)
}

fn lifecycle_error(
    error: crate::usecase::agent_session::operation::SessionLifecycleOperationError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::SessionLifecycleOperationError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
    }
}

fn lifecycle_lookup_error(
    error: crate::usecase::agent_session::operation::SessionLifecycleOperationError,
) -> SessionLifecycleLookupErrorDtoV1 {
    use crate::usecase::agent_session::operation::SessionLifecycleOperationError as E;
    match error {
        E::InvalidRequest => SessionLifecycleLookupErrorDtoV1::InvalidRequest,
        E::NotFound => SessionLifecycleLookupErrorDtoV1::NotFound,
        E::QueryBusy => SessionLifecycleLookupErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => SessionLifecycleLookupErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => SessionLifecycleLookupErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            SessionLifecycleLookupErrorDtoV1::Internal { correlation_id }
        }
        other => SessionLifecycleLookupErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "session_lifecycle_lookup",
                &format!("{other:?}"),
            ),
        },
    }
}

fn pending_recovery_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> PendingRecoveryQueryErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest | E::SnapshotMismatch => PendingRecoveryQueryErrorDtoV1::InvalidRequest,
        E::CursorMismatch => PendingRecoveryQueryErrorDtoV1::CursorMismatch,
        E::CursorExpired => PendingRecoveryQueryErrorDtoV1::CursorExpired,
        E::QueryBusy => PendingRecoveryQueryErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => PendingRecoveryQueryErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => PendingRecoveryQueryErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => PendingRecoveryQueryErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            PendingRecoveryQueryErrorDtoV1::Internal { correlation_id }
        }
        other => PendingRecoveryQueryErrorDtoV1::Internal {
            correlation_id: presentation_correlation("pending_recovery", &format!("{other:?}")),
        },
    }
}

fn pending_recovery_snapshot_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> PendingRecoverySnapshotQueryErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => PendingRecoverySnapshotQueryErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => PendingRecoverySnapshotQueryErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "pending_recovery_snapshot",
                "read-only recovery query returned a mutation-admission error",
            ),
        },
        E::NotFound => PendingRecoverySnapshotQueryErrorDtoV1::NotFound,
        E::SnapshotMismatch => PendingRecoverySnapshotQueryErrorDtoV1::SnapshotMismatch,
        E::CursorMismatch => PendingRecoverySnapshotQueryErrorDtoV1::CursorMismatch,
        E::CursorExpired => PendingRecoverySnapshotQueryErrorDtoV1::CursorExpired,
        E::DetailsCompacted => PendingRecoverySnapshotQueryErrorDtoV1::DetailsCompacted,
        E::QueryBusy => PendingRecoverySnapshotQueryErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => PendingRecoverySnapshotQueryErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => PendingRecoverySnapshotQueryErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => {
            PendingRecoverySnapshotQueryErrorDtoV1::StorageUnavailable {
                failure: failure.into(),
            }
        }
        E::Internal { correlation_id } => {
            PendingRecoverySnapshotQueryErrorDtoV1::Internal { correlation_id }
        }
    }
}

fn recovery_command_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> RecoveryActionCommandErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => RecoveryActionCommandErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => RecoveryActionCommandErrorDtoV1::ShutdownInProgress,
        E::NotFound => RecoveryActionCommandErrorDtoV1::NotFound,
        E::StorageUnavailable { failure } => RecoveryActionCommandErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            RecoveryActionCommandErrorDtoV1::Internal { correlation_id }
        }
        other => RecoveryActionCommandErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "recovery_action_command",
                &format!("{other:?}"),
            ),
        },
    }
}

fn recovery_lookup_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> RecoveryActionLookupErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => RecoveryActionLookupErrorDtoV1::InvalidRequest,
        E::NotFound => RecoveryActionLookupErrorDtoV1::NotFound,
        E::QueryBusy => RecoveryActionLookupErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => RecoveryActionLookupErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => RecoveryActionLookupErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            RecoveryActionLookupErrorDtoV1::Internal { correlation_id }
        }
        other => RecoveryActionLookupErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "recovery_action_lookup",
                &format!("{other:?}"),
            ),
        },
    }
}

#[tauri::command]
pub async fn list_pending_agent_recovery(
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>,
    limit: Option<usize>,
    partition: Option<PendingPartitionDtoV1>,
    owner: Option<String>,
    shutdown_plan_id: Option<String>,
    shutdown_epoch: Option<String>,
    cursor: Option<String>,
) -> Result<PendingRecoveryPageDtoV1, PendingRecoveryQueryErrorDtoV1> {
    let partition = partition.map(|value| match value {
        PendingPartitionDtoV1::Owner => crate::domain::local_event::PendingPartition::Owner,
        PendingPartitionDtoV1::ClosedSession => {
            crate::domain::local_event::PendingPartition::ClosedSession
        }
        PendingPartitionDtoV1::ArchivedSession => {
            crate::domain::local_event::PendingPartition::ArchivedSession
        }
        PendingPartitionDtoV1::UnownedRuntime => {
            crate::domain::local_event::PendingPartition::UnownedRuntime
        }
    });
    let shutdown_plan = match (shutdown_plan_id, shutdown_epoch) {
        (Some(plan_id), Some(epoch)) if !plan_id.is_empty() => {
            Some(crate::domain::local_event::ShutdownPlanKey {
                plan_id,
                epoch: decode_nonnegative_i64_decimal(&epoch)
                    .ok_or(PendingRecoveryQueryErrorDtoV1::InvalidRequest)?,
            })
        }
        (None, None) => None,
        _ => return Err(PendingRecoveryQueryErrorDtoV1::InvalidRequest),
    };
    usecase
        .pending(
            crate::usecase::agent_session::operation::PendingRecoveryQuery {
                limit: limit.unwrap_or(32),
                partition,
                owner,
                shutdown_plan,
                cursor,
            },
        )
        .await
        .and_then(checked_pending_recovery_page)
        .map_err(pending_recovery_error)
}

#[tauri::command]
pub async fn get_pending_recovery_snapshot(
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>,
    plan_id: String,
    epoch: String,
    snapshot_id: String,
    partition: PendingPartitionDtoV1,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<PendingRecoveryPageDtoV1, PendingRecoverySnapshotQueryErrorDtoV1> {
    let epoch = decode_nonnegative_i64_decimal(&epoch)
        .ok_or(PendingRecoverySnapshotQueryErrorDtoV1::InvalidRequest)?;
    let partition = match partition {
        PendingPartitionDtoV1::ClosedSession => {
            crate::domain::local_event::PendingPartition::ClosedSession
        }
        PendingPartitionDtoV1::ArchivedSession => {
            crate::domain::local_event::PendingPartition::ArchivedSession
        }
        PendingPartitionDtoV1::UnownedRuntime => {
            crate::domain::local_event::PendingPartition::UnownedRuntime
        }
        PendingPartitionDtoV1::Owner => {
            return Err(PendingRecoverySnapshotQueryErrorDtoV1::InvalidRequest)
        }
    };
    usecase
        .pending_snapshot(
            crate::usecase::agent_session::operation::PendingRecoverySnapshotQuery {
                plan: crate::domain::local_event::ShutdownPlanKey { plan_id, epoch },
                snapshot_id,
                partition,
                limit: limit.unwrap_or(32),
                cursor,
            },
        )
        .await
        .and_then(checked_pending_recovery_page)
        .map_err(pending_recovery_snapshot_error)
}

#[tauri::command]
pub async fn list_pending_agent_attempts(
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    scope_id: String,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<PendingCallerAttemptPageDtoV1, OperationApplicationErrorDtoV1> {
    journal
        .pending_page_for_scope(
            TAURI_OPERATION_PRINCIPAL,
            &scope_id,
            limit.unwrap_or(32),
            cursor.as_deref(),
        )
        .await
        .map(Into::into)
        .map_err(caller_journal_application_error)
}

#[tauri::command]
pub async fn acknowledge_agent_attempt(
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    kind: String,
    caller_request_id: String,
) -> Result<(), OperationApplicationErrorDtoV1> {
    let kind = crate::domain::local_event::OperationKind::parse(&kind)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    journal
        .acknowledge_attempt(TAURI_OPERATION_PRINCIPAL, kind, &caller_request_id)
        .await
        .map_err(caller_journal_application_error)
}

#[tauri::command]
pub async fn resolve_pending_recovery_action(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>,
    request: RecoveryActionRequestDtoV1,
) -> Result<RecoveryActionOutcomeDtoV1, RecoveryActionCommandErrorDtoV1> {
    let origin_revision = decode_nonnegative_u64_decimal(&request.origin_revision)
        .ok_or(RecoveryActionCommandErrorDtoV1::InvalidRequest)?;
    match usecase.get_action(&request.action_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            ensure_mutation_admission(store.inner().as_ref())
                .await
                .map_err(recovery_command_common)?;
        }
        Err(crate::usecase::agent_session::operation::RecoveryActionError::InvalidRequest) => {
            return Err(RecoveryActionCommandErrorDtoV1::InvalidRequest);
        }
        Err(_) => {
            return Ok(RecoveryActionOutcomeDtoV1::ActionOutcomeUnknown {
                action_id: request.action_id,
            });
        }
    }
    let outcome = usecase
        .request(
            crate::usecase::agent_session::operation::RecoveryActionRequest {
                action_id: request.action_id,
                obligation_id: request.obligation_id,
                origin_revision,
                action: request.action.into(),
            },
        )
        .await
        .map_err(recovery_command_error)?;
    if let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
        ref action_id,
        ..
    } = outcome
    {
        let status = usecase
            .get_action_status(action_id)
            .await
            .map_err(recovery_command_error)?;
        return Ok(RecoveryActionOutcomeDtoV1::from_durable_status(status));
    }
    Ok(outcome.into())
}

#[tauri::command]
pub async fn get_recovery_action(
    usecase: tauri::State<'_, Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>,
    action_id: String,
) -> Result<RecoveryActionStatusDtoV1, RecoveryActionLookupErrorDtoV1> {
    usecase
        .get_action_status(&action_id)
        .await
        .map(Into::into)
        .map_err(recovery_lookup_error)
}

#[tauri::command]
pub async fn request_session_lifecycle(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    usecase: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase>,
    >,
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    request: SessionLifecycleRequestDtoV1,
) -> Result<SessionLifecycleCommandResultDtoV1, SessionLifecycleCommandErrorDtoV1> {
    let expected_session_revision =
        decode_nonnegative_i64_decimal(&request.expected_session_revision)
            .ok_or(SessionLifecycleCommandErrorDtoV1::InvalidRequest)?;
    let exact_command = serde_json::to_vec(&request)
        .map_err(|_| SessionLifecycleCommandErrorDtoV1::InvalidRequest)?;
    // A closed admission gate may reject only genuinely new lifecycle work.
    // Replays are resolved from the durable caller binding first.
    let replaying_existing = match usecase
        .get_operation(TAURI_OPERATION_PRINCIPAL, &request.request_id)
        .await
    {
        Ok(_) => true,
        Err(
            crate::usecase::agent_session::operation::SessionLifecycleOperationError::NotFound,
        ) => {
            ensure_mutation_admission(store.inner().as_ref())
                .await
                .map_err(lifecycle_command_common)?;
            false
        }
        Err(
            crate::usecase::agent_session::operation::SessionLifecycleOperationError::InvalidRequest,
        ) => return Err(SessionLifecycleCommandErrorDtoV1::InvalidRequest),
        Err(_) => {
            return Ok(SessionLifecycleCommandResultDtoV1::OutcomeUnknown {
                request_id: request.request_id,
            });
        }
    };
    if !replaying_existing {
        match journal
            .record_attempt_scoped(
                TAURI_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::SessionLifecycle,
                &request.request_id,
                &exact_command,
                Some(&request.session_id),
            )
            .await
        {
            Ok(_) => {}
            Err(crate::usecase::agent_session::operation::CallerJournalError::OutcomeUnknown) => {
                return Ok(SessionLifecycleCommandResultDtoV1::OutcomeUnknown {
                    request_id: request.request_id,
                });
            }
            Err(
                crate::usecase::agent_session::operation::CallerJournalError::RejectedBeforeCommit,
            ) => {
                return Ok(SessionLifecycleCommandResultDtoV1::Rejected {
                    rejection: SessionLifecycleRejectionDtoV1::Failed {
                        failure: SafeOperationFailureDtoV1::from(caller_journal_failure(
                            "The local caller attempt could not be saved.",
                        )),
                    },
                });
            }
            Err(error) => {
                return Err(lifecycle_command_common(caller_journal_application_error(
                    error,
                )))
            }
        }
    }
    let request_id = request.request_id.clone();
    let action = match request.action {
        SessionLifecycleActionDtoV1::Close => {
            crate::usecase::agent_session::operation::SessionLifecycleAction::Close
        }
        SessionLifecycleActionDtoV1::ArchiveOpen => {
            crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveOpen
        }
        SessionLifecycleActionDtoV1::ArchiveClosed => {
            crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveClosed
        }
        SessionLifecycleActionDtoV1::SwitchBackend { backend_id } => {
            crate::usecase::agent_session::operation::SessionLifecycleAction::SwitchBackend {
                backend_id,
            }
        }
    };
    let outcome = usecase
        .request(
            crate::usecase::agent_session::operation::SessionLifecycleRequest {
                principal: TAURI_OPERATION_PRINCIPAL.to_string(),
                request_id: request.request_id,
                session_id: request.session_id,
                expected_session_revision,
                action,
            },
        )
        .await
        .map_err(|error| lifecycle_command_common(lifecycle_error(error)))?;
    if !matches!(
        &outcome,
        crate::usecase::agent_session::operation::SessionLifecycleCommandResult::OutcomeUnknown { .. }
    ) {
        let accepted = matches!(
            &outcome,
            crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Accepted { .. }
        );
        if let Err(error) = journal
            .resolve_attempt_if_present(
                TAURI_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::SessionLifecycle,
                &request_id,
                &exact_command,
                accepted,
            )
            .await
        {
            log::warn!("caller session lifecycle journal clear requires reconciliation: {error:?}");
        }
    }
    Ok(outcome.into())
}

#[tauri::command]
pub async fn get_session_lifecycle_operation(
    usecase: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase>,
    >,
    operation_id: String,
) -> Result<
    (
        crate::adaptor::protocol::agent_session_v1::SessionLifecycleReceiptDtoV1,
        crate::adaptor::protocol::agent_session_v1::SessionLifecycleStateDtoV1,
    ),
    SessionLifecycleLookupErrorDtoV1,
> {
    usecase
        .get_operation(TAURI_OPERATION_PRINCIPAL, &operation_id)
        .await
        .map(|(receipt, state)| (receipt.into(), state.into()))
        .map_err(lifecycle_lookup_error)
}

#[tauri::command]
pub async fn resume_agent_queue(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
) -> Result<(), AppError> {
    ensure_mutation_admission_message(store.inner().as_ref())
        .await
        .map_err(AppError::new)?;
    runtime
        .resume_queue(&chat_session_id)
        .await
        .map_err(|error| AppError::new(normalize_mutation_error(error.to_string())))
}

#[tauri::command]
pub async fn cancel_agent_queued_turn(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
    queued_turn_id: Option<String>,
) -> Result<crate::usecase::agent_session::runtime::usecase::CancelQueuedTurnResponse, String> {
    runtime
        .cancel_queued_turn(&chat_session_id, queued_turn_id.as_deref())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn build_agent_task_list_report(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    chat_session_id: String,
) -> Result<AgentTaskListReport, String> {
    runtime
        .build_agent_task_list_report(&chat_session_id)
        .await
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn init_agent_sessions(
    runtime: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    >,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<InitSessionsResponseDtoV1, String> {
    runtime
        .init_sessions(&worktree_path, open_tabs.inner())
        .await
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnreadableTauriSessionLoader;

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::session_feedback_load::SessionLoadPort
        for UnreadableTauriSessionLoader
    {
        async fn load_session(
            &self,
            _session_id: &str,
        ) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String>
        {
            Err(format!(
                "{} sqlite read failed at /private/secret/session.db token=raw-secret \
                 sql=SELECT * FROM terminal_records provider_payload={{\"prompt\":\"raw-provider-secret\"}}",
                "壊".repeat(1_000)
            ))
        }
    }

    #[tokio::test]
    async fn tauri_session_load_returns_safe_error_and_persists_canonical_feedback() {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let feedback = Arc::new(
            crate::usecase::agent_session::feedback::SessionFeedbackUsecase::new(
                store.clone(),
                store.generation_id().to_string(),
            ),
        );
        let load =
            crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase::new(
                Arc::new(UnreadableTauriSessionLoader),
                feedback.clone(),
            );

        let error = dispatch_feedback_supervised_session_load(
            &load,
            "unreadable-session",
            "tauri-load-attempt",
        )
        .await
        .expect_err("unreadable data/meta must fail the public load");
        let public = serde_json::to_value(error).unwrap();
        assert_eq!(public["type"], "storage_unavailable");
        assert_eq!(public["failure"]["kind"], "persist_failure");
        assert_eq!(
            public["failure"]["label"],
            "The session could not be loaded."
        );
        assert!(public["failure"]["label"].as_str().unwrap().len() <= 160);
        assert!(public["failure"]["detail"].as_str().unwrap().len() <= 2_048);
        assert!(!public.to_string().contains("raw-secret"));
        assert!(!public.to_string().contains("/private/secret"));
        assert!(!public.to_string().contains("SELECT * FROM"));
        assert!(!public.to_string().contains("raw-provider-secret"));

        let page = feedback
            .list("unreadable-session", 32, None)
            .await
            .expect("canonical feedback remains queryable without session data");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].attempt_id, "tauri-load-attempt");
        assert_eq!(
            page.entries[0].operation,
            crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation::LoadSession
        );
        assert_eq!(
            page.entries[0].actions,
            vec![crate::usecase::agent_session::feedback::FeedbackAction::Dismiss]
        );
        assert_eq!(
            page.entries[0].failure.correlation_id,
            public["failure"]["correlation_id"].as_str().unwrap()
        );
        assert!(
            crate::usecase::agent_session::session_feedback_load::session_load_failure_was_logged(
                public["failure"]["correlation_id"].as_str().unwrap(),
            ),
            "the exact public/feedback correlation identity must also reach the failure log",
        );
    }

    #[test]
    fn transaction_shutdown_rejection_keeps_the_public_admission_result_closed() {
        assert_eq!(
            normalize_mutation_error(
                "agent SQLite projection commit failed: storage unavailable: PreviousShutdownReconciliationRequired: Application shutdown is in progress"
                    .to_string(),
            ),
            "ShutdownInProgress"
        );
    }

    struct CrossSurfaceReplaySendGate;

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAdmissionGate for CrossSurfaceReplaySendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: "cross-surface-session".to_string(),
                initial_session: None,
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: "cross-surface-input".to_string(),
                human_message_id: "cross-surface-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "hello".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
                provider_established: true,
            })
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) {
        }
    }

    struct JournalResolveFaultSendGate {
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAdmissionGate for JournalResolveFaultSendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: "f03-journal-session".to_string(),
                initial_session: None,
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: "f03-journal-input".to_string(),
                human_message_id: "f03-journal-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "journal replay".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
                provider_established: true,
            })
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Acceptance is already committed here. Fail only the following
            // caller-journal resolution write so the public replay must heal it.
            self.store.fault_injector().arm_fail_before_begin();
        }
    }

    struct RealStoreNewSessionSendGate {
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        plan_calls: std::sync::atomic::AtomicUsize,
        effects: Arc<std::sync::atomic::AtomicUsize>,
        planning_allowed: bool,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAdmissionGate for RealStoreNewSessionSendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.plan_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if !self.planning_allowed {
                return Err(crate::domain::local_event::SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::Internal,
                    false,
                    "A response-loss replay must not plan another session.",
                    "b005-unexpected-replan",
                ));
            }
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: "b005-new-session".to_string(),
                initial_session: Some(
                    crate::usecase::agent_session::session::build_new_session_with_id(
                        "b005-new-session".to_string(),
                        "/tmp/b005-new-session",
                        Some("codex".to_string()),
                        crate::domain::agent_session::PermissionMode::Ask,
                        None,
                        false,
                        false,
                        None,
                    ),
                ),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: "b005-input".to_string(),
                human_message_id: "b005-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "hello".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
                provider_established: true,
            })
        }

        async fn acceptance_state_mutations(
            &self,
            plan: &crate::usecase::agent_session::operation::SendPlan,
            events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
        ) -> Result<
            Vec<crate::domain::local_event::LocalStateMutation>,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.session_store
                .prepare_send_acceptance_mutations(
                    crate::usecase::agent_session::session::SendAcceptanceProjectionInput {
                        session_id: &plan.session_id,
                        initial_session: plan.initial_session.as_ref(),
                        human_message_id: &plan.human_message_id,
                        prompt: &plan.prompt,
                        disposition: &plan.disposition,
                        reserved_turn_id: plan.reserved_turn_id.as_deref(),
                        input_ref: &plan.input_ref,
                        events,
                    },
                )
                .map_err(|_| {
                    crate::domain::local_event::SafeOperationFailure::new(
                        crate::domain::local_event::SessionOperationFailureKind::PersistFailure,
                        true,
                        "The send projection could not be prepared.",
                        "b005-projection",
                    )
                })
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn install_test_session_authority(
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    ) {
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository,
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
    }

    struct PublicTauriStopGate {
        turn_id: String,
        session_revision: u64,
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::StopAdmissionGate for PublicTauriStopGate {
        async fn target_snapshot(
            &self,
            _session_id: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::StopTargetSnapshot,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(
                crate::usecase::agent_session::operation::StopTargetSnapshot {
                    session_revision: self.session_revision,
                    active_turn_id: self.turn_id.clone(),
                    queue_paused: false,
                },
            )
        }

        async fn interrupt(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedStopEffect,
        ) -> Result<
            crate::usecase::agent_session::operation::StopEffectObservation,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(
                crate::usecase::agent_session::operation::StopEffectObservation {
                    terminal_reason: Some(
                        crate::domain::agent_session::events::InterruptReason::Abort,
                    ),
                },
            )
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum LifecycleCrashBoundary {
        BeforeAcceptance,
        AfterAcceptance,
        AfterEffectBeforeResult,
        AfterResultCommit,
    }

    struct RealStoreLifecycleGate {
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        snapshot: crate::usecase::agent_session::operation::SessionLifecycleSnapshot,
        projected_state: crate::usecase::agent_session::session::SessionState,
        backend_selection: Option<(String, String)>,
        boundary: LifecycleCrashBoundary,
        effects: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SessionLifecycleGate for RealStoreLifecycleGate {
        async fn session_snapshot(
            &self,
            _session_id: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SessionLifecycleSnapshot,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(self.snapshot.clone())
        }

        async fn acceptance_state_mutations(
            &self,
            session_id: &str,
            _action: &crate::usecase::agent_session::operation::SessionLifecycleAction,
            events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
        ) -> Result<
            Vec<crate::domain::local_event::LocalStateMutation>,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.session_store
                .prepare_lifecycle_acceptance_mutations(
                    session_id,
                    events,
                    self.projected_state.clone(),
                    self.backend_selection
                        .as_ref()
                        .map(|(backend_id, model_id)| (backend_id.as_str(), model_id.as_str())),
                )
                .map_err(|_| {
                    crate::domain::local_event::SafeOperationFailure::new(
                        crate::domain::local_event::SessionOperationFailureKind::PersistFailure,
                        true,
                        "The lifecycle projection could not be prepared.",
                        "b095-projection",
                    )
                })
        }

        async fn execute(
            &self,
            _effect: &crate::usecase::agent_session::operation::SessionLifecycleEffect,
        ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.boundary {
                LifecycleCrashBoundary::AfterEffectBeforeResult => {
                    self.store.fault_injector().arm_fail_before_begin();
                }
                LifecycleCrashBoundary::AfterResultCommit => {
                    self.store
                        .fault_injector()
                        .arm_crash_after_commit_before_readback();
                }
                LifecycleCrashBoundary::BeforeAcceptance
                | LifecycleCrashBoundary::AfterAcceptance => {}
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn b095_real_store_close_crash_boundaries_survive_restart_for_active_and_idle() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        for active in [false, true] {
            for boundary in [
                LifecycleCrashBoundary::BeforeAcceptance,
                LifecycleCrashBoundary::AfterAcceptance,
                LifecycleCrashBoundary::AfterEffectBeforeResult,
                LifecycleCrashBoundary::AfterResultCommit,
            ] {
                let data = tempfile::tempdir().unwrap();
                let session_id = format!(
                    "b095-{}-{}",
                    if active { "active" } else { "idle" },
                    match boundary {
                        LifecycleCrashBoundary::BeforeAcceptance => "before-acceptance",
                        LifecycleCrashBoundary::AfterAcceptance => "after-acceptance",
                        LifecycleCrashBoundary::AfterEffectBeforeResult => "after-effect",
                        LifecycleCrashBoundary::AfterResultCommit => "after-result",
                    }
                );
                let request_id = format!("close-{session_id}");
                let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let mut request =
                    crate::usecase::agent_session::operation::SessionLifecycleRequest {
                        principal: TAURI_OPERATION_PRINCIPAL.to_string(),
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        expected_session_revision: 0,
                        action:
                            crate::usecase::agent_session::operation::SessionLifecycleAction::Close,
                    };

                let first = {
                    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                            data.path().to_path_buf(),
                        ),
                    )
                    .unwrap();
                    let session_store = Arc::new(crate::test_support::build_session_store());
                    install_test_session_authority(&session_store, &store);
                    let mut session =
                        crate::usecase::agent_session::session::build_new_session_with_id(
                            session_id.clone(),
                            data.path().to_str().unwrap(),
                            Some("codex".to_string()),
                            crate::domain::agent_session::PermissionMode::Ask,
                            None,
                            false,
                            false,
                            None,
                        );
                    if !active {
                        session.state = crate::usecase::agent_session::session::SessionState::Idle;
                    }
                    session_store
                        .save_full_session_for_migration_or_restore(data.path(), &session)
                        .unwrap();
                    if active {
                        session_store
                            .append_session_event_and_project_state(
                                data.path(),
                                &session_id,
                                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted {
                                    turn_id: 1,
                                    message_id: "b095-human-1".to_string(),
                                    assistant_message_id: Some("b095-agent-1".to_string()),
                                    prompt: crate::domain::agent_session::events::PromptInput::default(),
                                    at: 1.0,
                                },
                            )
                            .unwrap();
                    }
                    let meta = session_store
                        .get_session_meta(data.path(), &session_id)
                        .unwrap()
                        .unwrap();
                    request.expected_session_revision = i64::try_from(meta.state_revision).unwrap();
                    let snapshot =
                        crate::usecase::agent_session::operation::SessionLifecycleSnapshot {
                            session_revision: i64::try_from(meta.state_revision).unwrap(),
                            lifecycle:
                                crate::usecase::agent_session::operation::SessionLifecycleState::Open {
                                    idle: !active,
                                    active_turn_id: active.then_some(1),
                                },
                            queue_paused: false,
                            has_runtime: true,
                            has_pending_permission: false,
                            has_pending_recovery: false,
                            has_pending_provider_operation: active,
                        };
                    let gate = Arc::new(RealStoreLifecycleGate {
                        session_store,
                        store: store.clone(),
                        snapshot,
                        projected_state:
                            crate::usecase::agent_session::session::SessionState::Closed,
                        backend_selection: None,
                        boundary,
                        effects: effects.clone(),
                    });
                    let usecase = crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase::new(
                        store.clone(),
                        store.clone(),
                        gate,
                        store.generation_id().to_string(),
                    );
                    match boundary {
                        LifecycleCrashBoundary::BeforeAcceptance => {
                            store.fault_injector().arm_fail_before_begin();
                        }
                        LifecycleCrashBoundary::AfterAcceptance => {
                            store
                                .fault_injector()
                                .arm_crash_after_commit_before_readback();
                        }
                        LifecycleCrashBoundary::AfterEffectBeforeResult
                        | LifecycleCrashBoundary::AfterResultCommit => {}
                    }
                    usecase.request(request.clone()).await.unwrap()
                };

                let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                    crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                        data.path().to_path_buf(),
                    ),
                )
                .unwrap();
                let session_store = Arc::new(crate::test_support::build_session_store());
                install_test_session_authority(&session_store, &store);
                let meta = session_store
                    .get_session_meta(data.path(), &session_id)
                    .unwrap()
                    .unwrap();
                let stream = store
                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                        stream_id: crate::domain::local_event::StreamId::agent_session(&session_id)
                            .unwrap(),
                        after: None,
                        limit: 64,
                    })
                    .await
                    .unwrap();
                let count_event = |predicate: fn(
                    &crate::domain::agent_session::events::AgentSessionDomainEvent,
                ) -> bool| {
                    stream
                        .events
                        .iter()
                        .filter(|event| match &event.event {
                            crate::domain::local_event::LoadedDomainEvent::Known(event) => {
                                match event.as_ref() {
                                    crate::domain::local_event::LocalDomainEvent::AgentSession(
                                        event,
                                    ) => predicate(event),
                                    _ => false,
                                }
                            }
                            crate::domain::local_event::LoadedDomainEvent::Unknown { .. } => false,
                        })
                        .count()
                };
                let session_closed = count_event(|event| {
                    matches!(
                        event,
                        crate::domain::agent_session::events::AgentSessionDomainEvent::SessionClosed { .. }
                    )
                });
                let queue_paused = count_event(|event| {
                    matches!(
                        event,
                        crate::domain::agent_session::events::AgentSessionDomainEvent::QueuePaused { .. }
                    )
                });
                let interrupted = count_event(|event| {
                    matches!(
                        event,
                        crate::domain::agent_session::events::AgentSessionDomainEvent::TurnInterrupted {
                            turn_id: 1,
                            reason: crate::domain::agent_session::events::InterruptReason::SessionClosed,
                            ..
                        }
                    )
                });

                if boundary == LifecycleCrashBoundary::BeforeAcceptance {
                    assert!(matches!(
                        first,
                        crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Rejected(_)
                    ));
                    assert_eq!(
                        meta.state,
                        if active {
                            crate::usecase::agent_session::session::SessionState::Active
                        } else {
                            crate::usecase::agent_session::session::SessionState::Idle
                        }
                    );
                    assert!(session_store
                        .load_queue_paused_at(data.path(), &session_id)
                        .unwrap()
                        .is_none());
                    assert_eq!(session_closed, 0);
                    assert_eq!(queue_paused, 0);
                    assert_eq!(interrupted, 0);
                    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 0);
                    continue;
                }

                assert_eq!(
                    meta.state,
                    crate::usecase::agent_session::session::SessionState::Closed
                );
                assert!(
                    session_store
                        .load_queue_paused_at(data.path(), &session_id)
                        .unwrap()
                        .is_some(),
                    "active={active}, boundary={boundary:?}"
                );
                assert_eq!(session_closed, 1);
                assert_eq!(queue_paused, 1);
                assert_eq!(interrupted, usize::from(active));
                let terminal = store
                    .query(
                        crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                            session_id: session_id.clone(),
                            turn_id: "1".to_string(),
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    matches!(
                        terminal,
                        crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(Some(_))
                    ),
                    active,
                    "Idle close must not manufacture a synthetic terminal"
                );

                let restart_gate = Arc::new(RealStoreLifecycleGate {
                    session_store,
                    store: store.clone(),
                    snapshot: crate::usecase::agent_session::operation::SessionLifecycleSnapshot {
                        session_revision: i64::try_from(meta.state_revision).unwrap(),
                        lifecycle:
                            crate::usecase::agent_session::operation::SessionLifecycleState::Closed,
                        queue_paused: true,
                        has_runtime: false,
                        has_pending_permission: false,
                        has_pending_recovery: false,
                        has_pending_provider_operation: false,
                    },
                    projected_state: crate::usecase::agent_session::session::SessionState::Closed,
                    backend_selection: None,
                    boundary,
                    effects: effects.clone(),
                });
                let restarted =
                    crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase::new(
                        store.clone(),
                        store.clone(),
                        restart_gate,
                        store.generation_id().to_string(),
                    );
                let replay = restarted.request(request.clone()).await.unwrap();
                let crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Accepted {
                    receipt: replay_receipt,
                    state: replay_state,
                } = replay
                else {
                    panic!("accepted close must replay after restart");
                };
                assert_eq!(replay_receipt.session_id, session_id);
                let (lookup_receipt, lookup_state) = restarted
                    .get_operation(TAURI_OPERATION_PRINCIPAL, &request_id)
                    .await
                    .unwrap();
                assert_eq!(lookup_receipt, replay_receipt);
                assert_eq!(lookup_state, replay_state);
                match boundary {
                    LifecycleCrashBoundary::AfterAcceptance
                    | LifecycleCrashBoundary::AfterEffectBeforeResult => assert!(matches!(
                        replay_state,
                        crate::usecase::agent_session::operation::SessionLifecycleOperationState::ReconciliationRequired { .. }
                    )),
                    LifecycleCrashBoundary::AfterResultCommit => assert_eq!(
                        replay_state,
                        crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed
                    ),
                    LifecycleCrashBoundary::BeforeAcceptance => unreachable!(),
                }
                let expected_effects = usize::from(matches!(
                    boundary,
                    LifecycleCrashBoundary::AfterEffectBeforeResult
                        | LifecycleCrashBoundary::AfterResultCommit
                ));
                assert_eq!(
                    effects.load(std::sync::atomic::Ordering::SeqCst),
                    expected_effects
                );
            }
        }
    }

    struct RealLifecycleBehaviorOutcome {
        _data: tempfile::TempDir,
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        session_id: String,
        before_state: crate::usecase::agent_session::session::SessionState,
        before_revision: u64,
        before_backend: String,
        before_event_count: usize,
        effects: Arc<std::sync::atomic::AtomicUsize>,
        result: crate::usecase::agent_session::operation::SessionLifecycleCommandResult,
    }

    async fn run_real_lifecycle_behavior(
        identity: &str,
        initial_state: crate::usecase::agent_session::session::SessionState,
        active: bool,
        action: crate::usecase::agent_session::operation::SessionLifecycleAction,
        projected_state: crate::usecase::agent_session::session::SessionState,
        backend_selection: Option<(String, String)>,
        configure_snapshot: impl FnOnce(
            &mut crate::usecase::agent_session::operation::SessionLifecycleSnapshot,
        ),
    ) -> RealLifecycleBehaviorOutcome {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        install_test_session_authority(&session_store, &store);
        let session_id = format!("b051-{identity}");
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data.path().to_str().unwrap(),
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session.state = initial_state.clone();
        session_store
            .save_full_session_for_migration_or_restore(data.path(), &session)
            .unwrap();
        if active {
            session_store
                .append_session_event_and_project_state(
                    data.path(),
                    &session_id,
                    crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted {
                        turn_id: 1,
                        message_id: format!("{identity}-human"),
                        assistant_message_id: Some(format!("{identity}-agent")),
                        prompt: crate::domain::agent_session::events::PromptInput::default(),
                        at: 1.0,
                    },
                )
                .unwrap();
        }
        let before = session_store
            .get_session_meta(data.path(), &session_id)
            .unwrap()
            .unwrap();
        let before_stream = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session(&session_id)
                    .unwrap(),
                after: None,
                limit: 64,
            })
            .await
            .unwrap();
        let lifecycle = match before.state {
            crate::usecase::agent_session::session::SessionState::Closed => {
                crate::usecase::agent_session::operation::SessionLifecycleState::Closed
            }
            crate::usecase::agent_session::session::SessionState::Archived => {
                crate::usecase::agent_session::operation::SessionLifecycleState::Archived
            }
            _ => crate::usecase::agent_session::operation::SessionLifecycleState::Open {
                idle: !active,
                active_turn_id: active.then_some(1),
            },
        };
        let mut snapshot = crate::usecase::agent_session::operation::SessionLifecycleSnapshot {
            session_revision: i64::try_from(before.state_revision).unwrap(),
            lifecycle,
            queue_paused: false,
            has_runtime: before.state
                != crate::usecase::agent_session::session::SessionState::Closed,
            has_pending_permission: false,
            has_pending_recovery: false,
            has_pending_provider_operation: active,
        };
        configure_snapshot(&mut snapshot);
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gate = Arc::new(RealStoreLifecycleGate {
            session_store: session_store.clone(),
            store: store.clone(),
            snapshot,
            projected_state,
            backend_selection,
            boundary: LifecycleCrashBoundary::AfterAcceptance,
            effects: effects.clone(),
        });
        let usecase =
            crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate,
                store.generation_id().to_string(),
            );
        let result = usecase
            .request(
                crate::usecase::agent_session::operation::SessionLifecycleRequest {
                    principal: TAURI_OPERATION_PRINCIPAL.to_string(),
                    request_id: format!("request-{identity}"),
                    session_id: session_id.clone(),
                    expected_session_revision: i64::try_from(before.state_revision).unwrap(),
                    action,
                },
            )
            .await
            .unwrap();
        RealLifecycleBehaviorOutcome {
            _data: data,
            store,
            session_store,
            session_id,
            before_state: before.state,
            before_revision: before.state_revision,
            before_backend: before.backend_id,
            before_event_count: before_stream.events.len(),
            effects,
            result,
        }
    }

    async fn real_lifecycle_events(
        outcome: &RealLifecycleBehaviorOutcome,
    ) -> Vec<crate::domain::agent_session::events::AgentSessionDomainEvent> {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        outcome
            .store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session(&outcome.session_id)
                    .unwrap(),
                after: None,
                limit: 64,
            })
            .await
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.event {
                crate::domain::local_event::LoadedDomainEvent::Known(event) => match *event {
                    crate::domain::local_event::LocalDomainEvent::AgentSession(event) => {
                        Some(event)
                    }
                    _ => None,
                },
                crate::domain::local_event::LoadedDomainEvent::Unknown { .. } => None,
            })
            .collect()
    }

    async fn assert_real_close_or_open_archive(
        identity: &str,
        active: bool,
        action: crate::usecase::agent_session::operation::SessionLifecycleAction,
        expected_state: crate::usecase::agent_session::session::SessionState,
    ) {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        let outcome = run_real_lifecycle_behavior(
            identity,
            if active {
                crate::usecase::agent_session::session::SessionState::Active
            } else {
                crate::usecase::agent_session::session::SessionState::Idle
            },
            active,
            action,
            expected_state.clone(),
            None,
            |_| {},
        )
        .await;
        assert!(matches!(
            outcome.result,
            crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Accepted {
                state: crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed,
                ..
            }
        ));
        assert_eq!(outcome.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        let after = outcome
            .session_store
            .get_session_meta(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.state, expected_state);
        assert!(outcome
            .session_store
            .load_queue_paused_at(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .is_some());
        let events = real_lifecycle_events(&outcome).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::domain::agent_session::events::AgentSessionDomainEvent::TurnInterrupted { .. }))
                .count(),
            usize::from(active)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::domain::agent_session::events::AgentSessionDomainEvent::SessionClosed { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::domain::agent_session::events::AgentSessionDomainEvent::QueuePaused { .. }))
                .count(),
            1
        );
        let terminal = outcome
            .store
            .query(
                crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                    session_id: outcome.session_id.clone(),
                    turn_id: "1".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            matches!(
                terminal,
                crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(Some(_))
            ),
            active
        );
    }

    #[tokio::test]
    async fn close_quit_active_session_close_commits_terminal_and_pause() {
        assert_real_close_or_open_archive(
            "active-close",
            true,
            crate::usecase::agent_session::operation::SessionLifecycleAction::Close,
            crate::usecase::agent_session::session::SessionState::Closed,
        )
        .await;
    }

    #[tokio::test]
    async fn close_quit_idle_session_close_has_no_synthetic_terminal() {
        assert_real_close_or_open_archive(
            "idle-close",
            false,
            crate::usecase::agent_session::operation::SessionLifecycleAction::Close,
            crate::usecase::agent_session::session::SessionState::Closed,
        )
        .await;
    }

    #[tokio::test]
    async fn close_quit_active_open_archive_commits_terminal_and_pause() {
        assert_real_close_or_open_archive(
            "active-open-archive",
            true,
            crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveOpen,
            crate::usecase::agent_session::session::SessionState::Archived,
        )
        .await;
    }

    #[tokio::test]
    async fn close_quit_idle_open_archive_has_no_synthetic_terminal() {
        assert_real_close_or_open_archive(
            "idle-open-archive",
            false,
            crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveOpen,
            crate::usecase::agent_session::session::SessionState::Archived,
        )
        .await;
    }

    #[tokio::test]
    async fn close_quit_closed_archive_changes_projection_only() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        let outcome = run_real_lifecycle_behavior(
            "closed-archive",
            crate::usecase::agent_session::session::SessionState::Closed,
            false,
            crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveClosed,
            crate::usecase::agent_session::session::SessionState::Archived,
            None,
            |_| {},
        )
        .await;
        assert!(matches!(
            outcome.result,
            crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Accepted {
                state: crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed,
                ..
            }
        ));
        assert_eq!(outcome.effects.load(std::sync::atomic::Ordering::SeqCst), 0);
        let after = outcome
            .session_store
            .get_session_meta(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            after.state,
            crate::usecase::agent_session::session::SessionState::Archived
        );
        assert!(outcome
            .session_store
            .load_queue_paused_at(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .is_none());
        let events = real_lifecycle_events(&outcome).await;
        assert_eq!(events.len(), outcome.before_event_count + 1);
        assert!(!events.iter().any(|event| matches!(
            event,
            crate::domain::agent_session::events::AgentSessionDomainEvent::TurnInterrupted { .. }
                | crate::domain::agent_session::events::AgentSessionDomainEvent::SessionClosed { .. }
                | crate::domain::agent_session::events::AgentSessionDomainEvent::QueuePaused { .. }
                | crate::domain::agent_session::events::AgentSessionDomainEvent::ObligationRecorded { .. }
        )));
        let terminal = outcome
            .store
            .query(
                crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                    session_id: outcome.session_id.clone(),
                    turn_id: "1".to_string(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            terminal,
            crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(None)
        ));
    }

    #[tokio::test]
    async fn close_quit_idle_backend_switch_is_ack_driven() {
        let outcome = run_real_lifecycle_behavior(
            "idle-backend-switch",
            crate::usecase::agent_session::session::SessionState::Idle,
            false,
            crate::usecase::agent_session::operation::SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
            crate::usecase::agent_session::session::SessionState::Idle,
            Some(("codex".to_string(), "codex-model".to_string())),
            |_| {},
        )
        .await;
        assert!(matches!(
            outcome.result,
            crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Accepted {
                state: crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed,
                ..
            }
        ));
        assert_eq!(
            outcome.effects.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the old runtime must acknowledge exactly one close before Completed is returned"
        );
        let after = outcome
            .session_store
            .get_session_meta(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.backend_id, "codex");
        assert_eq!(after.selected_model.as_deref(), Some("codex-model"));
        assert_eq!(
            after.state,
            crate::usecase::agent_session::session::SessionState::Idle
        );
        assert!(outcome
            .session_store
            .load_queue_paused_at(outcome._data.path(), &outcome.session_id)
            .unwrap()
            .is_some());
        let events = real_lifecycle_events(&outcome).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, crate::domain::agent_session::events::AgentSessionDomainEvent::QueuePaused { .. }))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            crate::domain::agent_session::events::AgentSessionDomainEvent::TurnInterrupted { .. }
                | crate::domain::agent_session::events::AgentSessionDomainEvent::SessionClosed { .. }
        )));
    }

    #[tokio::test]
    async fn close_quit_backend_switch_rejects_active_or_pending_session() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        for (label, active, expected, configure) in [
            (
                "active",
                true,
                crate::usecase::agent_session::operation::SessionLifecycleRejection::Busy,
                0_u8,
            ),
            (
                "pending-permission",
                false,
                crate::usecase::agent_session::operation::SessionLifecycleRejection::InvalidState,
                1,
            ),
            (
                "pending-recovery",
                false,
                crate::usecase::agent_session::operation::SessionLifecycleRejection::InvalidState,
                2,
            ),
            (
                "pending-provider",
                false,
                crate::usecase::agent_session::operation::SessionLifecycleRejection::InvalidState,
                3,
            ),
        ] {
            let outcome = run_real_lifecycle_behavior(
                &format!("backend-switch-reject-{label}"),
                if active {
                    crate::usecase::agent_session::session::SessionState::Active
                } else {
                    crate::usecase::agent_session::session::SessionState::Idle
                },
                active,
                crate::usecase::agent_session::operation::SessionLifecycleAction::SwitchBackend {
                    backend_id: "codex".to_string(),
                },
                crate::usecase::agent_session::session::SessionState::Idle,
                Some(("codex".to_string(), "codex-model".to_string())),
                |snapshot| match configure {
                    1 => snapshot.has_pending_permission = true,
                    2 => snapshot.has_pending_recovery = true,
                    3 => snapshot.has_pending_provider_operation = true,
                    _ => {}
                },
            )
            .await;
            assert_eq!(
                outcome.result,
                crate::usecase::agent_session::operation::SessionLifecycleCommandResult::Rejected(
                    expected
                ),
                "case={label}"
            );
            assert_eq!(
                outcome.effects.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "case={label}"
            );
            let after = outcome
                .session_store
                .get_session_meta(outcome._data.path(), &outcome.session_id)
                .unwrap()
                .unwrap();
            assert_eq!(after.state, outcome.before_state, "case={label}");
            assert_eq!(
                after.state_revision, outcome.before_revision,
                "case={label}"
            );
            assert_eq!(after.backend_id, outcome.before_backend, "case={label}");
            assert!(outcome
                .session_store
                .load_queue_paused_at(outcome._data.path(), &outcome.session_id)
                .unwrap()
                .is_none());
            let stream = outcome
                .store
                .load_stream(crate::domain::local_event::LoadStreamRequest {
                    stream_id: crate::domain::local_event::StreamId::agent_session(
                        &outcome.session_id,
                    )
                    .unwrap(),
                    after: None,
                    limit: 64,
                })
                .await
                .unwrap();
            assert_eq!(
                stream.events.len(),
                outcome.before_event_count,
                "case={label}"
            );
        }
    }

    #[tokio::test]
    async fn b075_b087_tauri_stop_rejects_invalid_identity_and_turn_before_effect() {
        use tauri::Manager as _;

        let cases = [
            ("a".to_string(), "1".to_string(), "0".to_string(), true),
            ("a".repeat(128), "1".to_string(), "0".to_string(), true),
            (
                "turn-max".to_string(),
                i64::MAX.to_string(),
                "0".to_string(),
                true,
            ),
            (
                "revision-one".to_string(),
                "1".to_string(),
                "1".to_string(),
                true,
            ),
            (
                "revision-max".to_string(),
                "1".to_string(),
                i64::MAX.to_string(),
                true,
            ),
            (String::new(), "1".to_string(), "0".to_string(), false),
            ("a".repeat(129), "1".to_string(), "0".to_string(), false),
            (
                "非ascii".to_string(),
                "1".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "bad/id".to_string(),
                "1".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-zero".to_string(),
                "0".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-leading-zero".to_string(),
                "01".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-plus".to_string(),
                "+1".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-negative".to_string(),
                "-1".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-exponent".to_string(),
                "1e0".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-unicode".to_string(),
                "１".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-leading-space".to_string(),
                " 1".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-trailing-space".to_string(),
                "1 ".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "turn-overflow".to_string(),
                "9223372036854775808".to_string(),
                "0".to_string(),
                false,
            ),
            (
                "revision-empty".to_string(),
                "1".to_string(),
                String::new(),
                false,
            ),
            (
                "revision-leading-zero".to_string(),
                "1".to_string(),
                "01".to_string(),
                false,
            ),
            (
                "revision-plus".to_string(),
                "1".to_string(),
                "+1".to_string(),
                false,
            ),
            (
                "revision-negative".to_string(),
                "1".to_string(),
                "-1".to_string(),
                false,
            ),
            (
                "revision-exponent".to_string(),
                "1".to_string(),
                "1e0".to_string(),
                false,
            ),
            (
                "revision-unicode".to_string(),
                "1".to_string(),
                "１".to_string(),
                false,
            ),
            (
                "revision-leading-space".to_string(),
                "1".to_string(),
                " 1".to_string(),
                false,
            ),
            (
                "revision-trailing-space".to_string(),
                "1".to_string(),
                "1 ".to_string(),
                false,
            ),
            (
                "revision-overflow".to_string(),
                "1".to_string(),
                "9223372036854775808".to_string(),
                false,
            ),
        ];

        for (request_id, turn_id, expected_session_revision, valid) in cases {
            let data = tempfile::tempdir().unwrap();
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    data.path().to_path_buf(),
                ),
            )
            .unwrap();
            let gate = Arc::new(PublicTauriStopGate {
                turn_id: turn_id.clone(),
                session_revision: decode_nonnegative_u64_decimal(&expected_session_revision)
                    .unwrap_or(0),
                effects: std::sync::atomic::AtomicUsize::new(0),
            });
            let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
                store.clone();
            let authority: Arc<
                dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
            > = store.clone();
            let usecase = Arc::new(
                crate::usecase::agent_session::operation::StopOperationUsecase::new(
                    repository,
                    authority,
                    gate.clone(),
                    store.generation_id().to_string(),
                ),
            );
            let journal = Arc::new(
                crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                    store.clone(),
                    store.clone(),
                    store.generation_id().to_string(),
                ),
            );
            let app = tauri::test::mock_builder()
                .manage(store)
                .manage(usecase)
                .manage(journal)
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let result = stop_agent_session(
                app.state::<Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>(),
                app.state::<Arc<crate::usecase::agent_session::operation::StopOperationUsecase>>(),
                app.state::<Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>(),
                StopOperationRequestDtoV1 {
                    request_id: request_id.clone(),
                    session_id: "b087-tauri-session".to_string(),
                    turn_id,
                    expected_session_revision,
                },
            )
            .await;
            if valid {
                let public = serde_json::to_value(result.expect("valid Stop identity")).unwrap();
                assert_eq!(public["type"], "accepted", "{request_id:?}");
                assert_eq!(
                    gate.effects.load(std::sync::atomic::Ordering::SeqCst),
                    1,
                    "{request_id:?}"
                );
            } else {
                let public =
                    serde_json::to_value(result.expect_err("invalid Stop request")).unwrap();
                assert_eq!(public["type"], "invalid_request", "{request_id:?}");
                assert_eq!(
                    gate.effects.load(std::sync::atomic::Ordering::SeqCst),
                    0,
                    "{request_id:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn b005_tauri_new_session_response_loss_restarts_to_one_canonical_receipt() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;

        let data = tempfile::tempdir().unwrap();
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let command = CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id: None,
                worktree_path: "/tmp/b005-new-session".to_string(),
            },
            content: "hello".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("codex".to_string()),
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };

        let first_json = {
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    data.path().to_path_buf(),
                ),
            )
            .unwrap();
            let session_store = Arc::new(crate::test_support::build_session_store());
            install_test_session_authority(&session_store, &store);
            let gate = Arc::new(RealStoreNewSessionSendGate {
                session_store,
                plan_calls: std::sync::atomic::AtomicUsize::new(0),
                effects: effects.clone(),
                planning_allowed: true,
            });
            let send = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate.clone(),
                store.generation_id().to_string(),
            );
            let journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                store.clone(),
                store.clone(),
                store.generation_id().to_string(),
            );
            let response = dispatch_durable_send(
                store.as_ref(),
                &send,
                &journal,
                "b005-operation".to_string(),
                command.clone(),
            )
            .await
            .expect("initial new-session send");
            assert_eq!(gate.plan_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
            serde_json::to_value(response).unwrap()
        };

        // The first public response is deliberately discarded before a fresh
        // store/usecase composition replays the exact caller identity.
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        install_test_session_authority(&session_store, &store);
        let restarted_gate = Arc::new(RealStoreNewSessionSendGate {
            session_store: session_store.clone(),
            plan_calls: std::sync::atomic::AtomicUsize::new(0),
            effects: effects.clone(),
            planning_allowed: false,
        });
        let restarted_send =
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                restarted_gate.clone(),
                store.generation_id().to_string(),
            );
        let restarted_journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            store.clone(),
            store.clone(),
            store.generation_id().to_string(),
        );
        let replay = dispatch_durable_send(
            store.as_ref(),
            &restarted_send,
            &restarted_journal,
            "b005-operation".to_string(),
            command,
        )
        .await
        .expect("restart replay");
        assert_eq!(serde_json::to_value(replay).unwrap(), first_json);
        assert_eq!(
            restarted_gate
                .plan_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);

        let (session, page, _) = session_store
            .get_session_with_latest_page(data.path(), "b005-new-session", 32)
            .unwrap()
            .expect("one durable new session");
        assert_eq!(session.id, "b005-new-session");
        assert_eq!(
            page.messages
                .iter()
                .filter(|message| {
                    message.id == "b005-human"
                        && message.role
                            == crate::usecase::agent_session::session::MessageRole::Human
                })
                .count(),
            1
        );
        let stream = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session("b005-new-session")
                    .unwrap(),
                after: None,
                limit: 32,
            })
            .await
            .unwrap();
        assert_eq!(
            stream
                .events
                .iter()
                .filter(|event| matches!(
                    &event.event,
                    crate::domain::local_event::LoadedDomainEvent::Known(event)
                        if matches!(
                            event.as_ref(),
                            crate::domain::local_event::LocalDomainEvent::AgentSession(
                                crate::domain::agent_session::events::AgentSessionDomainEvent::SendOperationAccepted { operation_id, .. }
                            ) if operation_id == "b005-operation"
                        )
                ))
                .count(),
            1
        );
        assert_eq!(
            stream
                .events
                .iter()
                .filter(|event| matches!(
                    &event.event,
                    crate::domain::local_event::LoadedDomainEvent::Known(event)
                        if matches!(
                            event.as_ref(),
                            crate::domain::local_event::LocalDomainEvent::AgentSession(
                                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted { turn_id: 1, .. }
                            )
                        )
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn f03_post_acceptance_journal_resolution_is_retried_by_the_shared_dispatcher() {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let gate = Arc::new(JournalResolveFaultSendGate {
            store: store.clone(),
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let send = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
            store.clone(),
            store.clone(),
            gate.clone(),
            store.generation_id().to_string(),
        );
        let journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            store.clone(),
            store.clone(),
            store.generation_id().to_string(),
        );
        let command = CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id: Some("f03-journal-session".to_string()),
                worktree_path: "/repo".to_string(),
            },
            content: "journal replay".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("codex".to_string()),
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };

        let first = dispatch_durable_send(
            store.as_ref(),
            &send,
            &journal,
            "f03-journal-operation".to_string(),
            command.clone(),
        )
        .await
        .unwrap();
        assert!(matches!(first, SendCommandOutcomeDtoV1::Accepted { .. }));
        assert_eq!(
            journal
                .pending_page_for_scope(TAURI_OPERATION_PRINCIPAL, "f03-journal-session", 8, None,)
                .await
                .unwrap()
                .entries[0]
                .resolution,
            crate::domain::local_event::CallerAttemptResolution::Pending,
            "the injected post-acceptance write loss leaves the exact caller identity pending"
        );

        let replay = dispatch_durable_send(
            store.as_ref(),
            &send,
            &journal,
            "f03-journal-operation".to_string(),
            command,
        )
        .await
        .unwrap();
        assert!(matches!(replay, SendCommandOutcomeDtoV1::Accepted { .. }));
        assert_eq!(
            journal
                .pending_page_for_scope(TAURI_OPERATION_PRINCIPAL, "f03-journal-session", 8, None,)
                .await
                .unwrap()
                .entries[0]
                .resolution,
            crate::domain::local_event::CallerAttemptResolution::Accepted
        );
        assert_eq!(
            gate.effects.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the healed journal replay must not start a second provider effect"
        );
    }

    #[tokio::test]
    async fn cross_surface_send_replay_bypasses_closed_admission_and_tauri_journal_creation() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;
        use crate::usecase::agent_session::operation::OperationBindingAuthority as _;

        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let send = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
            store.clone(),
            store.clone(),
            Arc::new(CrossSurfaceReplaySendGate),
            store.generation_id().to_string(),
        );
        let journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            store.clone(),
            store.clone(),
            store.generation_id().to_string(),
        );
        let command = CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id: Some("cross-surface-session".to_string()),
                worktree_path: "/repo".to_string(),
            },
            content: "hello".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: None,
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };
        let canonical_payload = serde_json::to_string(&command).unwrap();
        let accepted = send
            .send(
                crate::usecase::agent_session::operation::SendOperationRequest {
                    principal: TAURI_OPERATION_PRINCIPAL.to_string(),
                    operation_id: "cross-surface-send".to_string(),
                    canonical_payload,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            accepted,
            crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(_)
        ));

        let plan = crate::domain::local_event::ShutdownPlanKey {
            plan_id: "cross-surface-shutdown".to_string(),
            epoch: 1,
        };
        store
            .commit_batch(crate::domain::local_event::LocalAtomicBatch {
                commit_id: crate::domain::local_event::CommitIdentity::parse(
                    "cross-surface-shutdown-commit",
                )
                .unwrap(),
                idempotency: crate::domain::local_event::IdempotencyBinding {
                    generation_id: store.generation_id().to_string(),
                    operation_kind: crate::domain::local_event::OperationKind::ApplicationQuit
                        .into(),
                    idempotency_key: "cross-surface-shutdown".to_string(),
                    payload_hash: store.digest(b"cross-surface-shutdown"),
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![
                    crate::domain::local_event::LocalStateMutation::ShutdownPlan(
                        crate::domain::local_event::ShutdownPlanMutation {
                            key: plan.clone(),
                            phase: crate::domain::local_event::ApplicationShutdownPhase::Preparing,
                            summary: crate::domain::local_event::ShutdownPlanRecord {
                                operation_id: plan.plan_id.clone(),
                                intent: crate::domain::local_event::QuitIntent::Exit { code: 0 },
                                t0_ms: 0,
                                preparation_cutoff_ms: Some(13_000),
                                deadline_ms: 15_000,
                                target_count: Some(0),
                                prepared_count: Some(0),
                                effect_reserved_count: Some(0),
                                terminal_count: Some(0),
                                completed_count: Some(0),
                                unresolved_count: Some(0),
                                recovery_snapshot_count: Some(0),
                                recovery_snapshot_id: None,
                                boot_id: "cross-surface-test-boot".to_string(),
                                outcome: None,
                                failure: None,
                                shutdown_effect_count: None,
                                admission_open: None,
                                retry_quit_same_boot: None,
                            },
                            details_state:
                                crate::domain::local_event::ShutdownDetailsState::Available,
                            expected: crate::domain::local_event::RevisionGuard::Absent,
                            revision: crate::domain::local_event::Revision::new(0).unwrap(),
                        },
                    ),
                    crate::domain::local_event::LocalStateMutation::ShutdownLatestPointer(
                        crate::domain::local_event::ShutdownLatestPointerMutation {
                            expected: None,
                            new: Some(plan),
                        },
                    ),
                ],
            })
            .await
            .unwrap();
        assert!(matches!(
            ensure_mutation_admission(store.as_ref()).await,
            Err(OperationApplicationErrorDtoV1::ShutdownInProgress)
        ));
        store.fault_injector().arm_fail_before_begin();

        let replay = dispatch_durable_send(
            store.as_ref(),
            &send,
            &journal,
            "cross-surface-send".to_string(),
            command,
        )
        .await
        .unwrap();
        assert!(matches!(replay, SendCommandOutcomeDtoV1::Accepted { .. }));
        assert!(journal
            .pending_page_for_scope(TAURI_OPERATION_PRINCIPAL, "cross-surface-session", 8, None,)
            .await
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn resume_agent_queue_errors_preserve_typed_variants_and_string_wire_format() {
        let startup =
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::StartupTimeout {
                retry_count: 1,
                max_retries: 2,
            };
        let startup_message = startup.to_string();
        let startup = AppError::from(startup);
        assert!(matches!(
            startup,
            AppError::AgentStartupTimeout {
                retry_count: 1,
                max_retries: 2
            }
        ));
        assert_eq!(serde_json::to_value(&startup).unwrap(), startup_message);

        let other = AppError::from(
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::Other(
                "resume failed".to_string(),
            ),
        );
        assert!(matches!(other, AppError::Internal(ref message) if message == "resume failed"));
        assert_eq!(serde_json::to_value(&other).unwrap(), "resume failed");
    }

    // Spec issues-947: Tauri invoke 境界で permission_mode の欠落・対象外値を拒否する。
    // start_agent_session 内部の `validate_invoke_permission_mode` を command 相当の経路として
    // 直接呼び、欠落/旧語彙/未知語彙/空文字いずれも `?` で早期 return することを確認する
    // （= `update_permission_mode` も `start_agent_session_internal` も呼ばれない）。
    #[test]
    fn start_agent_session_validate_rejects_missing_or_invalid_permission_mode() {
        let invalid_inputs: Vec<Option<String>> = vec![
            None,
            Some(String::new()),
            Some("acceptEdits".to_string()),
            Some("bypassPermissions".to_string()),
            Some("plan".to_string()),
            Some("default".to_string()),
            Some("unknown".to_string()),
        ];
        for permission in invalid_inputs {
            let label = permission.clone();
            let err = validate_invoke_permission_mode(permission).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "{:?} must include allowed list, got: {err}",
                label
            );
        }
    }

    #[test]
    fn start_agent_session_validate_accepts_abstract_modes() {
        for mode in ["ask", "edit", "full"] {
            let validated = validate_invoke_permission_mode(Some(mode.to_string())).unwrap();
            assert_eq!(validated.as_str(), mode);
        }
    }

    // Tauri invoke 境界が拒否したとき、保存値が変更されないことを
    // 上位の command 経路を模した手順で確認する。
    #[test]
    fn start_agent_session_invalid_permission_mode_does_not_mutate_persisted_state() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            error_reason: None,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::claude::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        };
        store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        for invalid in [None, Some(String::new()), Some("acceptEdits".to_string())] {
            let result = validate_invoke_permission_mode(invalid.clone());
            assert!(result.is_err(), "{invalid:?} must be rejected");
            // command 本体は ? で早期 return するため、保存値は不変。
            let saved = store
                .load_full_session_for_restore(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.permission_mode, "edit");
        }
    }
}

pub(crate) async fn dispatch_durable_send(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
    send_operation: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    journal: &crate::usecase::agent_session::operation::CallerAttemptJournal,
    operation_id: String,
    command: CanonicalSendCommandV1,
) -> Result<SendCommandOutcomeDtoV1, SendCommandErrorDtoV1> {
    dispatch_durable_send_for_principal(
        store,
        send_operation,
        journal,
        TAURI_OPERATION_PRINCIPAL,
        operation_id,
        command,
    )
    .await
}

pub(crate) async fn dispatch_durable_send_for_principal(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
    send_operation: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    journal: &crate::usecase::agent_session::operation::CallerAttemptJournal,
    principal: &str,
    operation_id: String,
    command: CanonicalSendCommandV1,
) -> Result<SendCommandOutcomeDtoV1, SendCommandErrorDtoV1> {
    let canonical_payload =
        serde_json::to_string(&command).map_err(|_| SendCommandErrorDtoV1::InvalidRequest)?;
    // New-session attempts do not have a session identity until the atomic
    // acceptance commit chooses one. Keep them in the bounded application
    // scope so a response loss before the UI learns that identity is still
    // discoverable after restart.
    let attempt_scope = match &command.target {
        CanonicalSendTargetV1::Direct {
            chat_session_id: Some(session_id),
            ..
        } => session_id.as_str(),
        CanonicalSendTargetV1::Direct {
            chat_session_id: None,
            ..
        } => "application",
        CanonicalSendTargetV1::WorkflowApproval { execution_id } => execution_id.as_str(),
        CanonicalSendTargetV1::WorkflowTurn { .. } => {
            return Err(SendCommandErrorDtoV1::InvalidRequest);
        }
    };

    // Existing operation lookup must precede mutable admission. This lets a
    // same-payload retry converge on its immutable receipt after shutdown or
    // migration admission has changed. The subsequent send call still checks
    // the exact payload binding and returns PayloadConflict when necessary.
    let replaying_existing = match send_operation.get_operation(principal, &operation_id).await {
        Ok(_) => true,
        Err(crate::usecase::agent_session::operation::GetSendOperationError::OutcomeUnknown {
            ..
        }) => {
            // A durable caller-journal Pending row owns this identity. The
            // exact-command journal replay below must validate it and is the
            // only path allowed to continue resolving the same Tauri attempt.
            false
        }
        Err(crate::usecase::agent_session::operation::GetSendOperationError::NotFound) => {
            ensure_mutation_admission(store)
                .await
                .map_err(send_command_common)?;
            false
        }
        Err(crate::usecase::agent_session::operation::GetSendOperationError::InvalidRequest) => {
            return Err(SendCommandErrorDtoV1::InvalidRequest);
        }
        Err(_) => {
            return Ok(SendCommandOutcomeDtoV1::OutcomeUnknown { operation_id });
        }
    };
    if !replaying_existing {
        match journal
            .record_attempt_scoped(
                principal,
                crate::domain::local_event::OperationKind::Send,
                &operation_id,
                canonical_payload.as_bytes(),
                Some(attempt_scope),
            )
            .await
        {
            Ok(_) => {}
            Err(crate::usecase::agent_session::operation::CallerJournalError::OutcomeUnknown) => {
                return Ok(SendCommandOutcomeDtoV1::OutcomeUnknown { operation_id });
            }
            Err(
                crate::usecase::agent_session::operation::CallerJournalError::RejectedBeforeCommit,
            ) => {
                return Ok(SendCommandOutcomeDtoV1::RejectedBeforeCommit {
                    failure: caller_journal_failure("The local caller attempt could not be saved.")
                        .into(),
                });
            }
            Err(error) => return Err(send_command_common(caller_journal_application_error(error))),
        }
    }
    let outcome = send_operation
        .send(
            crate::usecase::agent_session::operation::SendOperationRequest {
                principal: principal.to_string(),
                operation_id: operation_id.clone(),
                canonical_payload: canonical_payload.clone(),
            },
        )
        .await
        .map_err(|error| send_command_common(send_operation_error(error)))?;
    match outcome {
        crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(operation) => {
            if let Err(error) = journal
                .resolve_attempt_if_present(
                    principal,
                    crate::domain::local_event::OperationKind::Send,
                    &operation_id,
                    canonical_payload.as_bytes(),
                    true,
                )
                .await
            {
                // The accepted operation remains authoritative and the same
                // journal identity stays discoverable for this exact retry.
                log::warn!("caller send journal clear requires reconciliation: {error:?}");
            }
            Ok(SendCommandOutcomeDtoV1::Accepted {
                operation: operation.into(),
            })
        }
        crate::usecase::agent_session::operation::SendCommandOutcome::RejectedBeforeCommit {
            failure,
        } => {
            if let Err(error) = journal
                .resolve_attempt_if_present(
                    principal,
                    crate::domain::local_event::OperationKind::Send,
                    &operation_id,
                    canonical_payload.as_bytes(),
                    false,
                )
                .await
            {
                log::warn!("rejected caller send journal clear requires reconciliation: {error:?}");
            }
            Ok(SendCommandOutcomeDtoV1::RejectedBeforeCommit {
                failure: failure.into(),
            })
        }
        crate::usecase::agent_session::operation::SendCommandOutcome::OutcomeUnknown { .. } => {
            Ok(SendCommandOutcomeDtoV1::OutcomeUnknown { operation_id })
        }
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message(
    _app: tauri::AppHandle,
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    send_operation: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    >,
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    operation_id: String,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<crate::usecase::agent_session::session::ImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
    editor_context: Option<crate::usecase::agent_session::runtime::usecase::AgentEditorContext>,
) -> Result<SendCommandOutcomeDtoV1, SendCommandErrorDtoV1> {
    let permission_mode = validate_invoke_permission_mode(permission_mode)
        .map_err(|_| SendCommandErrorDtoV1::InvalidRequest)?;
    dispatch_durable_send(
        store.inner().as_ref(),
        send_operation.inner().as_ref(),
        journal.inner().as_ref(),
        operation_id,
        CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id,
                worktree_path,
            },
            content,
            permission_mode: permission_mode.as_str().to_string(),
            plan_mode: plan_mode.unwrap_or(false),
            backend_id,
            model_id,
            images: images.unwrap_or_default(),
            mentions: mentions.unwrap_or_default(),
            editor_context,
        },
    )
    .await
}

#[tauri::command]
pub async fn get_agent_send_operation(
    usecase: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    >,
    operation_id: String,
) -> Result<SendOperationViewDtoV1, SendLookupErrorDtoV1> {
    get_agent_send_operation_for_principal(
        usecase.inner().as_ref(),
        TAURI_OPERATION_PRINCIPAL,
        operation_id,
    )
    .await
}

pub(crate) async fn get_agent_send_operation_for_principal(
    usecase: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    principal: &str,
    operation_id: String,
) -> Result<SendOperationViewDtoV1, SendLookupErrorDtoV1> {
    use crate::usecase::agent_session::operation::GetSendOperationError as E;
    usecase
        .get_operation(principal, &operation_id)
        .await
        .map(Into::into)
        .map_err(|error| match error {
            E::InvalidRequest => SendLookupErrorDtoV1::InvalidRequest,
            E::OutcomeUnknown { operation_id } => {
                SendLookupErrorDtoV1::OutcomeUnknown { operation_id }
            }
            E::NotFound => SendLookupErrorDtoV1::NotFound,
            E::Internal { correlation_id } => SendLookupErrorDtoV1::Internal { correlation_id },
            E::QueryBusy => SendLookupErrorDtoV1::QueryBusy,
            E::DeadlineExceeded => SendLookupErrorDtoV1::DeadlineExceeded,
            E::StorageUnavailable { failure } => SendLookupErrorDtoV1::StorageUnavailable {
                failure: failure.into(),
            },
        })
}
