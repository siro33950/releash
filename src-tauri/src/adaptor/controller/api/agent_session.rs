//! Loopback WebSocket V1 adapter for durable agent-session operations.
//!
//! The socket is transport-only: it owns authentication/resource bounds and
//! delegates send/Stop/recovery semantics to the same usecases used by Tauri.
//! Session lifecycle is deliberately absent from this closed route set
//! (R-014).

use std::time::Instant;
use std::{collections::HashSet, sync::Arc};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::adaptor::controller::agent_session_operation_wiring::{
    CanonicalSendCommandV1, LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
};
use crate::adaptor::protocol::agent_session_notice::{
    SessionFeedbackPageMessage, SessionFeedbackRetryOutcomeMessage,
};
use crate::adaptor::protocol::agent_session_v1::{
    checked_pending_recovery_page, decode_nonnegative_i64_decimal, decode_nonnegative_u64_decimal,
    decode_positive_i64_decimal, GetSessionResponseDtoV1, OperationApplicationErrorDtoV1,
    PendingPartitionDtoV1, PendingRecoveryPageDtoV1, PermissionResponseCommandOutcomeDtoV1,
    PermissionResponseOperationViewDtoV1, PermissionResponseRequestDtoV1,
    RecoveryActionOutcomeDtoV1, RecoveryActionRequestDtoV1, RecoveryActionStatusDtoV1,
    SendCommandErrorDtoV1, SendCommandOutcomeDtoV1, SendOperationViewDtoV1,
    StopCommandOutcomeDtoV1, StopOperationReceiptDtoV1, StopOperationRequestDtoV1,
    StopOperationStateDtoV1,
};
use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationQuitLookupDtoV1, ApplicationQuitOutcomeDtoV1, ApplicationQuitRequestDtoV1,
    CurrentShutdownResultDtoV1, ShutdownPlanPageDtoV1, ShutdownTargetActionRequestDtoV1,
};
use crate::usecase::agent_session::operation::{
    GetSendOperationError, StopOperationError, StopOperationRequest,
};
#[cfg(test)]
use crate::usecase::agent_session::operation::{
    SendAgentMessageError, SendCommandOutcome, SendOperationRequest,
};

use super::{AgentSessionApiDeps, LocalApiState};

const LOCAL_API_OPERATION_PRINCIPAL: &str = LOCAL_INSTALLATION_OPERATION_PRINCIPAL;
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_INFLIGHT_REQUESTS: usize = 32;
const MAX_OUTBOUND_RESPONSES: usize = 32;
const CLOSE_MESSAGE_TOO_LARGE: u16 = 1009;
const CLOSE_TRY_AGAIN_LATER: u16 = 1013;
const RATE_PER_SECOND: f64 = 60.0;
const RATE_BURST: f64 = 120.0;

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

fn send_or_stop_admission_error(
    context: &str,
    error: OperationApplicationErrorDtoV1,
) -> OperationApplicationErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::ShutdownInProgress
        | OperationApplicationErrorDtoV1::Internal { .. } => error,
        other => OperationApplicationErrorDtoV1::Internal {
            correlation_id: common_error_correlation(context, &other),
        },
    }
}

fn recovery_admission_error(
    error: OperationApplicationErrorDtoV1,
) -> OperationApplicationErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::ShutdownInProgress
        | OperationApplicationErrorDtoV1::StorageUnavailable { .. }
        | OperationApplicationErrorDtoV1::Internal { .. } => error,
        other => OperationApplicationErrorDtoV1::Internal {
            correlation_id: common_error_correlation("websocket_recovery_admission", &other),
        },
    }
}

#[derive(Debug)]
struct ExplicitOption<T>(Option<T>);

impl<'de, T> Deserialize<'de> for ExplicitOption<T>
where
    T: serde::de::DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_null() {
            return Ok(Self(None));
        }
        T::deserialize(value)
            .map(Some)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum AgentSessionWsRequestV1 {
    GetSession {
        id: String,
        session_id: String,
        attempt_id: String,
    },
    RequestSend {
        id: String,
        operation_id: String,
        command: CanonicalSendCommandV1,
    },
    GetSend {
        id: String,
        operation_id: String,
    },
    RequestPermissionResponse {
        id: String,
        request: PermissionResponseRequestDtoV1,
    },
    GetPermissionResponse {
        id: String,
        operation_id: String,
    },
    RequestStop {
        id: String,
        request: StopOperationRequestDtoV1,
    },
    GetStop {
        id: String,
        operation_id: String,
    },
    GetPendingRecovery {
        id: String,
        limit: ExplicitOption<usize>,
        partition: ExplicitOption<PendingPartitionDtoV1>,
        owner: ExplicitOption<String>,
        shutdown_id: ExplicitOption<String>,
        cursor: ExplicitOption<String>,
    },
    GetPendingRecoverySnapshot {
        id: String,
        shutdown_id: String,
        snapshot_id: String,
        partition: PendingPartitionDtoV1,
        limit: ExplicitOption<usize>,
        cursor: ExplicitOption<String>,
    },
    RequestRecoveryAction {
        id: String,
        request: RecoveryActionRequestDtoV1,
    },
    GetRecoveryAction {
        id: String,
        action_id: String,
    },
    RequestApplicationQuit {
        id: String,
        request: ApplicationQuitRequestDtoV1,
    },
    GetApplicationQuit {
        id: String,
        operation_id: String,
    },
    GetCurrentShutdown {
        id: String,
    },
    GetShutdownPlan {
        id: String,
        shutdown_id: String,
        limit: ExplicitOption<usize>,
        cursor: ExplicitOption<String>,
    },
    ResolveShutdownTarget {
        id: String,
        request: ShutdownTargetActionRequestDtoV1,
    },
    ListFeedback {
        id: String,
        session_id: String,
        limit: ExplicitOption<usize>,
        cursor: ExplicitOption<String>,
    },
    DismissFeedback {
        id: String,
        session_id: String,
        feedback_id: String,
        expected_revision: String,
        action_id: String,
    },
    RetryFeedback {
        id: String,
        session_id: String,
        feedback_id: String,
        expected_revision: String,
        action_id: String,
    },
}

impl AgentSessionWsRequestV1 {
    fn id(&self) -> &str {
        match self {
            Self::GetSession { id, .. }
            | Self::RequestSend { id, .. }
            | Self::GetSend { id, .. }
            | Self::RequestPermissionResponse { id, .. }
            | Self::GetPermissionResponse { id, .. }
            | Self::RequestStop { id, .. }
            | Self::GetStop { id, .. }
            | Self::GetPendingRecovery { id, .. }
            | Self::GetPendingRecoverySnapshot { id, .. }
            | Self::RequestRecoveryAction { id, .. }
            | Self::GetRecoveryAction { id, .. }
            | Self::RequestApplicationQuit { id, .. }
            | Self::GetApplicationQuit { id, .. }
            | Self::GetCurrentShutdown { id }
            | Self::GetShutdownPlan { id, .. }
            | Self::ResolveShutdownTarget { id, .. }
            | Self::ListFeedback { id, .. }
            | Self::DismissFeedback { id, .. }
            | Self::RetryFeedback { id, .. } => id,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentSessionWsResultV1 {
    Session {
        session: Box<Option<GetSessionResponseDtoV1>>,
    },
    SendOperation {
        operation: SendOperationViewDtoV1,
    },
    SendOutcome {
        outcome: SendCommandOutcomeDtoV1,
    },
    PermissionResponseOutcome {
        outcome: PermissionResponseCommandOutcomeDtoV1,
    },
    PermissionResponseOperation {
        operation: PermissionResponseOperationViewDtoV1,
    },
    StopOutcome {
        outcome: StopCommandOutcomeDtoV1,
    },
    StopOperation {
        receipt: StopOperationReceiptDtoV1,
        state: StopOperationStateDtoV1,
    },
    PendingRecovery {
        page: PendingRecoveryPageDtoV1,
    },
    RecoveryActionOutcome {
        outcome: RecoveryActionOutcomeDtoV1,
    },
    RecoveryAction {
        status: RecoveryActionStatusDtoV1,
    },
    ApplicationQuitOutcome {
        outcome: ApplicationQuitOutcomeDtoV1,
    },
    ApplicationQuit {
        lookup: ApplicationQuitLookupDtoV1,
    },
    CurrentShutdown {
        result: CurrentShutdownResultDtoV1,
    },
    ShutdownPlan {
        page: Box<ShutdownPlanPageDtoV1>,
    },
    FeedbackPage {
        page: SessionFeedbackPageMessage,
    },
    FeedbackDismissed {
        feedback_id: String,
    },
    FeedbackRetried {
        outcome: SessionFeedbackRetryOutcomeMessage,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AgentSessionWsResponseV1 {
    Ok {
        id: String,
        result: Box<AgentSessionWsResultV1>,
    },
    Error {
        id: String,
        error: OperationApplicationErrorDtoV1,
    },
}

struct RateBudget {
    tokens: f64,
    updated_at: Instant,
    refill_per_second: f64,
}

impl RateBudget {
    fn new(refill_per_second: f64) -> Self {
        Self {
            tokens: RATE_BURST,
            updated_at: Instant::now(),
            refill_per_second,
        }
    }

    fn acquire(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens
            + now.duration_since(self.updated_at).as_secs_f64() * self.refill_per_second)
            .min(RATE_BURST);
        self.updated_at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

pub(super) fn router() -> Router<LocalApiState> {
    Router::new().route("/v1/agent-session", get(upgrade))
}

async fn upgrade(State(state): State<LocalApiState>, ws: WebSocketUpgrade) -> Response {
    let Some(deps) = state.agent_session else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Ok(permit) = deps.connection_limit.clone().try_acquire_owned() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "error",
                "error": { "type": "capacity_exceeded" }
            })),
        )
            .into_response();
    };
    let dispatcher: Arc<dyn WsDispatchService> = Arc::new(deps);
    // Admit the first byte beyond the public limit so the application can
    // return the protocol-mandated 1009 close frame instead of letting the
    // transport tear down the TCP stream before the peer receives it.
    ws.max_message_size(MAX_MESSAGE_BYTES + 1)
        .max_frame_size(MAX_MESSAGE_BYTES + 1)
        .on_upgrade(move |socket| serve(socket, dispatcher, permit))
}

#[async_trait::async_trait]
trait WsDispatchService: Send + Sync {
    async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1;
}

#[async_trait::async_trait]
impl WsDispatchService for AgentSessionApiDeps {
    async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
        dispatch(self, request).await
    }
}

async fn serve(
    socket: WebSocket,
    dispatcher: Arc<dyn WsDispatchService>,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    serve_with_outbound_budget(
        socket,
        dispatcher,
        permit,
        MAX_MESSAGE_BYTES,
        RATE_PER_SECOND,
        None,
    )
    .await;
}

async fn serve_with_outbound_budget(
    socket: WebSocket,
    dispatcher: Arc<dyn WsDispatchService>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    outbound_byte_limit: usize,
    rate_per_second: f64,
    writer_gate: Option<(Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Notify>)>,
) {
    let (mut sink, mut stream) = socket.split();
    let (outbound, mut outbound_rx) =
        tokio::sync::mpsc::channel::<OutboundResponse>(MAX_OUTBOUND_RESPONSES);
    let (close, mut close_rx) = tokio::sync::mpsc::unbounded_channel::<(u16, &'static str)>();
    let outbound_bytes = Arc::new(tokio::sync::Semaphore::new(outbound_byte_limit));
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some((code, reason)) = close_rx.recv() => {
                    let _ = sink.send(Message::Close(Some(CloseFrame {
                        code,
                        reason: reason.into(),
                    }))).await;
                    break;
                }
                Some(response) = outbound_rx.recv() => {
                    if let Some((gate, entered)) = &writer_gate {
                        entered.notify_one();
                        if gate.acquire().await.is_err() {
                            break;
                        }
                    }
                    if sink.send(Message::Text(response.encoded.clone().into())).await.is_err() {
                        break;
                    }
                    // `response` owns the outer-id reservation. It is released
                    // only after the frame has been accepted by the socket
                    // sink, never merely because dispatch completed.
                }
                else => break,
            }
        }
    });
    let inflight_limit = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_REQUESTS));
    let inflight_ids = Arc::new(std::sync::Mutex::new(HashSet::<String>::new()));
    let mut dispatch_tasks = tokio::task::JoinSet::new();
    let mut rate = RateBudget::new(rate_per_second);
    while let Some(message) = stream.next().await {
        let Ok(message) = message else {
            break;
        };
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        if text.len() > MAX_MESSAGE_BYTES {
            let _ = close.send((CLOSE_MESSAGE_TOO_LARGE, "request too large"));
            break;
        }
        let parsed = serde_json::from_str::<AgentSessionWsRequestV1>(text.as_str());
        let request = match parsed {
            Ok(request) if valid_outer_id(request.id()) => request,
            Ok(request) => {
                enqueue_ws_response(&outbound, &outbound_bytes, &close, invalid(request.id()));
                continue;
            }
            Err(_) => {
                enqueue_ws_response(&outbound, &outbound_bytes, &close, invalid("invalid"));
                continue;
            }
        };
        let id = request.id().to_string();
        if !rate.acquire() {
            enqueue_ws_response(
                &outbound,
                &outbound_bytes,
                &close,
                AgentSessionWsResponseV1::Error {
                    id,
                    error: OperationApplicationErrorDtoV1::RateLimited,
                },
            );
            continue;
        }
        {
            let mut ids = inflight_ids
                .lock()
                .expect("outer request id mutex poisoned");
            if !ids.insert(id.clone()) {
                drop(ids);
                enqueue_ws_response(
                    &outbound,
                    &outbound_bytes,
                    &close,
                    AgentSessionWsResponseV1::Error {
                        id,
                        error: OperationApplicationErrorDtoV1::RequestIdConflict,
                    },
                );
                continue;
            }
        }
        let permit = match inflight_limit.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                inflight_ids
                    .lock()
                    .expect("outer request id mutex poisoned")
                    .remove(&id);
                enqueue_ws_response(
                    &outbound,
                    &outbound_bytes,
                    &close,
                    AgentSessionWsResponseV1::Error {
                        id,
                        error: OperationApplicationErrorDtoV1::CapacityExceeded,
                    },
                );
                // Give the dedicated writer a chance to move bounded
                // rejection frames into the socket before admitting the next
                // frame. This keeps the in-flight limit independent from the
                // outbound-backpressure limit for a healthy peer.
                tokio::task::yield_now().await;
                continue;
            }
        };
        let dispatcher = Arc::clone(&dispatcher);
        let outbound = outbound.clone();
        let outbound_bytes = Arc::clone(&outbound_bytes);
        let close = close.clone();
        let inflight_ids = Arc::clone(&inflight_ids);
        let reservation = OuterRequestIdReservation { id, inflight_ids };
        dispatch_tasks.spawn(async move {
            let response = dispatcher.dispatch(request).await;
            drop(permit);
            enqueue_ws_response_reserved(&outbound, &outbound_bytes, &close, response, reservation);
        });
    }
    // A connection owns every outer-id reservation and dispatch task. Abort
    // unfinished work on disconnect so the connection permit, queue senders,
    // and reservations cannot outlive the socket indefinitely. Any operation
    // already accepted is durably replayable under its inner identity.
    dispatch_tasks.abort_all();
    while dispatch_tasks.join_next().await.is_some() {}
    drop(outbound);
    drop(close);
    let _ = writer.await;
}

struct OuterRequestIdReservation {
    id: String,
    inflight_ids: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl Drop for OuterRequestIdReservation {
    fn drop(&mut self) {
        self.inflight_ids
            .lock()
            .expect("outer request id mutex poisoned")
            .remove(&self.id);
    }
}

struct OutboundResponse {
    encoded: String,
    _bytes: tokio::sync::OwnedSemaphorePermit,
    _outer_id: Option<OuterRequestIdReservation>,
}

fn enqueue_ws_response(
    outbound: &tokio::sync::mpsc::Sender<OutboundResponse>,
    outbound_bytes: &Arc<tokio::sync::Semaphore>,
    close: &tokio::sync::mpsc::UnboundedSender<(u16, &'static str)>,
    response: AgentSessionWsResponseV1,
) {
    enqueue_ws_response_inner(outbound, outbound_bytes, close, response, None);
}

fn enqueue_ws_response_reserved(
    outbound: &tokio::sync::mpsc::Sender<OutboundResponse>,
    outbound_bytes: &Arc<tokio::sync::Semaphore>,
    close: &tokio::sync::mpsc::UnboundedSender<(u16, &'static str)>,
    response: AgentSessionWsResponseV1,
    reservation: OuterRequestIdReservation,
) {
    enqueue_ws_response_inner(outbound, outbound_bytes, close, response, Some(reservation));
}

fn enqueue_ws_response_inner(
    outbound: &tokio::sync::mpsc::Sender<OutboundResponse>,
    outbound_bytes: &Arc<tokio::sync::Semaphore>,
    close: &tokio::sync::mpsc::UnboundedSender<(u16, &'static str)>,
    response: AgentSessionWsResponseV1,
    reservation: Option<OuterRequestIdReservation>,
) {
    let id = match &response {
        AgentSessionWsResponseV1::Ok { id, .. } | AgentSessionWsResponseV1::Error { id, .. } => {
            id.clone()
        }
    };
    let mut encoded = match serde_json::to_string(&response) {
        Ok(encoded) => encoded,
        Err(_) => {
            let _ = close.send((1011, "response encoding failed"));
            return;
        }
    };
    if encoded.len() > MAX_MESSAGE_BYTES {
        encoded = serde_json::to_string(&AgentSessionWsResponseV1::Error {
            id,
            error: OperationApplicationErrorDtoV1::ResponseTooLarge,
        })
        .expect("bounded response-too-large error");
    }
    let permits = match u32::try_from(encoded.len()) {
        Ok(permits) => permits,
        Err(_) => {
            let _ = close.send((CLOSE_TRY_AGAIN_LATER, "outbound backpressure"));
            return;
        }
    };
    let permit = match outbound_bytes.clone().try_acquire_many_owned(permits) {
        Ok(permit) => permit,
        Err(_) => {
            let _ = close.send((CLOSE_TRY_AGAIN_LATER, "outbound backpressure"));
            return;
        }
    };
    if outbound
        .try_send(OutboundResponse {
            encoded,
            _bytes: permit,
            _outer_id: reservation,
        })
        .is_err()
    {
        let _ = close.send((CLOSE_TRY_AGAIN_LATER, "outbound backpressure"));
    }
}

fn valid_outer_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn invalid(id: &str) -> AgentSessionWsResponseV1 {
    AgentSessionWsResponseV1::Error {
        id: id.to_string(),
        error: OperationApplicationErrorDtoV1::InvalidRequest,
    }
}

fn stop_command_error(error: StopOperationError) -> OperationApplicationErrorDtoV1 {
    match error {
        StopOperationError::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        StopOperationError::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        StopOperationError::ShutdownInProgress => {
            OperationApplicationErrorDtoV1::ShutdownInProgress
        }
        StopOperationError::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_stop_command",
            &format!("{other:?}"),
        ),
    }
}

fn stop_lookup_error(error: StopOperationError) -> OperationApplicationErrorDtoV1 {
    match error {
        StopOperationError::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        StopOperationError::NotFound => OperationApplicationErrorDtoV1::NotFound,
        StopOperationError::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        StopOperationError::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        StopOperationError::StorageUnavailable { failure } => {
            OperationApplicationErrorDtoV1::StorageUnavailable {
                failure: failure.into(),
            }
        }
        StopOperationError::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_stop_lookup",
            &format!("{other:?}"),
        ),
    }
}

fn send_query_error(error: GetSendOperationError) -> OperationApplicationErrorDtoV1 {
    match error {
        GetSendOperationError::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        GetSendOperationError::OutcomeUnknown { operation_id } => {
            OperationApplicationErrorDtoV1::OutcomeUnknown { operation_id }
        }
        GetSendOperationError::NotFound => OperationApplicationErrorDtoV1::NotFound,
        GetSendOperationError::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        GetSendOperationError::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        GetSendOperationError::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        GetSendOperationError::StorageUnavailable { failure } => {
            OperationApplicationErrorDtoV1::StorageUnavailable {
                failure: failure.into(),
            }
        }
    }
}

fn pending_recovery_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest | E::SnapshotMismatch => OperationApplicationErrorDtoV1::InvalidRequest,
        E::CursorMismatch => OperationApplicationErrorDtoV1::CursorMismatch,
        E::CursorExpired => OperationApplicationErrorDtoV1::CursorExpired,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => OperationApplicationErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_pending_recovery",
            &format!("{other:?}"),
        ),
    }
}

fn pending_recovery_snapshot_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::SnapshotMismatch => OperationApplicationErrorDtoV1::SnapshotMismatch,
        E::CursorMismatch => OperationApplicationErrorDtoV1::CursorMismatch,
        E::CursorExpired => OperationApplicationErrorDtoV1::CursorExpired,
        E::DetailsCompacted => OperationApplicationErrorDtoV1::DetailsCompacted,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => OperationApplicationErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
    }
}

fn recovery_command_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_recovery_command",
            &format!("{other:?}"),
        ),
    }
}

fn recovery_lookup_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::NotFound => OperationApplicationErrorDtoV1::NotFound,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_recovery_lookup",
            &format!("{other:?}"),
        ),
    }
}

fn feedback_query_error(
    error: crate::usecase::agent_session::feedback::FeedbackError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::feedback::FeedbackError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "websocket_feedback_query",
                "read-only feedback query returned a mutation-admission error",
            ),
        },
        E::CapacityExceeded => OperationApplicationErrorDtoV1::FeedbackCapacityExceeded,
        E::CursorMismatch => OperationApplicationErrorDtoV1::CursorMismatch,
        E::CursorExpired => OperationApplicationErrorDtoV1::CursorExpired,
        E::QueryBusy => OperationApplicationErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => OperationApplicationErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => OperationApplicationErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_feedback_query",
            &format!("{other:?}"),
        ),
    }
}

fn feedback_mutation_error(
    error: crate::usecase::agent_session::feedback::FeedbackError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::feedback::FeedbackError as E;
    match error {
        E::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => OperationApplicationErrorDtoV1::ShutdownInProgress,
        E::CapacityExceeded => OperationApplicationErrorDtoV1::FeedbackCapacityExceeded,
        E::RevisionConflict { current_revision } => {
            OperationApplicationErrorDtoV1::RevisionConflict {
                current_revision: current_revision.to_string(),
            }
        }
        E::OutcomeUnknown { feedback_id } => OperationApplicationErrorDtoV1::OutcomeUnknown {
            operation_id: feedback_id,
        },
        E::StorageUnavailable { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
        other => crate::adaptor::presenter::application_lifecycle::presentation_error(
            "websocket_feedback_mutation",
            &format!("{other:?}"),
        ),
    }
}

fn session_feedback_load_error(
    error: crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadError,
) -> OperationApplicationErrorDtoV1 {
    use crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadError as E;
    match error {
        E::Feedback(error) => feedback_mutation_error(error),
        E::LoadFailed { failure } => OperationApplicationErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
    }
}

async fn ensure_mutation_admission(
    deps: &AgentSessionApiDeps,
) -> Result<(), OperationApplicationErrorDtoV1> {
    match deps
        .shutdown
        .current_shutdown()
        .await
        .map_err(crate::adaptor::presenter::application_lifecycle::query_error)?
    {
        Some(plan)
            if matches!(
                plan.phase,
                crate::domain::local_event::ApplicationShutdownPhase::Failed
                    | crate::domain::local_event::ApplicationShutdownPhase::Cancelled
                    | crate::domain::local_event::ApplicationShutdownPhase::Completed
            ) =>
        {
            Ok(())
        }
        Some(_) => Err(OperationApplicationErrorDtoV1::ShutdownInProgress),
        None => Ok(()),
    }
}

async fn dispatch(
    deps: &AgentSessionApiDeps,
    request: AgentSessionWsRequestV1,
) -> AgentSessionWsResponseV1 {
    let id = request.id().to_string();
    let result = match request {
        AgentSessionWsRequestV1::GetSession {
            session_id,
            attempt_id,
            ..
        } => {
            get_feedback_supervised_session(deps.feedback_load.as_ref(), &session_id, &attempt_id)
                .await
        }
        AgentSessionWsRequestV1::RequestSend {
            operation_id,
            command,
            ..
        } => request_send(deps, operation_id, command).await,
        AgentSessionWsRequestV1::GetSend { operation_id, .. } => {
            get_send(&deps.send, operation_id).await
        }
        AgentSessionWsRequestV1::RequestPermissionResponse { request, .. } => {
            request_permission_response(deps, request).await
        }
        AgentSessionWsRequestV1::GetPermissionResponse { operation_id, .. } => deps
            .permission_response
            .get_operation(LOCAL_API_OPERATION_PRINCIPAL, &operation_id)
            .await
            .map(crate::adaptor::presenter::agent_session::permission_response_operation)
            .map(|operation| AgentSessionWsResultV1::PermissionResponseOperation { operation })
            .map_err(|error| {
                OperationApplicationErrorDtoV1::from(
                    crate::adaptor::presenter::agent_session::permission_response_lookup_error(
                        error,
                    ),
                )
            }),
        AgentSessionWsRequestV1::RequestStop { request, .. } => request_stop(deps, request).await,
        AgentSessionWsRequestV1::GetStop { operation_id, .. } => deps
            .stop
            .get_operation(LOCAL_API_OPERATION_PRINCIPAL, &operation_id)
            .await
            .map(|(receipt, state)| AgentSessionWsResultV1::StopOperation {
                receipt: receipt.into(),
                state: state.into(),
            })
            .map_err(stop_lookup_error),
        AgentSessionWsRequestV1::GetPendingRecovery {
            limit,
            partition,
            owner,
            shutdown_id,
            cursor,
            ..
        } => list_pending_recovery(
            &deps.recovery,
            limit.0,
            partition.0,
            owner.0,
            shutdown_id.0,
            cursor.0,
        )
        .await,
        AgentSessionWsRequestV1::GetPendingRecoverySnapshot {
            shutdown_id,
            snapshot_id,
            partition,
            limit,
            cursor,
            ..
        } => {
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
                    return AgentSessionWsResponseV1::Error {
                        id,
                        error: OperationApplicationErrorDtoV1::InvalidRequest,
                    }
                }
            };
            deps.recovery
                .pending_snapshot(
                    crate::usecase::agent_session::operation::PendingRecoverySnapshotQuery {
                        plan: crate::domain::local_event::ShutdownPlanKey { shutdown_id },
                        snapshot_id,
                        partition,
                        limit: limit.0.unwrap_or(32),
                        cursor: cursor.0,
                    },
                )
                .await
                .and_then(checked_pending_recovery_page)
                .map(|page| AgentSessionWsResultV1::PendingRecovery { page })
                .map_err(pending_recovery_snapshot_error)
        }
        AgentSessionWsRequestV1::RequestRecoveryAction { request, .. } => {
            request_recovery_action(deps, request).await
        }
        AgentSessionWsRequestV1::GetRecoveryAction { action_id, .. } => {
            get_recovery_action(deps, action_id).await
        }
        AgentSessionWsRequestV1::RequestApplicationQuit { request, .. } => {
            request_application_quit(deps, request).await
        }
        AgentSessionWsRequestV1::GetApplicationQuit { operation_id, .. } => {
            crate::adaptor::controller::command::application_lifecycle::get_application_quit_operation_result(
                deps.shutdown.as_ref(),
                operation_id,
            )
            .await
            .map(|lookup| AgentSessionWsResultV1::ApplicationQuit { lookup })
            .map_err(Into::into)
        }
        AgentSessionWsRequestV1::GetCurrentShutdown { .. } => {
            crate::adaptor::controller::command::application_lifecycle::get_application_shutdown_result(
                deps.shutdown.as_ref(),
            )
            .await
            .map(|result| AgentSessionWsResultV1::CurrentShutdown { result })
            .map_err(Into::into)
        }
        AgentSessionWsRequestV1::GetShutdownPlan {
            shutdown_id,
            limit,
            cursor,
            ..
        } => get_shutdown_plan(deps, shutdown_id, limit.0, cursor.0).await,
        AgentSessionWsRequestV1::ResolveShutdownTarget { request, .. } => {
            resolve_shutdown_target(deps, request).await
        }
        AgentSessionWsRequestV1::ListFeedback {
            session_id,
            limit,
            cursor,
            ..
        } => list_feedback(&deps.feedback, session_id, limit.0, cursor.0).await,
        AgentSessionWsRequestV1::DismissFeedback {
            session_id,
            feedback_id,
            expected_revision,
            action_id,
            ..
        } => {
            dismiss_feedback(
                &deps.feedback,
                session_id,
                feedback_id,
                expected_revision,
                action_id,
            )
            .await
        }
        AgentSessionWsRequestV1::RetryFeedback {
            session_id,
            feedback_id,
            expected_revision,
            action_id,
            ..
        } => {
            retry_feedback(
                &deps.feedback,
                session_id,
                feedback_id,
                expected_revision,
                action_id,
            )
            .await
        }
    };
    match result {
        Ok(result) => AgentSessionWsResponseV1::Ok {
            id,
            result: Box::new(result),
        },
        Err(error) => AgentSessionWsResponseV1::Error { id, error },
    }
}

async fn get_feedback_supervised_session(
    usecase: &crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase,
    session_id: &str,
    attempt_id: &str,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    usecase
        .get_session(session_id, attempt_id)
        .await
        .map(|session| AgentSessionWsResultV1::Session {
            session: Box::new(session.map(Into::into)),
        })
        .map_err(session_feedback_load_error)
}

async fn list_pending_recovery(
    recovery: &crate::usecase::agent_session::operation::RecoveryActionUsecase,
    limit: Option<usize>,
    partition: Option<PendingPartitionDtoV1>,
    owner: Option<String>,
    shutdown_id: Option<String>,
    cursor: Option<String>,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
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
    let shutdown_plan = match shutdown_id {
        Some(shutdown_id) if !shutdown_id.is_empty() => {
            Some(crate::domain::local_event::ShutdownPlanKey { shutdown_id })
        }
        None => None,
        Some(_) => return Err(OperationApplicationErrorDtoV1::InvalidRequest),
    };
    recovery
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
        .map(|page| AgentSessionWsResultV1::PendingRecovery { page })
        .map_err(pending_recovery_error)
}

fn parse_decimal_revision(raw: &str) -> Option<u64> {
    decode_nonnegative_u64_decimal(raw)
}

async fn list_feedback(
    feedback: &crate::usecase::agent_session::feedback::SessionFeedbackUsecase,
    session_id: String,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    feedback
        .list(&session_id, limit.unwrap_or(32), cursor)
        .await
        .map(|page| AgentSessionWsResultV1::FeedbackPage { page: page.into() })
        .map_err(feedback_query_error)
}

async fn dismiss_feedback(
    feedback: &crate::usecase::agent_session::feedback::SessionFeedbackUsecase,
    session_id: String,
    feedback_id: String,
    expected_revision: String,
    action_id: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let revision = parse_decimal_revision(&expected_revision)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    feedback
        .dismiss(&session_id, &feedback_id, revision, &action_id)
        .await
        .map(|()| AgentSessionWsResultV1::FeedbackDismissed { feedback_id })
        .map_err(feedback_mutation_error)
}

async fn retry_feedback(
    feedback: &crate::usecase::agent_session::feedback::SessionFeedbackUsecase,
    session_id: String,
    feedback_id: String,
    expected_revision: String,
    action_id: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let revision = parse_decimal_revision(&expected_revision)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    feedback
        .retry_resolution(&session_id, &feedback_id, revision, &action_id)
        .await
        .map(|outcome| AgentSessionWsResultV1::FeedbackRetried {
            outcome: outcome.into(),
        })
        .map_err(feedback_mutation_error)
}

async fn request_recovery_action(
    deps: &AgentSessionApiDeps,
    request: RecoveryActionRequestDtoV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let origin_revision = decode_nonnegative_u64_decimal(&request.origin_revision)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    match deps.recovery.get_action(&request.action_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            ensure_mutation_admission(deps)
                .await
                .map_err(recovery_admission_error)?;
        }
        Err(crate::usecase::agent_session::operation::RecoveryActionError::InvalidRequest) => {
            return Err(OperationApplicationErrorDtoV1::InvalidRequest);
        }
        Err(_) => {
            return Ok(AgentSessionWsResultV1::RecoveryActionOutcome {
                outcome: RecoveryActionOutcomeDtoV1::ActionOutcomeUnknown {
                    action_id: request.action_id,
                },
            });
        }
    }
    execute_recovery_action(&deps.recovery, request, origin_revision)
        .await
        .map(|outcome| AgentSessionWsResultV1::RecoveryActionOutcome { outcome })
}

async fn execute_recovery_action(
    recovery: &crate::usecase::agent_session::operation::RecoveryActionUsecase,
    request: RecoveryActionRequestDtoV1,
    origin_revision: u64,
) -> Result<RecoveryActionOutcomeDtoV1, OperationApplicationErrorDtoV1> {
    let outcome = recovery
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
    let outcome =
        if let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
            ref action_id,
            ..
        } = outcome
        {
            let status = recovery
                .get_action_status(action_id)
                .await
                .map_err(recovery_command_error)?;
            RecoveryActionOutcomeDtoV1::from_durable_status(status)
        } else {
            outcome.into()
        };
    Ok(outcome)
}

async fn get_recovery_action(
    deps: &AgentSessionApiDeps,
    action_id: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let status = deps
        .recovery
        .get_action_status(&action_id)
        .await
        .map_err(recovery_lookup_error)?;
    Ok(AgentSessionWsResultV1::RecoveryAction {
        status: status.into(),
    })
}

async fn request_application_quit(
    deps: &AgentSessionApiDeps,
    request: ApplicationQuitRequestDtoV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let (outcome, process_action) =
        crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            deps.shutdown.as_ref(),
            request,
        )
        .await
        .map_err(OperationApplicationErrorDtoV1::from)?;
    if let Some(process_action) = process_action {
        deps.process_actions
            .dispatch_tauri(deps.app.clone(), process_action);
    }
    Ok(AgentSessionWsResultV1::ApplicationQuitOutcome { outcome })
}

async fn resolve_shutdown_target(
    deps: &AgentSessionApiDeps,
    request: ShutdownTargetActionRequestDtoV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let ordinal = decode_nonnegative_i64_decimal(&request.ordinal)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    let origin_revision = decode_nonnegative_u64_decimal(&request.origin_revision)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    let execution = deps
        .shutdown
        .resolve_shutdown_target_action(
            crate::usecase::shutdown_coordinator::ShutdownTargetActionRequest {
                action_id: request.action_id,
                plan: crate::domain::local_event::ShutdownPlanKey {
                    shutdown_id: request.shutdown_id,
                },
                ordinal,
                target_key: request.target_key,
                origin_revision,
                action: request.action.into(),
            },
        )
        .await
        .map_err(recovery_command_error)?;
    let outcome =
        if let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
            ref action_id,
            ..
        } = execution.outcome
        {
            let status = deps
                .shutdown
                .get_shutdown_target_action_status(action_id)
                .await
                .map_err(recovery_command_error)?;
            RecoveryActionOutcomeDtoV1::from_durable_status(status)
        } else {
            execution.outcome.into()
        };
    if let Some(process_action) = execution.process_action {
        deps.process_actions
            .dispatch_tauri(deps.app.clone(), process_action);
    }
    Ok(AgentSessionWsResultV1::RecoveryActionOutcome { outcome })
}

async fn get_shutdown_plan(
    deps: &AgentSessionApiDeps,
    shutdown_id: String,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let page =
        crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
            deps.shutdown.as_ref(),
            shutdown_id,
            limit,
            cursor,
        )
        .await
        .map_err(OperationApplicationErrorDtoV1::from)?;
    Ok(AgentSessionWsResultV1::ShutdownPlan {
        page: Box::new(page),
    })
}

async fn request_send(
    deps: &AgentSessionApiDeps,
    operation_id: String,
    command: CanonicalSendCommandV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    request_send_for_principal(deps, LOCAL_API_OPERATION_PRINCIPAL, operation_id, command).await
}

async fn request_send_for_principal(
    deps: &AgentSessionApiDeps,
    principal: &str,
    operation_id: String,
    command: CanonicalSendCommandV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    request_send_with_durable_dispatcher(
        deps.local_store.as_ref(),
        deps.send.as_ref(),
        deps.caller_journal.as_ref(),
        principal,
        operation_id,
        command,
    )
    .await
}

fn send_command_error(error: SendCommandErrorDtoV1) -> OperationApplicationErrorDtoV1 {
    match error {
        SendCommandErrorDtoV1::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
        SendCommandErrorDtoV1::PayloadConflict => OperationApplicationErrorDtoV1::PayloadConflict,
        SendCommandErrorDtoV1::NotFound => OperationApplicationErrorDtoV1::NotFound,
        SendCommandErrorDtoV1::CapacityExceeded => OperationApplicationErrorDtoV1::CapacityExceeded,
        SendCommandErrorDtoV1::FeedbackCapacityExceeded => {
            OperationApplicationErrorDtoV1::FeedbackCapacityExceeded
        }
        SendCommandErrorDtoV1::ShutdownInProgress => {
            OperationApplicationErrorDtoV1::ShutdownInProgress
        }
        SendCommandErrorDtoV1::ResponseTooLarge => OperationApplicationErrorDtoV1::ResponseTooLarge,
        SendCommandErrorDtoV1::Internal { correlation_id } => {
            OperationApplicationErrorDtoV1::Internal { correlation_id }
        }
    }
}

async fn request_send_with_durable_dispatcher(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
    send: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    journal: &crate::usecase::agent_session::operation::CallerAttemptJournal,
    principal: &str,
    operation_id: String,
    command: CanonicalSendCommandV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let outcome =
        crate::adaptor::controller::command::agent_session::session::dispatch_durable_send_for_principal(
            store,
            send,
            journal,
            principal,
            operation_id,
            command,
        )
        .await
        .map_err(send_command_error)?;
    Ok(AgentSessionWsResultV1::SendOutcome { outcome })
}

#[cfg(test)]
async fn execute_send_for_principal(
    send: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    principal: &str,
    operation_id: String,
    canonical_payload: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let outcome = send
        .send(SendOperationRequest {
            principal: principal.to_string(),
            operation_id: operation_id.clone(),
            canonical_payload,
        })
        .await
        .map_err(|error| match error {
            SendAgentMessageError::InvalidRequest => OperationApplicationErrorDtoV1::InvalidRequest,
            SendAgentMessageError::PayloadConflict => {
                OperationApplicationErrorDtoV1::PayloadConflict
            }
            SendAgentMessageError::ShutdownInProgress => {
                OperationApplicationErrorDtoV1::ShutdownInProgress
            }
            SendAgentMessageError::NotFound => OperationApplicationErrorDtoV1::NotFound,
            SendAgentMessageError::CapacityExceeded => {
                OperationApplicationErrorDtoV1::CapacityExceeded
            }
            SendAgentMessageError::Internal { correlation_id } => {
                OperationApplicationErrorDtoV1::Internal { correlation_id }
            }
        })?;
    match outcome {
        SendCommandOutcome::Accepted(operation) => Ok(AgentSessionWsResultV1::SendOutcome {
            outcome: SendCommandOutcomeDtoV1::Accepted {
                operation: operation.into(),
            },
        }),
        SendCommandOutcome::OutcomeUnknown { operation_id } => {
            Ok(AgentSessionWsResultV1::SendOutcome {
                outcome: SendCommandOutcomeDtoV1::OutcomeUnknown { operation_id },
            })
        }
        SendCommandOutcome::RejectedBeforeCommit { failure } => {
            Ok(AgentSessionWsResultV1::SendOutcome {
                outcome: SendCommandOutcomeDtoV1::RejectedBeforeCommit {
                    failure: failure.into(),
                },
            })
        }
    }
}

async fn get_send(
    send: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    operation_id: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    get_send_for_principal(send, LOCAL_API_OPERATION_PRINCIPAL, operation_id).await
}

async fn get_send_for_principal(
    send: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    principal: &str,
    operation_id: String,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    send.get_operation(principal, &operation_id)
        .await
        .map(|operation| AgentSessionWsResultV1::SendOperation {
            operation: operation.into(),
        })
        .map_err(send_query_error)
}

async fn request_permission_response(
    deps: &AgentSessionApiDeps,
    request: PermissionResponseRequestDtoV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let response = crate::adaptor::controller::command::agent_session::permission::permission_response_from_command(
        request.request_id.clone(),
        request.behavior,
        request.message,
        request.updated_input,
    )
    .map_err(|_| OperationApplicationErrorDtoV1::InvalidRequest)?;

    // A committed operation remains replayable after admission closes. The
    // usecase still validates the immutable principal and exact payload.
    match deps
        .permission_response
        .get_operation(LOCAL_API_OPERATION_PRINCIPAL, &request.operation_id)
        .await
    {
        Ok(_) => {}
        Err(
            crate::usecase::agent_session::operation::GetPermissionResponseOperationError::NotFound,
        ) => {
            ensure_mutation_admission(deps).await.map_err(|error| {
                send_or_stop_admission_error("websocket_permission_response_admission", error)
            })?;
        }
        Err(
            crate::usecase::agent_session::operation::GetPermissionResponseOperationError::InvalidRequest,
        ) => return Err(OperationApplicationErrorDtoV1::InvalidRequest),
        Err(_) => {
            return Ok(AgentSessionWsResultV1::PermissionResponseOutcome {
                outcome: PermissionResponseCommandOutcomeDtoV1::OutcomeUnknown {
                    operation_id: request.operation_id,
                },
            });
        }
    }
    let outcome = deps
        .permission_response
        .request(
            crate::usecase::agent_session::operation::PermissionResponseOperationRequest {
                principal: LOCAL_API_OPERATION_PRINCIPAL.to_string(),
                operation_id: request.operation_id,
                session_id: request.session_id,
                response,
            },
        )
        .await
        .map_err(|error| {
            OperationApplicationErrorDtoV1::from(
                crate::adaptor::presenter::agent_session::permission_response_command_error(error),
            )
        })?;
    Ok(AgentSessionWsResultV1::PermissionResponseOutcome {
        outcome: crate::adaptor::presenter::agent_session::permission_response_outcome(outcome),
    })
}

async fn request_stop(
    deps: &AgentSessionApiDeps,
    request: StopOperationRequestDtoV1,
) -> Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> {
    let expected_session_revision =
        decode_nonnegative_u64_decimal(&request.expected_session_revision)
            .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?;
    let turn_id = decode_positive_i64_decimal(&request.turn_id)
        .ok_or(OperationApplicationErrorDtoV1::InvalidRequest)?
        .to_string();
    match deps
        .stop
        .get_operation(LOCAL_API_OPERATION_PRINCIPAL, &request.request_id)
        .await
    {
        Ok(_) => {}
        Err(crate::usecase::agent_session::operation::StopOperationError::NotFound) => {
            ensure_mutation_admission(deps)
                .await
                .map_err(|error| send_or_stop_admission_error("websocket_stop_admission", error))?;
        }
        Err(crate::usecase::agent_session::operation::StopOperationError::InvalidRequest) => {
            return Err(OperationApplicationErrorDtoV1::InvalidRequest);
        }
        Err(_) => {
            return Ok(AgentSessionWsResultV1::StopOutcome {
                outcome: StopCommandOutcomeDtoV1::OutcomeUnknown {
                    request_id: request.request_id,
                },
            });
        }
    }
    let outcome = deps
        .stop
        .request(StopOperationRequest {
            principal: LOCAL_API_OPERATION_PRINCIPAL.to_string(),
            request_id: request.request_id,
            session_id: request.session_id,
            turn_id,
            expected_session_revision,
        })
        .await
        .map_err(stop_command_error)?;
    Ok(AgentSessionWsResultV1::StopOutcome {
        outcome: outcome.into(),
    })
}

// Keep the trait import local so the response type of `upgrade` stays plain.
use axum::response::IntoResponse as _;

#[cfg(test)]
mod tests {
    use super::*;

    struct UnreadableWebSocketSessionLoader;

    struct ManualRecoveryExecutor {
        effects: std::sync::atomic::AtomicUsize,
    }

    struct SuccessfulFeedbackResolution {
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::feedback::FeedbackResolutionPort
        for SuccessfulFeedbackResolution
    {
        async fn retry_exact_resolution(
            &self,
            _resolution_identity: &str,
        ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::RecoveryEffectExecutor for ManualRecoveryExecutor {
        async fn execute(
            &self,
            _request: &crate::usecase::agent_session::operation::RecoveryEffectRequest,
        ) -> Result<
            crate::usecase::agent_session::operation::RecoveryEffectResult,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(
                crate::usecase::agent_session::operation::RecoveryEffectResult {
                    classification: crate::domain::agent_session::events::RecoveryResultClassification::Unchanged,
                    safe_result: "Kept for manual resolution.".to_string(),
					owner_mutations: Vec::new(),
                    owner_batch: None,
                },
            )
        }
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::session_feedback_load::SessionLoadPort
        for UnreadableWebSocketSessionLoader
    {
        async fn load_session(
            &self,
            _session_id: &str,
        ) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String>
        {
            Err(format!(
                "{} corrupt meta at /private/secret/session.json token=raw-secret \
                 sql=SELECT * FROM terminal_records provider_payload={{\"prompt\":\"raw-provider-secret\"}}",
                "壊".repeat(1_000)
            ))
        }
    }

    #[tokio::test]
    async fn websocket_session_load_returns_safe_error_and_the_same_canonical_feedback_shape() {
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
                store.installation_id().to_string(),
            ),
        );
        let load =
            crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase::new(
                Arc::new(UnreadableWebSocketSessionLoader),
                feedback.clone(),
            );

        let error =
            get_feedback_supervised_session(&load, "unreadable-session", "websocket-load-attempt")
                .await
                .expect_err("unreadable data/meta must fail the public load");
        let response = AgentSessionWsResponseV1::Error {
            id: "outer-load".to_string(),
            error,
        };
        let public = serde_json::to_value(response).unwrap();
        assert_eq!(public["status"], "error");
        assert_eq!(public["error"]["type"], "storage_unavailable");
        assert_eq!(public["error"]["failure"]["kind"], "persist_failure");
        assert!(public["error"]["failure"]["label"].as_str().unwrap().len() <= 160);
        assert!(public["error"]["failure"]["detail"].as_str().unwrap().len() <= 2_048);
        assert!(!public.to_string().contains("raw-secret"));
        assert!(!public.to_string().contains("/private/secret"));
        assert!(!public.to_string().contains("SELECT * FROM"));
        assert!(!public.to_string().contains("raw-provider-secret"));

        let page = feedback
            .list("unreadable-session", 32, None)
            .await
            .expect("feedback query must not depend on readable session data");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].attempt_id, "websocket-load-attempt");
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
            public["error"]["failure"]["correlation_id"]
                .as_str()
                .unwrap()
        );
        assert!(
            crate::usecase::agent_session::session_feedback_load::session_load_failure_was_logged(
                public["error"]["failure"]["correlation_id"]
                    .as_str()
                    .unwrap(),
            ),
            "the exact public/feedback correlation identity must also reach the failure log",
        );
    }

    #[tokio::test]
    async fn b072_feedback_page_dismiss_and_retry_are_semantically_equal_across_surfaces() {
        use tauri::Manager as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let tauri_data = tempfile::tempdir().unwrap();
        let websocket_data = tempfile::tempdir().unwrap();
        let build = |path: &std::path::Path| {
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    path.to_path_buf(),
                ),
            )
            .unwrap();
            let resolution = Arc::new(SuccessfulFeedbackResolution {
                effects: std::sync::atomic::AtomicUsize::new(0),
            });
            let feedback = Arc::new(
                crate::usecase::agent_session::feedback::SessionFeedbackUsecase::new(
                    store.clone(),
                    store.installation_id().to_string(),
                )
                .with_resolution_port(resolution.clone()),
            );
            (store, feedback, resolution)
        };
        let (_tauri_store, tauri_feedback, tauri_resolution) = build(tauri_data.path());
        let (_websocket_store, websocket_feedback, websocket_resolution) =
            build(websocket_data.path());

        let tauri_reservation = tauri_feedback
            .reserve_attempt(
                "b072-feedback-session",
                crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation::LoadSession,
                "b072-feedback-attempt",
            )
            .await
            .unwrap();
        let websocket_reservation = websocket_feedback
            .reserve_attempt(
                "b072-feedback-session",
                crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation::LoadSession,
                "b072-feedback-attempt",
            )
            .await
            .unwrap();
        let failure = || {
            crate::domain::local_event::SafeOperationFailure::new(
                crate::domain::local_event::SessionOperationFailureKind::PersistFailure,
                true,
                "The session could not be loaded.",
                "b072-feedback-correlation",
            )
        };
        let tauri_entry = tauri_feedback
            .materialize_failure(&tauri_reservation, failure(), None)
            .await
            .unwrap();
        let websocket_entry = websocket_feedback
            .materialize_failure(&websocket_reservation, failure(), None)
            .await
            .unwrap();
        let tauri_retry_reservation = tauri_feedback
            .reserve_attempt(
                "b072-feedback-session",
                crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation::RespondPermission,
                "b072-feedback-retry-attempt",
            )
            .await
            .unwrap();
        let websocket_retry_reservation = websocket_feedback
            .reserve_attempt(
                "b072-feedback-session",
                crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation::RespondPermission,
                "b072-feedback-retry-attempt",
            )
            .await
            .unwrap();
        let tauri_retry_entry = tauri_feedback
            .materialize_failure(
                &tauri_retry_reservation,
                failure(),
                Some("b072-resolution-identity".to_string()),
            )
            .await
            .unwrap();
        let websocket_retry_entry = websocket_feedback
            .materialize_failure(
                &websocket_retry_reservation,
                failure(),
                Some("b072-resolution-identity".to_string()),
            )
            .await
            .unwrap();
        let tauri_action = tauri_entry
            .action_identity(crate::usecase::agent_session::feedback::FeedbackAction::Dismiss)
            .unwrap()
            .to_string();
        let websocket_action = websocket_entry
            .action_identity(crate::usecase::agent_session::feedback::FeedbackAction::Dismiss)
            .unwrap()
            .to_string();
        assert_eq!(tauri_entry.feedback_id, websocket_entry.feedback_id);
        assert_eq!(tauri_action, websocket_action);
        assert_eq!(
            tauri_retry_entry.feedback_id,
            websocket_retry_entry.feedback_id
        );
        let tauri_retry_action = tauri_retry_entry
            .action_identity(
                crate::usecase::agent_session::feedback::FeedbackAction::RetryResolution,
            )
            .unwrap()
            .to_string();
        let websocket_retry_action = websocket_retry_entry
            .action_identity(
                crate::usecase::agent_session::feedback::FeedbackAction::RetryResolution,
            )
            .unwrap()
            .to_string();
        assert_eq!(tauri_retry_action, websocket_retry_action);

        let app = tauri::test::mock_builder()
            .manage(tauri_feedback.clone())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(FeedbackWsDispatcher {
            feedback: websocket_feedback.clone(),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("authenticated feedback WebSocket");
        let tauri_page =
            crate::adaptor::controller::command::agent_session::notice::list_agent_session_feedback(
                app.state::<Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>>(),
                "b072-feedback-session".to_string(),
                32,
                None,
            )
            .await
            .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "list_feedback",
                    "id": "b072-feedback-page",
                    "session_id": "b072-feedback-session",
                    "limit": 32,
                    "cursor": null,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_page = response_json(&mut socket).await;
        assert_eq!(websocket_page["status"], "ok");
        assert_eq!(websocket_page["result"]["type"], "feedback_page");
        assert_eq!(
            websocket_page["result"]["page"],
            serde_json::to_value(&tauri_page).unwrap()
        );

        crate::adaptor::controller::command::agent_session::notice::dismiss_agent_session_feedback(
            app.state::<Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>>(),
            "b072-feedback-session".to_string(),
            tauri_entry.feedback_id.clone(),
            tauri_entry.revision.to_string(),
            tauri_action,
        )
        .await
        .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "dismiss_feedback",
                    "id": "b072-feedback-dismiss",
                    "session_id": "b072-feedback-session",
                    "feedback_id": websocket_entry.feedback_id,
                    "expected_revision": websocket_entry.revision.to_string(),
                    "action_id": websocket_action,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_dismissed = response_json(&mut socket).await;
        assert_eq!(websocket_dismissed["status"], "ok");
        assert_eq!(websocket_dismissed["result"]["type"], "feedback_dismissed");
        assert_eq!(
            websocket_dismissed["result"]["feedback_id"],
            websocket_entry.feedback_id
        );
        let tauri_retried =
            crate::adaptor::controller::command::agent_session::notice::retry_agent_session_feedback(
                app.state::<Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>>(),
                "b072-feedback-session".to_string(),
                tauri_retry_entry.feedback_id.clone(),
                tauri_retry_entry.revision.to_string(),
                tauri_retry_action,
            )
            .await
            .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "retry_feedback",
                    "id": "b072-feedback-retry",
                    "session_id": "b072-feedback-session",
                    "feedback_id": websocket_retry_entry.feedback_id,
                    "expected_revision": websocket_retry_entry.revision.to_string(),
                    "action_id": websocket_retry_action,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_retried = response_json(&mut socket).await;
        assert_eq!(websocket_retried["status"], "ok");
        assert_eq!(websocket_retried["result"]["type"], "feedback_retried");
        assert_eq!(
            websocket_retried["result"]["outcome"],
            serde_json::to_value(tauri_retried).unwrap()
        );
        assert_eq!(
            tauri_resolution
                .effects
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            websocket_resolution
                .effects
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let tauri_after = tauri_feedback
            .list("b072-feedback-session", 32, None)
            .await
            .unwrap();
        let websocket_after = websocket_feedback
            .list("b072-feedback-session", 32, None)
            .await
            .unwrap();
        assert!(tauri_after.entries.is_empty());
        assert_eq!(tauri_after, websocket_after);

        let missing_action_identity = serde_json::json!({
            "type": "dismiss_feedback",
            "id": "b072-missing-action",
            "session_id": "b072-feedback-session",
            "feedback_id": websocket_entry.feedback_id,
            "expected_revision": "0"
        });
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(missing_action_identity).is_err(),
            "WebSocket controls must carry the backend-issued action identity"
        );
        let missing_retry_action_identity = serde_json::json!({
            "type": "retry_feedback",
            "id": "b072-missing-retry-action",
            "session_id": "b072-feedback-session",
            "feedback_id": websocket_retry_entry.feedback_id,
            "expected_revision": "0"
        });
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(missing_retry_action_identity)
                .is_err(),
            "WebSocket retry control must carry the backend-issued action identity"
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b007_pending_caller_send_query_is_unknown_then_converges_on_public_surfaces() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;
        use crate::usecase::agent_session::session::AgentSessionProjectionCodec as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        async fn websocket_get(
            store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
            send: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
            journal: Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>,
            operation_id: &str,
        ) -> serde_json::Value {
            let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableSendWsDispatcher {
                store,
                send,
                journal,
            });
            let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
            let mut request = url.as_str().into_client_request().unwrap();
            request
                .headers_mut()
                .insert("authorization", "Bearer b004-token".parse().unwrap());
            let (mut socket, _) = tokio_tungstenite::connect_async(request)
                .await
                .expect("authenticated B007 WebSocket");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "get_send",
                        "id": format!("b007-query-{operation_id}"),
                        "operation_id": operation_id,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let response = response_json(&mut socket).await;
            let _ = socket.close(None).await;
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(250), socket.next()).await;
            server.abort();
            let _ = server.await;
            tokio::task::yield_now().await;
            response
        }

        async fn websocket_send(
            store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
            send: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
            journal: Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>,
            operation_id: &str,
            command: CanonicalSendCommandV1,
        ) -> serde_json::Value {
            let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableSendWsDispatcher {
                store,
                send,
                journal,
            });
            let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
            let mut request = url.as_str().into_client_request().unwrap();
            request
                .headers_mut()
                .insert("authorization", "Bearer b004-token".parse().unwrap());
            let (mut socket, _) = tokio_tungstenite::connect_async(request)
                .await
                .expect("authenticated B007 send WebSocket");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "request_send",
                        "id": format!("b007-send-{operation_id}"),
                        "operation_id": operation_id,
                        "command": command,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let response = response_json(&mut socket).await;
            let _ = socket.close(None).await;
            server.abort();
            let _ = server.await;
            response
        }

        let data = tempfile::tempdir().unwrap();
        let session_id = "b007-public-session";
        let operation_id = "b007-public-outcome-unknown";
        let rejected_id = "b007-public-rejected";
        let worktree_path = data.path().to_string_lossy().to_string();
        let command = CanonicalSendCommandV1 {
            target: crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                chat_session_id: Some(session_id.to_string()),
                worktree_path: worktree_path.clone(),
            },
            content: "resolve this exact caller attempt".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("codex".to_string()),
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };
        let canonical_payload = serde_json::to_string(&command).unwrap();

        {
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    data.path().to_path_buf(),
                ),
            )
            .unwrap();
            let session_store = Arc::new(crate::test_support::build_session_store());
            let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
                store.clone();
            session_store.set_local_event_repository(
                repository,
                store.installation_id().to_string(),
                Arc::new(
                    crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
                ),
            );
            let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
                session_id.to_string(),
                &worktree_path,
                Some("codex".to_string()),
                crate::domain::agent_session::PermissionMode::Ask,
                None,
                false,
                false,
                None,
            );
            session.state = crate::usecase::agent_session::session::SessionState::Idle;
            session_store
                .save_full_session_for_restore(data.path(), &session)
                .unwrap();
            let gate = Arc::new(PublicIdentitySendGate {
                session_store: session_store.clone(),
                session_id: session_id.to_string(),
                plan_calls: std::sync::atomic::AtomicUsize::new(0),
                effects: std::sync::atomic::AtomicUsize::new(0),
            });
            let send = Arc::new(
                crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                    store.clone(),
                    store.clone(),
                    gate.clone(),
                    store.installation_id().to_string(),
                ),
            );
            let journal = Arc::new(
                crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                    store.clone(),
                    store.clone(),
                    store.installation_id().to_string(),
                ),
            );
            journal
                .record_attempt_scoped(
                    LOCAL_API_OPERATION_PRINCIPAL,
                    crate::domain::local_event::OperationKind::Send,
                    operation_id,
                    canonical_payload.as_bytes(),
                    Some(session_id),
                )
                .await
                .unwrap();
            store.fault_injector().arm_drop_reply();

            let outcome =
                crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                    store.as_ref(),
                    send.as_ref(),
                    journal.as_ref(),
                    operation_id.to_string(),
                    command.clone(),
                )
                .await
                .expect("writer reply loss is a closed send outcome");
            assert_eq!(
                serde_json::to_value(outcome).unwrap(),
                serde_json::json!({
                    "type": "outcome_unknown",
                    "operation_id": operation_id,
                })
            );
            assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 0);

            let tauri = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
                send.as_ref(),
                LOCAL_API_OPERATION_PRINCIPAL,
                operation_id.to_string(),
            )
            .await
            .expect("lookup resolves the durably committed send after reply loss");
            let tauri = serde_json::to_value(tauri).unwrap();
            assert_eq!(tauri["receipt"]["operation_id"], operation_id);
            assert_eq!(tauri["latest_status"]["type"], "awaiting_provider_start");
            let websocket = websocket_get(store.clone(), send, journal, operation_id).await;
            assert_eq!(websocket["status"], "ok");
            assert_eq!(websocket["result"]["type"], "send_operation");
            assert_eq!(websocket["result"]["operation"], tauri);
        }

        let store = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                    crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                        data.path().to_path_buf(),
                    ),
                ) {
                    Ok(store) => break store,
                    Err(
                        crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError::WriterLockHeld,
                    ) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                    Err(error) => panic!("restart canonical store failed: {error}"),
                }
            }
        })
        .await
        .expect("prior public transport must release the crashed store writer authority");
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository(
            repository,
            store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let gate = Arc::new(PublicIdentitySendGate {
            session_store: session_store.clone(),
            session_id: session_id.to_string(),
            plan_calls: std::sync::atomic::AtomicUsize::new(0),
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate.clone(),
                store.installation_id().to_string(),
            ),
        );
        let journal = Arc::new(
            crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                store.clone(),
                store.clone(),
                store.installation_id().to_string(),
            ),
        );
        assert_eq!(
            send.recover_pending_provider_effects_pass().await.unwrap(),
            1,
            "production startup recovery must resume the accepted unreserved provider effect"
        );
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);

        let conflict_id = "b007-public-payload-conflict";
        journal
            .record_attempt_scoped(
                LOCAL_API_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::Send,
                conflict_id,
                canonical_payload.as_bytes(),
                Some(session_id),
            )
            .await
            .unwrap();
        let mut changed_command = command.clone();
        changed_command.content = "changed after the caller crash".to_string();
        let conflict = websocket_send(
            store.clone(),
            send.clone(),
            journal.clone(),
            conflict_id,
            changed_command,
        )
        .await;
        assert_eq!(conflict["status"], "error");
        assert_eq!(conflict["error"]["type"], "payload_conflict");
        assert_eq!(
            gate.effects.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "payload conflict must not duplicate the recovered provider effect"
        );

        let accepted = websocket_send(
            store.clone(),
            send.clone(),
            journal.clone(),
            operation_id,
            command.clone(),
        )
        .await;
        assert_eq!(accepted["status"], "ok");
        assert_eq!(accepted["result"]["type"], "send_outcome");
        assert_eq!(accepted["result"]["outcome"]["type"], "accepted");
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        let attempts = journal
            .pending_page_for_scope(LOCAL_API_OPERATION_PRINCIPAL, session_id, 8, None)
            .await
            .unwrap()
            .entries;
        assert_eq!(
            attempts
                .iter()
                .find(|attempt| attempt.caller_request_id == operation_id)
                .unwrap()
                .resolution,
            crate::domain::local_event::CallerAttemptResolution::Accepted
        );
        assert_eq!(
            attempts
                .iter()
                .find(|attempt| attempt.caller_request_id == conflict_id)
                .unwrap()
                .resolution,
            crate::domain::local_event::CallerAttemptResolution::Pending
        );
        let tauri = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
            send.as_ref(),
            LOCAL_API_OPERATION_PRINCIPAL,
            operation_id.to_string(),
        )
        .await
        .expect("resolved operation through Tauri query");
        let websocket =
            websocket_get(store.clone(), send.clone(), journal.clone(), operation_id).await;
        assert_eq!(websocket["status"], "ok");
        assert_eq!(websocket["result"]["type"], "send_operation");
        assert_eq!(
            websocket["result"]["operation"],
            serde_json::to_value(tauri).unwrap()
        );
        let (_, page, _) = session_store
            .get_session_with_latest_page(data.path(), session_id, 32)
            .unwrap()
            .expect("one recovered session projection");
        assert_eq!(
            page.messages
                .iter()
                .filter(|message| message.id == "b009-human")
                .count(),
            1,
            "caller-attempt replay must materialize one human message"
        );
        assert_eq!(
            page.messages
                .iter()
                .filter(|message| message.id == "b009-human:agent")
                .count(),
            1,
            "caller-attempt replay must reserve one assistant message"
        );
        let stream = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session(session_id).unwrap(),
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
                                crate::domain::agent_session::events::AgentSessionDomainEvent::SendOperationAccepted { operation_id: saved, .. }
                            ) if saved == operation_id
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
                                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted { .. }
                            )
                        )
                ))
                .count(),
            1
        );
        let projection = match store
            .query(
                crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                    session_id: session_id.to_string(),
                },
            )
            .await
            .unwrap()
        {
            crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                Some(projection),
            ) => projection,
            other => panic!("unexpected recovered session projection: {other:?}"),
        };
        let projection =
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
                .decode(&projection.projection)
                .unwrap();
        assert!(projection.pending_send_queue.is_empty());

        journal
            .record_attempt(
                LOCAL_API_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::Send,
                rejected_id,
                canonical_payload.as_bytes(),
            )
            .await
            .unwrap();
        let pending = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
            send.as_ref(),
            LOCAL_API_OPERATION_PRINCIPAL,
            rejected_id.to_string(),
        )
        .await
        .expect_err("second Pending identity must be unknown");
        assert_eq!(
            serde_json::to_value(pending).unwrap()["type"],
            "outcome_unknown"
        );
        journal
            .clear_attempt(
                LOCAL_API_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::Send,
                rejected_id,
                canonical_payload.as_bytes(),
                false,
            )
            .await
            .unwrap();
        let tauri = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
            send.as_ref(),
            LOCAL_API_OPERATION_PRINCIPAL,
            rejected_id.to_string(),
        )
        .await
        .expect_err("rejected-before-commit identity must converge to NotFound");
        assert_eq!(serde_json::to_value(tauri).unwrap()["type"], "not_found");
        let websocket = websocket_get(store.clone(), send, journal, rejected_id).await;
        assert_eq!(websocket["status"], "error");
        assert_eq!(websocket["error"]["type"], "not_found");
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b072_recovery_page_and_action_are_semantically_equal_across_surfaces() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;
        use crate::usecase::agent_session::operation::OperationBindingAuthority as _;
        use tauri::Manager as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let obligation_id = "b072-recovery-obligation";
        let record = crate::domain::local_event::ObligationRecord::ProviderEstablish {
            operation_id: obligation_id.to_string(),
            effect_identity: format!("{obligation_id}.provider"),
            session_id: "b072-recovery-session".to_string(),
            state: crate::domain::local_event::ObligationStateRecord::Pending,
        };
        store
            .commit_batch(crate::domain::local_event::LocalAtomicBatch {
                commit_id: crate::domain::local_event::CommitIdentity::parse("b072-recovery-seed")
                    .unwrap(),
                idempotency: crate::domain::local_event::IdempotencyBinding {
                    installation_id: store.installation_id().to_string(),
                    operation_kind: crate::domain::local_event::CommitOperationKind::Recovery,
                    idempotency_key: "b072-recovery-seed".to_string(),
                    payload_hash: store.digest(b"b072-recovery-seed/v1"),
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![crate::domain::local_event::LocalStateMutation::Obligation(
                    crate::domain::local_event::ObligationMutation {
                        obligation_id: obligation_id.to_string(),
                        record,
                        pending: Some(crate::domain::local_event::PendingIndexEntry {
                            ordered_key: "b072-recovery-0001".to_string(),
                            owner: "b072-recovery-session".to_string(),
                            partition: crate::domain::local_event::PendingPartition::Owner,
                            shutdown_plan: None,
                        }),
                        expected: crate::domain::local_event::RevisionGuard::Absent,
                        revision: crate::domain::local_event::Revision::new(0).unwrap(),
                    },
                )],
            })
            .await
            .unwrap();
        let executor = Arc::new(ManualRecoveryExecutor {
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let recovery = Arc::new(
            crate::usecase::agent_session::operation::RecoveryActionUsecase::new(
                store.clone(),
                store.clone(),
                executor.clone(),
                store.installation_id().to_string(),
            ),
        );
        let app = tauri::test::mock_builder()
            .manage(store.clone())
            .manage(recovery.clone())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(RecoveryWsDispatcher {
            recovery: recovery.clone(),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut websocket_request = url.as_str().into_client_request().unwrap();
        websocket_request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
            .await
            .expect("authenticated recovery WebSocket");

        let tauri_page = crate::adaptor::controller::command::agent_session::session::list_pending_agent_recovery(
            app.state::<Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>(),
            Some(32),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_pending_recovery",
                    "id": "b072-recovery-page",
                    "limit": 32,
                    "partition": null,
                    "owner": null,
                    "shutdown_id": null,
                    "cursor": null,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_page = response_json(&mut socket).await;
        assert_eq!(websocket_page["status"], "ok");
        assert_eq!(websocket_page["result"]["type"], "pending_recovery");
        assert_eq!(
            websocket_page["result"]["page"],
            serde_json::to_value(&tauri_page).unwrap()
        );

        let identity = tauri_page.entries[0].action_identities[0].clone();
        let request = RecoveryActionRequestDtoV1 {
            action_id: identity.action_id,
            obligation_id: tauri_page.entries[0].obligation_id.clone(),
            origin_revision: identity.origin_revision,
            action: identity.action,
        };
        let websocket_request = serde_json::json!({
            "action_id": &request.action_id,
            "obligation_id": &request.obligation_id,
            "origin_revision": &request.origin_revision,
            "action": request.action,
        });
        let origin_revision = decode_nonnegative_u64_decimal(&request.origin_revision).unwrap();
        let tauri_outcome = crate::adaptor::controller::command::agent_session::session::resolve_pending_recovery_action(
            app.state::<Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>(),
            app.state::<Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>>(),
            request.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            origin_revision,
            decode_nonnegative_u64_decimal(&request.origin_revision).unwrap()
        );
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_recovery_action",
                    "id": "b072-recovery-action",
                    "request": websocket_request,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_outcome = response_json(&mut socket).await;
        assert_eq!(websocket_outcome["status"], "ok");
        assert_eq!(
            websocket_outcome["result"]["type"],
            "recovery_action_outcome"
        );
        assert_eq!(
            websocket_outcome["result"]["outcome"],
            serde_json::to_value(tauri_outcome).unwrap()
        );
        assert_eq!(
            executor.effects.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the cross-surface action replay must not execute a second effect"
        );
        server.abort();
    }

    #[derive(Clone)]
    struct TransportTestState {
        dispatcher: Arc<dyn WsDispatchService>,
        connection_limit: Arc<tokio::sync::Semaphore>,
        outbound_byte_limit: usize,
        rate_per_second: f64,
        writer_gate: Option<(Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Notify>)>,
    }

    struct RecordingWsDispatcher {
        calls: std::sync::atomic::AtomicUsize,
        hold: std::sync::atomic::AtomicBool,
        oversized_response: std::sync::atomic::AtomicBool,
        boundary_response: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        released: tokio::sync::Notify,
    }

    impl RecordingWsDispatcher {
        fn new(hold: bool) -> Arc<Self> {
            Arc::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                hold: std::sync::atomic::AtomicBool::new(hold),
                oversized_response: std::sync::atomic::AtomicBool::new(false),
                boundary_response: std::sync::atomic::AtomicBool::new(false),
                entered: tokio::sync::Notify::new(),
                released: tokio::sync::Notify::new(),
            })
        }

        async fn wait_for_calls(&self, expected: usize) {
            while self.calls.load(std::sync::atomic::Ordering::SeqCst) < expected {
                self.entered.notified().await;
            }
        }

        fn release(&self) {
            self.hold.store(false, std::sync::atomic::Ordering::SeqCst);
            self.released.notify_waiters();
        }

        fn return_oversized_response(&self) {
            self.oversized_response
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }

        fn return_boundary_response(&self) {
            self.boundary_response
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl WsDispatchService for RecordingWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.entered.notify_waiters();
            while self.hold.load(std::sync::atomic::Ordering::SeqCst) {
                self.released.notified().await;
            }
            if self
                .boundary_response
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let empty = AgentSessionWsResponseV1::Error {
                    id: id.clone(),
                    error: OperationApplicationErrorDtoV1::Internal {
                        correlation_id: String::new(),
                    },
                };
                let overhead = serde_json::to_vec(&empty).unwrap().len();
                return AgentSessionWsResponseV1::Error {
                    id,
                    error: OperationApplicationErrorDtoV1::Internal {
                        correlation_id: "x".repeat(MAX_MESSAGE_BYTES - overhead),
                    },
                };
            }
            AgentSessionWsResponseV1::Error {
                id,
                error: if self
                    .oversized_response
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    OperationApplicationErrorDtoV1::Internal {
                        correlation_id: "x".repeat(MAX_MESSAGE_BYTES + 1),
                    }
                } else {
                    OperationApplicationErrorDtoV1::InvalidRequest
                },
            }
        }
    }

    async fn transport_test_upgrade(
        State(state): State<TransportTestState>,
        ws: WebSocketUpgrade,
    ) -> Response {
        let Ok(permit) = state.connection_limit.clone().try_acquire_owned() else {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        let outbound_byte_limit = state.outbound_byte_limit;
        let rate_per_second = state.rate_per_second;
        let writer_gate = state.writer_gate;
        let dispatcher: Arc<dyn WsDispatchService> = state.dispatcher;
        ws.max_message_size(MAX_MESSAGE_BYTES + 1)
            .max_frame_size(MAX_MESSAGE_BYTES + 1)
            .on_upgrade(move |socket| {
                serve_with_outbound_budget(
                    socket,
                    dispatcher,
                    permit,
                    outbound_byte_limit,
                    rate_per_second,
                    writer_gate,
                )
            })
    }

    async fn spawn_transport_server(
        dispatcher: Arc<dyn WsDispatchService>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        spawn_transport_server_with_budget(dispatcher, MAX_MESSAGE_BYTES).await
    }

    async fn spawn_transport_server_with_budget(
        dispatcher: Arc<dyn WsDispatchService>,
        outbound_byte_limit: usize,
    ) -> (String, tokio::task::JoinHandle<()>) {
        spawn_transport_server_with_writer_gate(dispatcher, outbound_byte_limit, None).await
    }

    async fn spawn_transport_server_with_writer_gate(
        dispatcher: Arc<dyn WsDispatchService>,
        outbound_byte_limit: usize,
        writer_gate: Option<(Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Notify>)>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/v1/agent-session", get(transport_test_upgrade))
            .with_state(TransportTestState {
                dispatcher,
                connection_limit: Arc::new(tokio::sync::Semaphore::new(
                    super::super::MAX_AGENT_SESSION_CONNECTIONS,
                )),
                outbound_byte_limit,
                rate_per_second: 0.0,
                writer_gate,
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("ws://{address}/v1/agent-session"), server)
    }

    async fn spawn_authenticated_transport_server(
        dispatcher: Arc<dyn WsDispatchService>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/v1/agent-session", get(transport_test_upgrade))
            .layer(axum::middleware::from_fn_with_state(
                Arc::<str>::from("b004-token"),
                super::super::auth::require_bearer,
            ))
            .with_state(TransportTestState {
                dispatcher,
                connection_limit: Arc::new(tokio::sync::Semaphore::new(
                    super::super::MAX_AGENT_SESSION_CONNECTIONS,
                )),
                outbound_byte_limit: MAX_MESSAGE_BYTES,
                rate_per_second: 0.0,
                writer_gate: None,
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("ws://{address}/v1/agent-session"), server)
    }

    struct ConcurrentPublicSendGate {
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        planned: Arc<tokio::sync::Barrier>,
        effects: std::sync::atomic::AtomicUsize,
    }

    struct PublicIdentitySendGate {
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        session_id: String,
        plan_calls: std::sync::atomic::AtomicUsize,
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAcceptancePort for PublicIdentitySendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _operation_id: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.plan_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let allocation = self
                .session_store
                .send_acceptance_allocation(&self.session_id)
                .expect("public identity send allocation must be readable");
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: self.session_id.clone(),
                initial_session: None,
                session_projection_guard: allocation.session_projection_guard,
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: allocation.next_turn_id.to_string(),
                },
                input_ref: "b009-input".to_string(),
                human_message_id: "b009-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "identity boundary".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
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
                        session_projection_guard: plan.session_projection_guard,
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
                        "The identity-boundary projection could not be prepared.",
                        "b009-projection",
                    )
                })
        }

        async fn canonical_immediate_turn_is_current(
            &self,
            _session_id: &str,
            _turn_id: u64,
        ) -> Result<bool, crate::domain::local_event::SafeOperationFailure> {
            Ok(true)
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) -> Result<
            crate::usecase::agent_session::operation::SendEffectDispatch,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::usecase::agent_session::operation::SendEffectDispatch::Scheduled)
        }
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::SendAcceptancePort for ConcurrentPublicSendGate {
        async fn plan_send(
            &self,
            _principal: &str,
            _operation_id: &str,
            _canonical_payload: &str,
        ) -> Result<
            crate::usecase::agent_session::operation::SendPlan,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.planned.wait().await;
            Ok(crate::usecase::agent_session::operation::SendPlan {
                session_id: "b004-session".to_string(),
                initial_session: Some(
                    crate::usecase::agent_session::session::build_new_session_with_id(
                        "b004-session".to_string(),
                        "/tmp/b004-session",
                        Some("codex".to_string()),
                        crate::domain::agent_session::PermissionMode::Ask,
                        None,
                        false,
                        false,
                        None,
                    ),
                ),
                session_projection_guard: crate::domain::local_event::RevisionGuard::Absent,
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: "b004-input".to_string(),
                human_message_id: "b004-human".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "hello".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
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
                        session_projection_guard: plan.session_projection_guard,
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
                        "The concurrent send projection could not be prepared.",
                        "b004-projection",
                    )
                })
        }

        async fn canonical_immediate_turn_is_current(
            &self,
            _session_id: &str,
            _turn_id: u64,
        ) -> Result<bool, crate::domain::local_event::SafeOperationFailure> {
            Ok(true)
        }

        async fn start_provider_effect(
            &self,
            _effect: &crate::usecase::agent_session::operation::AcceptedSendEffect,
        ) -> Result<
            crate::usecase::agent_session::operation::SendEffectDispatch,
            crate::domain::local_event::SafeOperationFailure,
        > {
            self.effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::usecase::agent_session::operation::SendEffectDispatch::Scheduled)
        }
    }

    struct DurableSendWsDispatcher {
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        send: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
        journal: Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>,
    }

    struct FeedbackWsDispatcher {
        feedback: Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>,
    }

    struct RecoveryWsDispatcher {
        recovery: Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>,
    }

    #[async_trait::async_trait]
    impl WsDispatchService for RecoveryWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            let result = match request {
                AgentSessionWsRequestV1::GetPendingRecovery {
                    limit,
                    partition,
                    owner,
                    shutdown_id,
                    cursor,
                    ..
                } => {
                    list_pending_recovery(
                        &self.recovery,
                        limit.0,
                        partition.0,
                        owner.0,
                        shutdown_id.0,
                        cursor.0,
                    )
                    .await
                }
                AgentSessionWsRequestV1::RequestRecoveryAction { request, .. } => {
                    match decode_nonnegative_u64_decimal(&request.origin_revision) {
                        Some(origin_revision) => {
                            execute_recovery_action(&self.recovery, request, origin_revision)
                                .await
                                .map(|outcome| AgentSessionWsResultV1::RecoveryActionOutcome {
                                    outcome,
                                })
                        }
                        None => Err(OperationApplicationErrorDtoV1::InvalidRequest),
                    }
                }
                _ => return invalid(&id),
            };
            match result {
                Ok(result) => AgentSessionWsResponseV1::Ok {
                    id,
                    result: Box::new(result),
                },
                Err(error) => AgentSessionWsResponseV1::Error { id, error },
            }
        }
    }

    #[async_trait::async_trait]
    impl WsDispatchService for FeedbackWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            let result = match request {
                AgentSessionWsRequestV1::ListFeedback {
                    session_id,
                    limit,
                    cursor,
                    ..
                } => list_feedback(&self.feedback, session_id, limit.0, cursor.0).await,
                AgentSessionWsRequestV1::DismissFeedback {
                    session_id,
                    feedback_id,
                    expected_revision,
                    action_id,
                    ..
                } => {
                    dismiss_feedback(
                        &self.feedback,
                        session_id,
                        feedback_id,
                        expected_revision,
                        action_id,
                    )
                    .await
                }
                AgentSessionWsRequestV1::RetryFeedback {
                    session_id,
                    feedback_id,
                    expected_revision,
                    action_id,
                    ..
                } => {
                    retry_feedback(
                        &self.feedback,
                        session_id,
                        feedback_id,
                        expected_revision,
                        action_id,
                    )
                    .await
                }
                _ => return invalid(&id),
            };
            match result {
                Ok(result) => AgentSessionWsResponseV1::Ok {
                    id,
                    result: Box::new(result),
                },
                Err(error) => AgentSessionWsResponseV1::Error { id, error },
            }
        }
    }

    #[async_trait::async_trait]
    impl WsDispatchService for DurableSendWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            let result = match request {
                AgentSessionWsRequestV1::RequestSend {
                    operation_id,
                    command,
                    ..
                } => {
                    request_send_with_durable_dispatcher(
                        self.store.as_ref(),
                        self.send.as_ref(),
                        self.journal.as_ref(),
                        LOCAL_API_OPERATION_PRINCIPAL,
                        operation_id,
                        command,
                    )
                    .await
                }
                AgentSessionWsRequestV1::GetSend { operation_id, .. } => {
                    get_send(&self.send, operation_id).await
                }
                _ => return invalid(&id),
            };
            match result {
                Ok(result) => AgentSessionWsResponseV1::Ok {
                    id,
                    result: Box::new(result),
                },
                Err(error) => AgentSessionWsResponseV1::Error { id, error },
            }
        }
    }

    struct DurableStopGate {
        turn_id: String,
        session_revision: u64,
        effects: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::domain::agent_session::repository::AgentSessionLifecycleRepository for DurableStopGate {
        async fn restore_session(
            &self,
            session_id: &str,
        ) -> Result<
            crate::domain::agent_session::aggregates::session::Session,
            crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError,
        > {
            crate::usecase::agent_session::operation::StopTargetSnapshot {
                session_revision: self.session_revision,
                active_turn_id: self.turn_id.clone(),
                queue_paused: false,
            }
            .restore_session(session_id)
        }

        async fn prepare_session_change(
            &self,
            _session_id: &str,
            _expected_revision: u64,
            _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
        ) -> Result<
            Option<crate::domain::agent_session::repository::PreparedSessionChange>,
            crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError,
        > {
            Ok(Some(
                crate::domain::agent_session::repository::PreparedSessionChange::from_atomic_participant(
                    Vec::new(),
                ),
            ))
        }
    }

    #[async_trait::async_trait]
    impl crate::usecase::agent_session::operation::StopEffectPort for DurableStopGate {
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

    struct DurableStopWsDispatcher {
        usecase: Arc<crate::usecase::agent_session::operation::StopOperationUsecase>,
    }

    #[async_trait::async_trait]
    impl WsDispatchService for DurableStopWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            let AgentSessionWsRequestV1::RequestStop { request, .. } = request else {
                return invalid(&id);
            };
            let Some(expected_session_revision) =
                decode_nonnegative_u64_decimal(&request.expected_session_revision)
            else {
                return invalid(&id);
            };
            let Some(turn_id) = decode_positive_i64_decimal(&request.turn_id) else {
                return invalid(&id);
            };
            match self
                .usecase
                .request(
                    crate::usecase::agent_session::operation::StopOperationRequest {
                        principal: LOCAL_API_OPERATION_PRINCIPAL.to_string(),
                        request_id: request.request_id,
                        session_id: request.session_id,
                        turn_id: turn_id.to_string(),
                        expected_session_revision,
                    },
                )
                .await
            {
                Ok(outcome) => AgentSessionWsResponseV1::Ok {
                    id,
                    result: Box::new(AgentSessionWsResultV1::StopOutcome {
                        outcome: outcome.into(),
                    }),
                },
                Err(error) => AgentSessionWsResponseV1::Error {
                    id,
                    error: stop_command_error(error),
                },
            }
        }
    }

    #[derive(Default)]
    struct PublicShutdownExecutor {
        target_effects: std::sync::atomic::AtomicUsize,
        subordinate_shutdowns: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::usecase::shutdown_coordinator::ShutdownTargetExecutor for PublicShutdownExecutor {
        async fn targets(
            &self,
        ) -> Result<
            Vec<crate::usecase::shutdown_coordinator::ShutdownTarget>,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(Vec::new())
        }

        async fn execute_target(
            &self,
            _operation_id: &str,
            _effect_identity: &str,
            _owner_revision: crate::domain::local_event::Revision,
            _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
        ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
            self.target_effects
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn read_target_effect(
            &self,
            _operation_id: &str,
            _effect_identity: &str,
            _owner_revision: crate::domain::local_event::Revision,
            _target: &crate::usecase::shutdown_coordinator::ShutdownTarget,
        ) -> Result<
            crate::usecase::shutdown_coordinator::ShutdownEffectReadback,
            crate::domain::local_event::SafeOperationFailure,
        > {
            Ok(crate::usecase::shutdown_coordinator::ShutdownEffectReadback::ConfirmedNotStarted)
        }

        async fn shutdown_subordinates(
            &self,
        ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
            self.subordinate_shutdowns
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    struct DurableShutdownWsDispatcher {
        coordinator: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        exit_codes: Arc<std::sync::Mutex<Vec<i32>>>,
    }

    #[async_trait::async_trait]
    impl WsDispatchService for DurableShutdownWsDispatcher {
        async fn dispatch(&self, request: AgentSessionWsRequestV1) -> AgentSessionWsResponseV1 {
            let id = request.id().to_string();
            let result: Result<AgentSessionWsResultV1, OperationApplicationErrorDtoV1> =
                match request {
                    AgentSessionWsRequestV1::RequestApplicationQuit { request, .. } => {
                        crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
                            self.coordinator.as_ref(),
                            request,
                        )
                        .await
                        .map(|(outcome, process_action)| {
                            if let Some(process_action) = process_action {
                                self.exit_codes
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .push(process_action.code());
                            }
                            AgentSessionWsResultV1::ApplicationQuitOutcome { outcome }
                        })
                        .map_err(Into::into)
                    }
                    AgentSessionWsRequestV1::GetApplicationQuit { operation_id, .. } => {
                        crate::adaptor::controller::command::application_lifecycle::get_application_quit_operation_result(
                            self.coordinator.as_ref(),
                            operation_id,
                        )
                        .await
                        .map(|lookup| AgentSessionWsResultV1::ApplicationQuit { lookup })
                        .map_err(Into::into)
                    }
                    AgentSessionWsRequestV1::GetCurrentShutdown { .. } => {
                        crate::adaptor::controller::command::application_lifecycle::get_application_shutdown_result(
                            self.coordinator.as_ref(),
                        )
                        .await
                        .map(|result| AgentSessionWsResultV1::CurrentShutdown { result })
                        .map_err(Into::into)
                    }
                    AgentSessionWsRequestV1::GetShutdownPlan {
                        shutdown_id,
                        limit,
                        cursor,
                        ..
                    } => crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
                        self.coordinator.as_ref(),
                        shutdown_id,
                        limit.0,
                        cursor.0,
                    )
                    .await
                    .map(|page| AgentSessionWsResultV1::ShutdownPlan {
                        page: Box::new(page),
                    })
                    .map_err(Into::into),
                    _ => Err(OperationApplicationErrorDtoV1::InvalidRequest),
                };
            match result {
                Ok(result) => AgentSessionWsResponseV1::Ok {
                    id,
                    result: Box::new(result),
                },
                Err(error) => AgentSessionWsResponseV1::Error { id, error },
            }
        }
    }

    struct PublicQuitFixture {
        _data: tempfile::TempDir,
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        coordinator: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        executor: Arc<PublicShutdownExecutor>,
        operation_id: String,
        plan: crate::domain::local_event::ShutdownPlanKey,
    }

    impl PublicQuitFixture {
        async fn completed(request_id: &str) -> Self {
            let data = tempfile::tempdir().unwrap();
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    data.path().to_path_buf(),
                ),
            )
            .unwrap();
            let executor = Arc::new(PublicShutdownExecutor::default());
            let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
                store.clone();
            let authority: Arc<
                dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
            > = store.clone();
            let coordinator = Arc::new(
                crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                    repository,
                    authority,
                    executor.clone(),
                    store.installation_id().to_string(),
                    store.process_instance_id().to_string(),
                ),
            );
            let (outcome, _) = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
                coordinator.as_ref(),
                ApplicationQuitRequestDtoV1 {
                    request_id: request_id.to_string(),
                    intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                        code: 7,
                    },
                },
            )
            .await
            .expect("completed normal quit fixture");
            let outcome = serde_json::to_value(outcome).unwrap();
            assert_eq!(outcome["type"], "accepted");
            assert_eq!(outcome["state"]["type"], "completed");
            let operation_id = outcome["receipt"]["operation_id"]
                .as_str()
                .unwrap()
                .to_string();
            let plan = crate::domain::local_event::ShutdownPlanKey {
                shutdown_id: outcome["receipt"]["shutdown_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            };
            Self {
                _data: data,
                store,
                coordinator,
                executor,
                operation_id,
                plan,
            }
        }

        fn raw_connection(&self) -> rusqlite::Connection {
            let path = crate::adaptor::gateway::local_event_store::layout::StoreLayout::new(
                self._data.path(),
            )
            .database_path();
            rusqlite::Connection::open(path).expect("B088 boundary connection")
        }

        fn set_current_phase(&self, phase: &str, operation_state: &str, process_instance_id: &str) {
            let connection = self.raw_connection();
            let summary: String = connection
                .query_row(
                    "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
                    rusqlite::params![self.plan.shutdown_id],
                    |row| row.get(0),
                )
                .expect("B076 shutdown summary");
            let mut summary: serde_json::Value =
                serde_json::from_str(&summary).expect("B076 valid shutdown summary");
            summary["process_instance_id"] =
                serde_json::Value::String(process_instance_id.to_string());
            summary["outcome"] = serde_json::Value::String(
                match phase {
                    "completed" => "completed",
                    "failed" | "cancelled" => "aborted_before_activation",
                    "reconciliation_required" => "exited_with_recovery",
                    _ => "in_progress",
                }
                .to_string(),
            );
            let summary_failure = serde_json::json!({
                "kind": "deadline_exceeded",
                "retryable": true,
                "label": "B076 shutdown reconciliation is required.",
                "correlation_id": format!("b076-public-{phase}-failure"),
            });
            let status_failure = serde_json::json!({
                "kind": "DeadlineExceeded",
                "retryable": true,
                "message": "B076 shutdown reconciliation is required.",
                "correlation_id": format!("b076-public-{phase}-failure"),
            });
            if matches!(phase, "failed" | "cancelled" | "reconciliation_required") {
                summary["failure"] = summary_failure;
            } else if let Some(summary) = summary.as_object_mut() {
                summary.remove("failure");
            }
            let status = if matches!(
                operation_state,
                "failed_before_activation" | "reconciliation_required"
            ) {
                serde_json::json!({
                    "schema": "application_quit_status_v1",
                    "state": { "type": operation_state, "failure": status_failure },
                })
            } else {
                serde_json::json!({
                    "schema": "application_quit_status_v1",
                    "state": { "type": operation_state },
                })
            };
            connection
                .execute(
                    "UPDATE shutdown_plans SET phase = ?1, summary = ?2
                     WHERE shutdown_id = ?3",
                    rusqlite::params![phase, summary.to_string(), self.plan.shutdown_id],
                )
                .expect("B076 update shutdown phase");
            connection
                .execute(
                    "UPDATE operation_records SET latest_status = ?1
                     WHERE kind = 'application_quit' AND operation_id = ?2",
                    rusqlite::params![status.to_string(), self.operation_id],
                )
                .expect("B076 update shutdown operation status");
            connection
                .execute(
                    "UPDATE store_metadata
                     SET current_shutdown_id = ?1,
                         shutdown_pointer_revision = shutdown_pointer_revision + 1
                     WHERE id = 1",
                    rusqlite::params![self.plan.shutdown_id],
                )
                .expect("B076 restore current shutdown pointer");
        }

        async fn compact_and_wait_for_cleanup(&self) {
            self.coordinator
                .compact_shutdown_details(self.plan.clone())
                .await
                .expect("compact terminal quit details");
        }
    }

    async fn b076_empty_public_fixture() -> (
        tempfile::TempDir,
        Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        Arc<PublicShutdownExecutor>,
    ) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let executor = Arc::new(PublicShutdownExecutor::default());
        let coordinator = Arc::new(
            crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                store.clone(),
                store.clone(),
                executor.clone(),
                store.installation_id().to_string(),
                store.process_instance_id().to_string(),
            ),
        );
        (data, store, coordinator, executor)
    }

    async fn b088_assert_lookup_on_both_surfaces(
        coordinator: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        executor: &PublicShutdownExecutor,
        operation_id: &str,
        expected_type: &str,
        expected_state_type: Option<&str>,
        label: &str,
    ) -> Option<serde_json::Value> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let effects_before = (
            executor
                .target_effects
                .load(std::sync::atomic::Ordering::SeqCst),
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
        );
        let tauri = crate::adaptor::controller::command::application_lifecycle::get_application_quit_operation_result(
            coordinator.as_ref(),
            operation_id.to_string(),
        )
        .await;
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableShutdownWsDispatcher {
            coordinator,
            exit_codes: Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("authenticated B088 WebSocket");
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_application_quit",
                    "id": format!("b088-{label}"),
                    "operation_id": operation_id,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket = response_json(&mut socket).await;
        server.abort();

        let result = match tauri {
            Ok(tauri) => {
                let tauri = serde_json::to_value(tauri).unwrap();
                assert_eq!(websocket["status"], "ok", "{label}: {websocket}");
                assert_eq!(websocket["result"]["type"], "application_quit");
                assert_eq!(websocket["result"]["lookup"], tauri, "{label}");
                assert_eq!(tauri["type"], expected_type, "{label}");
                if let Some(expected_state_type) = expected_state_type {
                    assert_eq!(tauri["state"]["type"], expected_state_type, "{label}");
                }
                Some(tauri)
            }
            Err(tauri) => {
                let tauri = serde_json::to_value(tauri).unwrap();
                assert_eq!(websocket["status"], "error", "{label}: {websocket}");
                assert_eq!(websocket["error"]["type"], expected_type, "{label}");
                assert_eq!(tauri["type"], expected_type, "{label}");
                None
            }
        };
        assert_eq!(
            (
                executor
                    .target_effects
                    .load(std::sync::atomic::Ordering::SeqCst),
                executor
                    .subordinate_shutdowns
                    .load(std::sync::atomic::Ordering::SeqCst),
            ),
            effects_before,
            "known-operation lookup must be read-only: {label}"
        );
        result
    }

    fn current_request(id: &str) -> tokio_tungstenite::tungstenite::Message {
        tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({ "type": "get_current_shutdown", "id": id })
                .to_string()
                .into(),
        )
    }

    async fn b076_current_on_both_surfaces(
        coordinator: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        executor: &PublicShutdownExecutor,
        label: &str,
    ) -> Result<serde_json::Value, serde_json::Value> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let effects_before = (
            executor
                .target_effects
                .load(std::sync::atomic::Ordering::SeqCst),
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
        );
        let tauri = crate::adaptor::controller::command::application_lifecycle::get_application_shutdown_result(
            coordinator.as_ref(),
        )
        .await;
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableShutdownWsDispatcher {
            coordinator,
            exit_codes: Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("authenticated B076 WebSocket");
        socket
            .send(current_request(&format!("b076-{label}")))
            .await
            .unwrap();
        let websocket = response_json(&mut socket).await;
        server.abort();
        let result = match tauri {
            Ok(tauri) => {
                let tauri = serde_json::to_value(tauri).unwrap();
                assert_eq!(websocket["status"], "ok", "{label}: {websocket}");
                assert_eq!(websocket["result"]["type"], "current_shutdown");
                assert_eq!(websocket["result"]["result"], tauri, "{label}");
                Ok(tauri)
            }
            Err(tauri) => {
                let tauri = serde_json::to_value(tauri).unwrap();
                assert_eq!(websocket["status"], "error", "{label}: {websocket}");
                assert_eq!(websocket["error"]["type"], tauri["type"], "{label}");
                assert_eq!(tauri["type"], "internal", "{label}");
                assert!(websocket["error"]["correlation_id"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()));
                Err(tauri)
            }
        };
        assert_eq!(
            (
                executor
                    .target_effects
                    .load(std::sync::atomic::Ordering::SeqCst),
                executor
                    .subordinate_shutdowns
                    .load(std::sync::atomic::Ordering::SeqCst),
            ),
            effects_before,
            "current shutdown lookup must be read-only: {label}"
        );
        result
    }

    #[test]
    fn websocket_permission_response_uses_the_closed_shared_dto() {
        let request: AgentSessionWsRequestV1 = serde_json::from_value(serde_json::json!({
            "type": "request_permission_response",
            "id": "outer-permission-1",
            "request": {
                "operation_id": "permission-operation-1",
                "session_id": "session-1",
                "request_id": "provider-request-1",
                "behavior": "allow",
                "message": null,
                "updated_input": "{\"answers\":{\"Q\":\"A\"}}"
            }
        }))
        .unwrap();
        let AgentSessionWsRequestV1::RequestPermissionResponse { id, request } = request else {
            panic!("permission response route did not deserialize");
        };
        assert_eq!(id, "outer-permission-1");
        assert_eq!(request.operation_id, "permission-operation-1");
        assert_eq!(request.request_id, "provider-request-1");

        let encoded = serde_json::to_value(AgentSessionWsResponseV1::Ok {
            id,
            result: Box::new(AgentSessionWsResultV1::PermissionResponseOutcome {
                outcome: PermissionResponseCommandOutcomeDtoV1::Accepted {
                    operation: PermissionResponseOperationViewDtoV1 {
                        receipt: crate::adaptor::protocol::agent_session_v1::PermissionResponseOperationReceiptDtoV1 {
                            operation_id: "permission-operation-1".to_string(),
                            session_id: "session-1".to_string(),
                            request_id: "provider-request-1".to_string(),
                            input_ref: "permission-input-1".to_string(),
                        },
                        latest_status: crate::adaptor::protocol::agent_session_v1::PermissionResponseExecutionStatusDtoV1::Completed {
                            decision: crate::adaptor::protocol::agent_session_v1::PermissionResponseDecisionDtoV1::Allowed,
                        },
                    },
                },
            }),
        })
        .unwrap();
        assert_eq!(encoded["status"], "ok");
        assert_eq!(encoded["result"]["type"], "permission_response_outcome");
        assert_eq!(encoded["result"]["outcome"]["type"], "accepted");
        assert_eq!(
            encoded["result"]["outcome"]["operation"]["latest_status"]["type"],
            "completed"
        );
    }

    #[test]
    fn b075_websocket_integer_wire_kinds_cover_every_request_field_and_limit() {
        let semantic_integer_pairs = [
            (
                serde_json::json!({
                    "type": "request_stop",
                    "id": "integer-stop",
                    "request": {
                        "request_id": "stop-1",
                        "session_id": "session-1",
                        "turn_id": "1",
                        "expected_session_revision": "0"
                    }
                }),
                serde_json::json!({
                    "type": "request_stop",
                    "id": "integer-stop",
                    "request": {
                        "request_id": "stop-1",
                        "session_id": "session-1",
                        "turn_id": 1,
                        "expected_session_revision": "0"
                    }
                }),
            ),
            (
                serde_json::json!({
                    "type": "request_recovery_action",
                    "id": "integer-recovery-action",
                    "request": {
                        "action_id": "action-1",
                        "obligation_id": "obligation-1",
                        "origin_revision": "0",
                        "action": "read_again"
                    }
                }),
                serde_json::json!({
                    "type": "request_recovery_action",
                    "id": "integer-recovery-action",
                    "request": {
                        "action_id": "action-1",
                        "obligation_id": "obligation-1",
                        "origin_revision": 0,
                        "action": "read_again"
                    }
                }),
            ),
            (
                serde_json::json!({
                    "type": "resolve_shutdown_target",
                    "id": "integer-shutdown-action",
                    "request": {
                        "action_id": "action-1",
                        "shutdown_id": "plan-1",
                        "ordinal": "0",
                        "target_key": "target-1",
                        "origin_revision": "0",
                        "action": "read_again"
                    }
                }),
                serde_json::json!({
                    "type": "resolve_shutdown_target",
                    "id": "integer-shutdown-action",
                    "request": {
                        "action_id": "action-1",
                        "shutdown_id": "plan-1",
                        "ordinal": 0,
                        "target_key": "target-1",
                        "origin_revision": "0",
                        "action": "read_again"
                    }
                }),
            ),
            (
                serde_json::json!({
                    "type": "dismiss_feedback",
                    "id": "integer-feedback-dismiss",
                    "session_id": "session-1",
                    "feedback_id": "feedback-1",
                    "expected_revision": "0",
                    "action_id": "dismiss-1"
                }),
                serde_json::json!({
                    "type": "dismiss_feedback",
                    "id": "integer-feedback-dismiss",
                    "session_id": "session-1",
                    "feedback_id": "feedback-1",
                    "expected_revision": 0,
                    "action_id": "dismiss-1"
                }),
            ),
            (
                serde_json::json!({
                    "type": "retry_feedback",
                    "id": "integer-feedback-retry",
                    "session_id": "session-1",
                    "feedback_id": "feedback-1",
                    "expected_revision": "0",
                    "action_id": "retry-1"
                }),
                serde_json::json!({
                    "type": "retry_feedback",
                    "id": "integer-feedback-retry",
                    "session_id": "session-1",
                    "feedback_id": "feedback-1",
                    "expected_revision": 0,
                    "action_id": "retry-1"
                }),
            ),
        ];
        for (string_value, number_value) in semantic_integer_pairs {
            assert!(serde_json::from_value::<AgentSessionWsRequestV1>(string_value).is_ok());
            assert!(serde_json::from_value::<AgentSessionWsRequestV1>(number_value).is_err());
        }

        let limit_templates = [
            serde_json::json!({
                "type": "get_pending_recovery",
                "id": "limit-current-recovery",
                "limit": 1,
                "partition": null,
                "owner": null,
                "shutdown_id": null,
                "cursor": null
            }),
            serde_json::json!({
                "type": "get_pending_recovery_snapshot",
                "id": "limit-recovery-snapshot",
                "shutdown_id": "plan-1",
                "snapshot_id": "snapshot-1",
                "partition": "closed_session",
                "limit": 1,
                "cursor": null
            }),
            serde_json::json!({
                "type": "get_shutdown_plan",
                "id": "limit-shutdown-plan",
                "shutdown_id": "plan-1",
                "limit": 1,
                "cursor": null
            }),
            serde_json::json!({
                "type": "list_feedback",
                "id": "limit-feedback",
                "session_id": "session-1",
                "limit": 1,
                "cursor": null
            }),
        ];
        for mut template in limit_templates {
            for valid in [
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(i64::MAX),
            ] {
                template["limit"] = valid;
                assert!(
                    serde_json::from_value::<AgentSessionWsRequestV1>(template.clone()).is_ok()
                );
            }
            for invalid in [
                serde_json::json!("1"),
                serde_json::json!(-1),
                serde_json::json!(1.0),
            ] {
                template["limit"] = invalid;
                assert!(
                    serde_json::from_value::<AgentSessionWsRequestV1>(template.clone()).is_err()
                );
            }
        }
        let _: usize = MAX_MESSAGE_BYTES;
    }

    async fn response_json<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> serde_json::Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await.unwrap().unwrap() {
                tokio_tungstenite::tungstenite::Message::Text(value) => {
                    return serde_json::from_str(value.as_str()).unwrap();
                }
                tokio_tungstenite::tungstenite::Message::Ping(value) => {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Pong(value))
                        .await
                        .unwrap();
                }
                other => panic!("unexpected WebSocket response: {other:?}"),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b004_b010_authenticated_websocket_and_tauri_converge_on_one_send_and_conflicts() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;
        use tauri::Manager as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository(
            repository,
            store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let gate = Arc::new(ConcurrentPublicSendGate {
            session_store: session_store.clone(),
            planned: Arc::new(tokio::sync::Barrier::new(2)),
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate.clone(),
                store.installation_id().to_string(),
            ),
        );
        let journal = Arc::new(
            crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                store.clone(),
                store.clone(),
                store.installation_id().to_string(),
            ),
        );
        let command = CanonicalSendCommandV1 {
            target:
                crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                    chat_session_id: None,
                    worktree_path: "/tmp/b004-session".to_string(),
                },
            content: "hello".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("codex".to_string()),
            model_id: Some("model-a".to_string()),
            images: vec![crate::usecase::agent_session::session::ImageAttachment {
                data: "base64-image-a".to_string(),
                media_type: "image/png".to_string(),
            }],
            mentions: vec![crate::adaptor::protocol::mention::MentionReferenceInput {
                file_path: "src/a.rs".to_string(),
                start_line: Some(1),
                end_line: Some(2),
            }],
            editor_context: Some(
                crate::usecase::agent_session::runtime::usecase::AgentEditorContext {
                    active_editor_path: Some("src/a.rs".to_string()),
                    open_editor_paths: vec!["src/a.rs".to_string()],
                    selection: Some(
                        crate::usecase::agent_session::runtime::usecase::AgentEditorSelection {
                            file_path: "src/a.rs".to_string(),
                            start_line: 1,
                            end_line: 2,
                        },
                    ),
                },
            ),
        };
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableSendWsDispatcher {
            store: store.clone(),
            send: send.clone(),
            journal: journal.clone(),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut websocket_request = url.as_str().into_client_request().unwrap();
        websocket_request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
            .await
            .expect("authenticated WebSocket connection");

        let operation_id = "s".repeat(128);
        let tauri_store = store.clone();
        let tauri_send = send.clone();
        let tauri_journal = journal.clone();
        let tauri_command = command.clone();
        let tauri_operation_id = operation_id.clone();
        let tauri = tokio::spawn(async move {
            crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                tauri_store.as_ref(),
                tauri_send.as_ref(),
                tauri_journal.as_ref(),
                tauri_operation_id,
                tauri_command,
            )
            .await
        });
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_send",
                    "id": "b004-websocket-request",
                    "operation_id": operation_id,
                    "command": command.clone(),
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        let websocket = response_json(&mut socket).await;
        let tauri = tauri.await.unwrap().expect("Tauri send result");
        assert_eq!(websocket["status"], "ok");
        assert_eq!(websocket["result"]["type"], "send_outcome");
        assert_eq!(
            websocket["result"]["outcome"],
            serde_json::to_value(&tauri).unwrap(),
            "both public surfaces must expose the same receipt, disposition, and status"
        );
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);

        let (_, page, _) = session_store
            .get_session_with_latest_page(data.path(), "b004-session", 32)
            .unwrap()
            .expect("one canonical session projection");
        assert_eq!(
            page.messages
                .iter()
                .filter(|message| {
                    message.id == "b004-human"
                        && message.role
                            == crate::usecase::agent_session::session::MessageRole::Human
                })
                .count(),
            1
        );
        let stream = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session("b004-session")
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
                                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted { turn_id: 1, .. }
                            )
                        )
                ))
                .count(),
            1
        );
        let attempts = journal
            .pending_page_for_scope(LOCAL_API_OPERATION_PRINCIPAL, "application", 8, None)
            .await
            .unwrap()
            .entries;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].caller_request_id, operation_id);
        assert_eq!(
            attempts[0].operation_id.as_deref(),
            Some(operation_id.as_str())
        );
        assert_eq!(
            attempts[0].resolution,
            crate::domain::local_event::CallerAttemptResolution::Accepted
        );

        let app = tauri::test::mock_builder()
            .manage(send.clone())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let tauri_query = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation(
            app.state::<Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>>(),
            operation_id.clone(),
        )
        .await
        .unwrap();
        let original_operation = serde_json::to_value(&tauri_query).unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_send",
                    "id": "b072-websocket-send-query",
                    "operation_id": operation_id,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_query = response_json(&mut socket).await;
        assert_eq!(websocket_query["status"], "ok");
        assert_eq!(websocket_query["result"]["type"], "send_operation");
        assert_eq!(
            websocket_query["result"]["operation"], original_operation,
            "Tauri and authenticated WebSocket identity queries must present the same operation"
        );
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);

        let conflicts = [
            ("content", {
                let mut changed = command.clone();
                changed.content = "changed content".to_string();
                changed
            }),
            ("image.data", {
                let mut changed = command.clone();
                changed.images[0].data = "base64-image-b".to_string();
                changed
            }),
            ("image.media_type", {
                let mut changed = command.clone();
                changed.images[0].media_type = "image/jpeg".to_string();
                changed
            }),
            ("mention.file_path", {
                let mut changed = command.clone();
                changed.mentions[0].file_path = "src/b.rs".to_string();
                changed
            }),
            ("mention.start_line", {
                let mut changed = command.clone();
                changed.mentions[0].start_line = Some(3);
                changed
            }),
            ("mention.end_line", {
                let mut changed = command.clone();
                changed.mentions[0].end_line = Some(3);
                changed
            }),
            ("editor_context.active_editor_path", {
                let mut changed = command.clone();
                changed.editor_context.as_mut().unwrap().active_editor_path =
                    Some("src/b.rs".to_string());
                changed
            }),
            ("editor_context.open_editor_paths", {
                let mut changed = command.clone();
                changed.editor_context.as_mut().unwrap().open_editor_paths =
                    vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
                changed
            }),
            ("editor_context.selection.file_path", {
                let mut changed = command.clone();
                changed
                    .editor_context
                    .as_mut()
                    .unwrap()
                    .selection
                    .as_mut()
                    .unwrap()
                    .file_path = "src/b.rs".to_string();
                changed
            }),
            ("editor_context.selection.start_line", {
                let mut changed = command.clone();
                changed
                    .editor_context
                    .as_mut()
                    .unwrap()
                    .selection
                    .as_mut()
                    .unwrap()
                    .start_line = 2;
                changed
            }),
            ("editor_context.selection.end_line", {
                let mut changed = command.clone();
                changed
                    .editor_context
                    .as_mut()
                    .unwrap()
                    .selection
                    .as_mut()
                    .unwrap()
                    .end_line = 3;
                changed
            }),
            ("target.chat_session_id", {
                let mut changed = command.clone();
                changed.target = crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                    chat_session_id: Some("another-session".to_string()),
                    worktree_path: "/tmp/b004-session".to_string(),
                };
                changed
            }),
            ("target.worktree_path", {
                let mut changed = command.clone();
                changed.target = crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                    chat_session_id: None,
                    worktree_path: "/tmp/b004-other-worktree".to_string(),
                };
                changed
            }),
            ("target.variant", {
                let mut changed = command.clone();
                changed.target = crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::WorkflowApproval {
                    execution_id: "another-execution".to_string(),
                };
                changed
            }),
            ("configuration.permission_mode", {
                let mut changed = command.clone();
                changed.permission_mode = "edit".to_string();
                changed
            }),
            ("configuration.plan_mode", {
                let mut changed = command.clone();
                changed.plan_mode = true;
                changed
            }),
            ("configuration.backend_id", {
                let mut changed = command.clone();
                changed.backend_id = Some("claude".to_string());
                changed
            }),
            ("configuration.model_id", {
                let mut changed = command.clone();
                changed.model_id = Some("model-b".to_string());
                changed
            }),
        ];
        for (ordinal, (field, changed)) in conflicts.into_iter().enumerate() {
            let tauri_conflict =
                crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                    store.as_ref(),
                    send.as_ref(),
                    journal.as_ref(),
                    operation_id.clone(),
                    changed.clone(),
                )
                .await
                .expect_err("changed bound field must conflict on Tauri");
            assert_eq!(
                serde_json::to_value(tauri_conflict).unwrap()["type"],
                "payload_conflict",
                "field={field}"
            );
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "request_send",
                        "id": format!("b010-websocket-{ordinal}"),
                        "operation_id": operation_id,
                        "command": changed,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let websocket_conflict = response_json(&mut socket).await;
            assert_eq!(websocket_conflict["status"], "error", "field={field}");
            assert_eq!(
                websocket_conflict["error"]["type"], "payload_conflict",
                "field={field}"
            );
        }
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        let operation_after_conflicts = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation(
            app.state::<Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>>(),
            operation_id.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(operation_after_conflicts).unwrap(),
            original_operation,
            "all conflicts must leave the immutable receipt and latest status unchanged"
        );
        let (_, page_after_conflicts, _) = session_store
            .get_session_with_latest_page(data.path(), "b004-session", 32)
            .unwrap()
            .unwrap();
        assert_eq!(
            page_after_conflicts
                .messages
                .iter()
                .filter(|message| {
                    message.id == "b004-human"
                        && message.role
                            == crate::usecase::agent_session::session::MessageRole::Human
                })
                .count(),
            1
        );
        let stream_after_conflicts = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::agent_session("b004-session")
                    .unwrap(),
                after: None,
                limit: 32,
            })
            .await
            .unwrap();
        assert_eq!(
            stream_after_conflicts
                .events
                .iter()
                .filter(|event| matches!(
                    &event.event,
                    crate::domain::local_event::LoadedDomainEvent::Known(event)
                        if matches!(
                            event.as_ref(),
                            crate::domain::local_event::LocalDomainEvent::AgentSession(
                                crate::domain::agent_session::events::AgentSessionDomainEvent::SendOperationAccepted { .. }
                                    | crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted { .. }
                            )
                        )
                ))
                .count(),
            2,
            "one acceptance fact and one turn start must remain after all conflicts"
        );

        let invalid_tauri =
            crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                store.as_ref(),
                send.as_ref(),
                journal.as_ref(),
                "bad/id".to_string(),
                command.clone(),
            )
            .await
            .expect_err("Tauri must reject an invalid send identity");
        assert_eq!(
            serde_json::to_value(invalid_tauri).unwrap()["type"],
            "invalid_request"
        );
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_send",
                    "id": "b009-invalid-send",
                    "operation_id": "bad/id",
                    "command": command,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let invalid_websocket = response_json(&mut socket).await;
        assert_eq!(invalid_websocket["status"], "error");
        assert_eq!(invalid_websocket["error"]["type"], "invalid_request");
        assert_eq!(
            gate.effects.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "invalid public identities must not start another effect"
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b058_b072_b087_b088_tauri_and_authenticated_websocket_shutdown_surfaces_match() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let executor = Arc::new(PublicShutdownExecutor::default());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        let authority: Arc<
            dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
        > = store.clone();
        let coordinator = Arc::new(
            crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                repository,
                authority,
                executor.clone(),
                store.installation_id().to_string(),
                store.process_instance_id().to_string(),
            ),
        );
        let exit_codes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableShutdownWsDispatcher {
            coordinator: coordinator.clone(),
            exit_codes: exit_codes.clone(),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut websocket_request = url.as_str().into_client_request().unwrap();
        websocket_request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
            .await
            .expect("authenticated shutdown WebSocket");

        let request_id = "q".repeat(128);
        let request = ApplicationQuitRequestDtoV1 {
            request_id: request_id.clone(),
            intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                code: 17,
            },
        };
        let (tauri_outcome, tauri_exit_code) =
            crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
                coordinator.as_ref(),
                request.clone(),
            )
            .await
            .expect("Tauri adapter accepts the 128-byte quit identity");
        assert_eq!(
            tauri_exit_code,
            Some(crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit { code: 17 })
        );
        let tauri_outcome = serde_json::to_value(&tauri_outcome).unwrap();
        let operation_id = tauri_outcome["receipt"]["operation_id"]
            .as_str()
            .unwrap()
            .to_string();
        let shutdown_id = tauri_outcome["receipt"]["shutdown_id"]
            .as_str()
            .unwrap()
            .to_string();

        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_application_quit",
                    "id": "b072-quit-replay",
                    "request": request,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_outcome = response_json(&mut socket).await;
        assert_eq!(websocket_outcome["status"], "ok");
        assert_eq!(
            websocket_outcome["result"]["type"],
            "application_quit_outcome"
        );
        assert_eq!(websocket_outcome["result"]["outcome"], tauri_outcome);
        assert_eq!(
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "cross-surface replay must not start a second shutdown flight"
        );

        let tauri_lookup = crate::adaptor::controller::command::application_lifecycle::get_application_quit_operation_result(
            coordinator.as_ref(),
            operation_id.clone(),
        )
        .await
        .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_application_quit",
                    "id": "b072-known-quit",
                    "operation_id": operation_id,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_lookup = response_json(&mut socket).await;
        assert_eq!(websocket_lookup["status"], "ok");
        assert_eq!(
            websocket_lookup["result"]["lookup"],
            serde_json::to_value(tauri_lookup).unwrap()
        );

        let tauri_current = crate::adaptor::controller::command::application_lifecycle::get_application_shutdown_result(
            coordinator.as_ref(),
        )
        .await
        .unwrap();
        socket
            .send(current_request("b072-current-shutdown"))
            .await
            .unwrap();
        let websocket_current = response_json(&mut socket).await;
        assert_eq!(websocket_current["status"], "ok");
        assert_eq!(
            websocket_current["result"]["result"],
            serde_json::to_value(tauri_current).unwrap()
        );

        let tauri_page =
            crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
                coordinator.as_ref(),
                shutdown_id.clone(),
                Some(128),
                None,
            )
            .await
            .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_shutdown_plan",
                    "id": "b072-shutdown-page",
                    "shutdown_id": shutdown_id,
                    "limit": 128,
                    "cursor": null,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_page = response_json(&mut socket).await;
        assert_eq!(websocket_page["status"], "ok");
        assert_eq!(
            websocket_page["result"]["page"],
            serde_json::to_value(tauri_page).unwrap()
        );

        let conflicting = ApplicationQuitRequestDtoV1 {
            request_id: request_id.clone(),
            intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Restart {
                code: 99,
            },
        };
        let tauri_conflict = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            coordinator.as_ref(),
            conflicting.clone(),
        )
        .await
        .expect_err("Tauri changed-payload replay must conflict");
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_application_quit",
                    "id": "b058-websocket-conflict",
                    "request": conflicting,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_conflict = response_json(&mut socket).await;
        assert_eq!(websocket_conflict["status"], "error");
        assert_eq!(
            websocket_conflict["error"],
            serde_json::to_value(tauri_conflict).unwrap()
        );

        let tauri_invalid = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            coordinator.as_ref(),
            ApplicationQuitRequestDtoV1 {
                request_id: String::new(),
                intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                    code: 0,
                },
            },
        )
        .await
        .expect_err("Tauri invalid identity must fail before admission");
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_application_quit",
                    "id": "b087-websocket-invalid",
                    "request": {
                        "request_id": "bad/id",
                        "intent": { "type": "exit", "code": 0 }
                    },
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_invalid = response_json(&mut socket).await;
        assert_eq!(websocket_invalid["status"], "error");
        assert_eq!(
            websocket_invalid["error"]["type"],
            serde_json::to_value(tauri_invalid).unwrap()["type"]
        );
        assert_eq!(
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            executor
                .target_effects
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );

        let one_byte = ApplicationQuitRequestDtoV1 {
            request_id: "a".to_string(),
            intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                code: -3,
            },
        };
        let (tauri_one_byte, _) = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            coordinator.as_ref(),
            one_byte.clone(),
        )
        .await
        .expect("Tauri accepts a one-byte quit identity");
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_application_quit",
                    "id": "b087-websocket-one-byte",
                    "request": one_byte,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_one_byte = response_json(&mut socket).await;
        assert_eq!(websocket_one_byte["status"], "ok");
        assert_eq!(
            websocket_one_byte["result"]["outcome"],
            serde_json::to_value(tauri_one_byte).unwrap()
        );
        assert_eq!(
            executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        assert_eq!(
            exit_codes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[17, -3]
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b087_quit_identity_matrix_is_exact_on_tauri_and_authenticated_websocket() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        #[derive(Clone, Copy, Debug)]
        enum Surface {
            Tauri,
            WebSocket,
        }

        let cases = [
            ("one-byte", "a".to_string(), true),
            (
                "one-hundred-twenty-eight-bytes",
                "A0._:-".repeat(21) + "Aa",
                true,
            ),
            ("empty", String::new(), false),
            ("one-hundred-twenty-nine-bytes", "a".repeat(129), false),
            ("non-ascii", "非ascii".to_string(), false),
            ("forbidden-ascii", "bad/id".to_string(), false),
        ];
        assert_eq!(cases[1].1.len(), 128);

        for surface in [Surface::Tauri, Surface::WebSocket] {
            for (ordinal, (label, request_id, valid)) in cases.iter().enumerate() {
                let data = tempfile::tempdir().unwrap();
                let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                    crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                        data.path().to_path_buf(),
                    ),
                )
                .unwrap();
                let executor = Arc::new(PublicShutdownExecutor::default());
                let repository: Arc<
                    dyn crate::domain::local_event::LocalEventTransactionRepository,
                > = store.clone();
                let authority: Arc<
                    dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
                > = store.clone();
                let coordinator = Arc::new(
                    crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                        repository,
                        authority,
                        executor.clone(),
                        store.installation_id().to_string(),
                        store.process_instance_id().to_string(),
                    ),
                );
                let request = ApplicationQuitRequestDtoV1 {
                    request_id: request_id.clone(),
                    intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                        code: 0,
                    },
                };

                match surface {
                    Surface::Tauri => {
                        let result = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
                            coordinator.as_ref(),
                            request,
                        )
                        .await;
                        if *valid {
                            let (outcome, exit_code) = result.expect("valid Tauri quit identity");
                            assert_eq!(
                                serde_json::to_value(outcome).unwrap()["type"],
                                "accepted",
                                "{surface:?}/{label}"
                            );
                            assert_eq!(
                                exit_code,
                                Some(crate::usecase::shutdown_coordinator::ApplicationProcessAction::Exit { code: 0 })
                            );
                        } else {
                            let error = result.expect_err("invalid Tauri quit identity");
                            assert_eq!(
                                serde_json::to_value(error).unwrap()["type"],
                                "invalid_request",
                                "{surface:?}/{label}"
                            );
                        }
                    }
                    Surface::WebSocket => {
                        let exit_codes = Arc::new(std::sync::Mutex::new(Vec::new()));
                        let dispatcher: Arc<dyn WsDispatchService> =
                            Arc::new(DurableShutdownWsDispatcher {
                                coordinator: coordinator.clone(),
                                exit_codes: exit_codes.clone(),
                            });
                        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
                        let mut websocket_request = url.as_str().into_client_request().unwrap();
                        websocket_request
                            .headers_mut()
                            .insert("authorization", "Bearer b004-token".parse().unwrap());
                        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
                            .await
                            .expect("authenticated quit-identity WebSocket");
                        socket
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "type": "request_application_quit",
                                    "id": format!("b087-quit-{ordinal}"),
                                    "request": request,
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                        let response = response_json(&mut socket).await;
                        server.abort();
                        if *valid {
                            assert_eq!(response["status"], "ok", "{surface:?}/{label}");
                            assert_eq!(response["result"]["type"], "application_quit_outcome");
                            assert_eq!(response["result"]["outcome"]["type"], "accepted");
                            assert_eq!(
                                exit_codes
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .as_slice(),
                                &[0]
                            );
                        } else {
                            assert_eq!(response["status"], "error", "{surface:?}/{label}");
                            assert_eq!(response["error"]["type"], "invalid_request");
                            assert!(exit_codes
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .is_empty());
                        }
                    }
                }

                assert_eq!(
                    executor
                        .subordinate_shutdowns
                        .load(std::sync::atomic::Ordering::SeqCst),
                    usize::from(*valid),
                    "{surface:?}/{label}"
                );
                assert_eq!(
                    executor
                        .target_effects
                        .load(std::sync::atomic::Ordering::SeqCst),
                    0,
                    "{surface:?}/{label}"
                );
                if !valid {
                    let current = crate::adaptor::controller::command::application_lifecycle::get_application_shutdown_result(
                        coordinator.as_ref(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(
                        serde_json::to_value(current).unwrap(),
                        serde_json::json!({ "type": "current", "plan": null }),
                        "invalid identity must not anchor shutdown state: {surface:?}/{label}"
                    );
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b088_known_quit_closed_variant_matrix_matches_tauri_and_authenticated_websocket() {
        let available = PublicQuitFixture::completed("b088-available").await;
        b088_assert_lookup_on_both_surfaces(
            available.coordinator.clone(),
            available.executor.as_ref(),
            &available.operation_id,
            "found",
            Some("completed"),
            "available-live",
        )
        .await
        .expect("available live normal projection");

        let compacted = PublicQuitFixture::completed("b088-compacted").await;
        compacted.compact_and_wait_for_cleanup().await;
        let compacted_live = b088_assert_lookup_on_both_surfaces(
            compacted.coordinator.clone(),
            compacted.executor.as_ref(),
            &compacted.operation_id,
            "found",
            Some("completed"),
            "compacted-live",
        )
        .await
        .expect("compacted live normal projection");
        assert_eq!(compacted_live["state"]["type"], "completed");

        let unissued_id = "f".repeat(64);
        assert_ne!(unissued_id, available.operation_id);
        b088_assert_lookup_on_both_surfaces(
            available.coordinator.clone(),
            available.executor.as_ref(),
            &unissued_id,
            "not_found",
            None,
            "unissued-no-fallback",
        )
        .await;

        let missing_operation = PublicQuitFixture::completed("b088-missing-operation").await;
        missing_operation
            .raw_connection()
            .execute(
                "DELETE FROM operation_records WHERE kind = 'application_quit' AND operation_id = ?1",
                rusqlite::params![missing_operation.operation_id],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            missing_operation.coordinator.clone(),
            missing_operation.executor.as_ref(),
            &missing_operation.operation_id,
            "internal",
            None,
            "missing-operation-authority",
        )
        .await;

        let missing_plan = PublicQuitFixture::completed("b088-missing-plan").await;
        missing_plan
            .raw_connection()
            .execute(
                "DELETE FROM shutdown_plans WHERE shutdown_id = ?1",
                rusqlite::params![missing_plan.plan.shutdown_id],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            missing_plan.coordinator.clone(),
            missing_plan.executor.as_ref(),
            &missing_plan.operation_id,
            "internal",
            None,
            "missing-plan-reference",
        )
        .await;

        let receipt_decode = PublicQuitFixture::completed("b088-receipt-decode").await;
        receipt_decode
            .raw_connection()
            .execute(
                "UPDATE operation_records SET receipt = 'not-json' WHERE kind = 'application_quit' AND operation_id = ?1",
                rusqlite::params![receipt_decode.operation_id],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            receipt_decode.coordinator.clone(),
            receipt_decode.executor.as_ref(),
            &receipt_decode.operation_id,
            "internal",
            None,
            "receipt-decode-failure",
        )
        .await;

        let status_decode = PublicQuitFixture::completed("b088-status-decode").await;
        status_decode
            .raw_connection()
            .execute(
                "UPDATE operation_records SET latest_status = '{\"schema\":\"wrong\",\"state\":{\"type\":\"completed\"}}' WHERE kind = 'application_quit' AND operation_id = ?1",
                rusqlite::params![status_decode.operation_id],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            status_decode.coordinator.clone(),
            status_decode.executor.as_ref(),
            &status_decode.operation_id,
            "internal",
            None,
            "status-decode-failure",
        )
        .await;

        let binding_integrity = PublicQuitFixture::completed("b088-binding-integrity").await;
        binding_integrity
            .raw_connection()
            .execute(
                "UPDATE operation_bindings SET binding_hmac = zeroblob(32)
                 WHERE installation_id = ?1 AND kind = 'application_quit' AND operation_id = ?2",
                rusqlite::params![
                    binding_integrity.store.installation_id(),
                    binding_integrity.operation_id
                ],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            binding_integrity.coordinator.clone(),
            binding_integrity.executor.as_ref(),
            &binding_integrity.operation_id,
            "internal",
            None,
            "binding-integrity-mismatch",
        )
        .await;

        let accepted_inner = PublicQuitFixture::completed("b088-accepted-inner-unknown").await;
        let unknown_status = serde_json::json!({
            "schema": "application_quit_status_v1",
            "state": {
                "type": "outcome_unknown",
                "operation_id": accepted_inner.operation_id,
                "shutdown_id": accepted_inner.plan.shutdown_id,
                "activation_commit_id": "a".repeat(64),
            }
        })
        .to_string();
        accepted_inner
            .raw_connection()
            .execute(
                "UPDATE operation_records SET latest_status = ?1
                 WHERE kind = 'application_quit' AND operation_id = ?2",
                rusqlite::params![unknown_status, accepted_inner.operation_id],
            )
            .unwrap();
        b088_assert_lookup_on_both_surfaces(
            accepted_inner.coordinator.clone(),
            accepted_inner.executor.as_ref(),
            &accepted_inner.operation_id,
            "found",
            Some("outcome_unknown"),
            "accepted-inner-outcome-unknown",
        )
        .await
        .expect("accepted-inner OutcomeUnknown remains a normal-operation variant");

        let unknown_data = tempfile::tempdir().unwrap();
        let unknown_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                unknown_data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let unknown_executor = Arc::new(PublicShutdownExecutor::default());
        let unknown_coordinator = Arc::new(
            crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                unknown_store.clone(),
                unknown_store.clone(),
                unknown_executor.clone(),
                unknown_store.installation_id().to_string(),
                unknown_store.process_instance_id().to_string(),
            ),
        );
        unknown_coordinator.set_pre_acceptance_hook(Arc::new({
            let fault = unknown_store.fault_injector().clone();
            move || {
                let fault = fault.clone();
                Box::pin(async move {
                    fault.arm_crash_after_commit_before_readback();
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }
        }));
        let (top_unknown, exit_code) = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            unknown_coordinator.as_ref(),
            ApplicationQuitRequestDtoV1 {
                request_id: "b088-top-unknown".to_string(),
                intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit {
                    code: 9,
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(exit_code, None);
        let top_unknown = serde_json::to_value(top_unknown).unwrap();
        assert_eq!(top_unknown["type"], "outcome_unknown");
        let top_unknown_operation = top_unknown["operation_id"].as_str().unwrap();
        b088_assert_lookup_on_both_surfaces(
            unknown_coordinator,
            unknown_executor.as_ref(),
            top_unknown_operation,
            "outcome_unknown",
            None,
            "top-level-acceptance-outcome-unknown",
        )
        .await
        .expect("top-level OutcomeUnknown lookup");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b096_available_to_compacted_plan_is_monotonic_on_tauri_and_authenticated_websocket() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let fixture = PublicQuitFixture::completed("b096-public-compaction").await;
        let shutdown_id = fixture.plan.shutdown_id.clone();
        let available =
            crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
                fixture.coordinator.as_ref(),
                shutdown_id.clone(),
                Some(128),
                None,
            )
            .await
            .expect("available terminal plan through the Tauri adapter");
        let available = serde_json::to_value(available).unwrap();
        assert_eq!(available["plan"]["details_state"], "available");

        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableShutdownWsDispatcher {
            coordinator: fixture.coordinator.clone(),
            exit_codes: Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("authenticated B096 WebSocket");

        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_shutdown_plan",
                    "id": "b096-available",
                    "shutdown_id": shutdown_id,
                    "limit": 128,
                    "cursor": null,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_available = response_json(&mut socket).await;
        assert_eq!(websocket_available["status"], "ok");
        assert_eq!(websocket_available["result"]["page"], available);

        fixture.compact_and_wait_for_cleanup().await;
        let invariant_fields = [
            "shutdown_id",
            "phase",
            "operation_id",
            "intent",
            "exit_code",
            "t0_ms",
            "preparation_cutoff_ms",
            "deadline_ms",
            "target_count",
            "prepared_count",
            "effect_reserved_count",
            "terminal_count",
            "completed_count",
            "unresolved_count",
            "recovery_snapshot_count",
            "recovery_snapshot_id",
            "outcome",
            "safe_failure",
        ];
        for repetition in 0..3 {
            let tauri = crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
                fixture.coordinator.as_ref(),
                fixture.plan.shutdown_id.clone(),
                Some(128),
                None,
            )
            .await
            .expect("compacted terminal plan through the Tauri adapter");
            let tauri = serde_json::to_value(tauri).unwrap();
            assert_eq!(tauri["plan"]["details_state"], "compacted");
            assert_eq!(tauri["targets"], serde_json::json!([]));
            assert_eq!(tauri["next_cursor"], serde_json::Value::Null);
            for field in invariant_fields {
                assert_eq!(
                    tauri["plan"][field], available["plan"][field],
                    "compaction must preserve {field}"
                );
            }

            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "get_shutdown_plan",
                        "id": format!("b096-compacted-{repetition}"),
                        "shutdown_id": fixture.plan.shutdown_id,
                        "limit": 128,
                        "cursor": null,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let websocket = response_json(&mut socket).await;
            assert_eq!(websocket["status"], "ok");
            assert_eq!(websocket["result"]["page"], tauri);
        }
        assert_eq!(
            fixture
                .executor
                .target_effects
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            fixture
                .executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b076_current_shutdown_closed_variants_match_tauri_and_authenticated_websocket() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let (_empty_data, _empty_store, empty_coordinator, empty_executor) =
            b076_empty_public_fixture().await;
        let empty = b076_current_on_both_surfaces(
            empty_coordinator,
            empty_executor.as_ref(),
            "no-shutdown",
        )
        .await
        .expect("empty current shutdown projection");
        assert_eq!(
            empty,
            serde_json::json!({ "type": "current", "plan": null })
        );

        let phases = PublicQuitFixture::completed("b076-public-phase-matrix").await;
        let same_boot_id = phases.store.process_instance_id().to_string();
        for (stored_phase, operation_state, public_phase) in [
            ("prepared", "preparing", "preparing"),
            ("activated", "activated", "activated"),
            ("quiescing", "activated", "quiescing"),
            ("completed", "completed", "completed"),
            ("failed", "failed_before_activation", "failed"),
            ("cancelled", "failed_before_activation", "cancelled"),
            (
                "reconciliation_required",
                "reconciliation_required",
                "reconciliation_required",
            ),
        ] {
            phases.set_current_phase(stored_phase, operation_state, &same_boot_id);
            let current = b076_current_on_both_surfaces(
                phases.coordinator.clone(),
                phases.executor.as_ref(),
                &format!("same-boot-{stored_phase}"),
            )
            .await
            .unwrap_or_else(|error| panic!("same-boot {stored_phase} must be public: {error}"));
            assert_eq!(current["type"], "current", "{stored_phase}");
            assert_eq!(
                current["plan"]["shutdown_id"], phases.plan.shutdown_id,
                "{stored_phase}"
            );
            assert_eq!(current["plan"]["phase"], public_phase, "{stored_phase}");
        }

        let previous_nonterminal =
            PublicQuitFixture::completed("b076-public-previous-nonterminal").await;
        previous_nonterminal.set_current_phase("quiescing", "activated", "b076-previous-boot");
        let previous_nonterminal_current = b076_current_on_both_surfaces(
            previous_nonterminal.coordinator.clone(),
            previous_nonterminal.executor.as_ref(),
            "previous-nonterminal",
        )
        .await
        .expect("previous-boot nonterminal projection");
        assert_eq!(
            previous_nonterminal_current["plan"]["shutdown_id"],
            previous_nonterminal.plan.shutdown_id
        );
        assert_eq!(
            previous_nonterminal_current["plan"]["phase"],
            "reconciliation_required"
        );
        assert_eq!(
            previous_nonterminal_current["plan"]["safe_failure"]["kind"],
            "deadline_exceeded"
        );

        let completed = PublicQuitFixture::completed("b076-public-completed").await;
        let same_boot = b076_current_on_both_surfaces(
            completed.coordinator.clone(),
            completed.executor.as_ref(),
            "same-boot-completed",
        )
        .await
        .expect("same-boot completed current projection");
        assert_eq!(same_boot["type"], "current");
        assert_eq!(same_boot["plan"]["shutdown_id"], completed.plan.shutdown_id);
        assert_eq!(same_boot["plan"]["phase"], "completed");

        let restarted = Arc::new(
            crate::usecase::shutdown_coordinator::ShutdownCoordinator::new(
                completed.store.clone(),
                completed.store.clone(),
                completed.executor.clone(),
                completed.store.installation_id().to_string(),
                "b076-restarted-boot".to_string(),
            ),
        );
        let previous_terminal = b076_current_on_both_surfaces(
            restarted.clone(),
            completed.executor.as_ref(),
            "previous-terminal",
        )
        .await
        .expect("previous terminal current projection");
        assert_eq!(
            previous_terminal,
            serde_json::json!({ "type": "current", "plan": null })
        );
        let tauri_history =
            crate::adaptor::controller::command::application_lifecycle::get_shutdown_plan_result(
                restarted.as_ref(),
                completed.plan.shutdown_id.clone(),
                Some(128),
                None,
            )
            .await
            .expect("previous terminal exact history through Tauri");
        let tauri_history = serde_json::to_value(tauri_history).unwrap();
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableShutdownWsDispatcher {
            coordinator: restarted,
            exit_codes: Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("authenticated B076 history WebSocket");
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "get_shutdown_plan",
                    "id": "b076-previous-terminal-history",
                    "shutdown_id": completed.plan.shutdown_id,
                    "limit": 128,
                    "cursor": null,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let websocket_history = response_json(&mut socket).await;
        server.abort();
        assert_eq!(websocket_history["status"], "ok");
        assert_eq!(websocket_history["result"]["page"], tauri_history);
        assert_eq!(tauri_history["plan"]["phase"], "completed");

        let (_unknown_data, unknown_store, unknown_coordinator, unknown_executor) =
            b076_empty_public_fixture().await;
        unknown_coordinator.set_pre_acceptance_hook(Arc::new({
            let fault = unknown_store.fault_injector().clone();
            move || {
                let fault = fault.clone();
                Box::pin(async move {
                    fault.arm_crash_after_commit_before_readback();
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }
        }));
        let (unknown_outcome, unknown_exit_code) = crate::adaptor::controller::command::application_lifecycle::request_application_quit_result(
            unknown_coordinator.as_ref(),
            ApplicationQuitRequestDtoV1 {
                request_id: "b076-public-first-writer-unknown".to_string(),
                intent: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Restart {
                    code: 76,
                },
            },
        )
        .await
        .expect("B076 first-writer unknown request");
        assert_eq!(unknown_exit_code, None);
        let unknown_outcome = serde_json::to_value(unknown_outcome).unwrap();
        assert_eq!(unknown_outcome["type"], "outcome_unknown");
        let unknown_current = b076_current_on_both_surfaces(
            unknown_coordinator,
            unknown_executor.as_ref(),
            "first-writer-outcome-unknown",
        )
        .await
        .expect("first-writer unknown current projection");
        assert_eq!(unknown_current["type"], "outcome_unknown");
        assert_eq!(
            unknown_current["operation_id"],
            unknown_outcome["operation_id"]
        );
        assert_eq!(
            unknown_current["intent"],
            serde_json::json!({ "type": "restart", "code": 76 })
        );

        let mismatch = PublicQuitFixture::completed("b076-public-mismatch").await;
        mismatch
            .raw_connection()
            .execute(
                "UPDATE operation_records
                 SET latest_status = '{\"schema\":\"application_quit_status_v1\",\"state\":{\"type\":\"activated\"}}'
                 WHERE kind = 'application_quit' AND operation_id = ?1",
                rusqlite::params![mismatch.operation_id],
            )
            .unwrap();
        let reconciled = b076_current_on_both_surfaces(
            mismatch.coordinator.clone(),
            mismatch.executor.as_ref(),
            "authority-mismatch",
        )
        .await
        .expect("redundant authority mismatch projection");
        assert_eq!(reconciled["plan"]["shutdown_id"], mismatch.plan.shutdown_id);
        assert_eq!(reconciled["plan"]["phase"], "reconciliation_required");
        assert_eq!(
            reconciled["plan"]["safe_failure"]["kind"],
            "shutdown_authority_mismatch"
        );
        assert_eq!(
            mismatch
                .executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        let decode = PublicQuitFixture::completed("b076-public-decode").await;
        decode
            .raw_connection()
            .execute(
                "UPDATE operation_records SET latest_status = 'not-json'
                 WHERE kind = 'application_quit' AND operation_id = ?1",
                rusqlite::params![decode.operation_id],
            )
            .unwrap();
        let internal = b076_current_on_both_surfaces(
            decode.coordinator.clone(),
            decode.executor.as_ref(),
            "decode-failure",
        )
        .await
        .expect_err("decode failure must be public Internal");
        assert_eq!(internal["type"], "internal");
        assert_eq!(
            decode
                .executor
                .subordinate_shutdowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        for case in [
            "summary-decode",
            "binding-integrity",
            "required-operation-reference",
            "receipt-reference",
            "identity-uniqueness",
        ] {
            let fixture = PublicQuitFixture::completed(&format!("b076-public-{case}")).await;
            let connection = fixture.raw_connection();
            match case {
                "summary-decode" => {
                    connection
                        .execute(
                            "UPDATE shutdown_plans SET summary = 'not-json'
                             WHERE shutdown_id = ?1",
                            rusqlite::params![fixture.plan.shutdown_id],
                        )
                        .unwrap();
                }
                "binding-integrity" => {
                    connection
                        .execute(
                            "UPDATE operation_bindings SET binding_hmac = zeroblob(32)
                             WHERE installation_id = ?1 AND kind = 'application_quit'
                               AND operation_id = ?2",
                            rusqlite::params![
                                fixture.store.installation_id(),
                                fixture.operation_id
                            ],
                        )
                        .unwrap();
                }
                "required-operation-reference" => {
                    connection
                        .execute(
                            "DELETE FROM operation_records
                             WHERE kind = 'application_quit' AND operation_id = ?1",
                            rusqlite::params![fixture.operation_id],
                        )
                        .unwrap();
                }
                "receipt-reference" => {
                    let receipt: String = connection
                        .query_row(
                            "SELECT receipt FROM operation_records
                             WHERE kind = 'application_quit' AND operation_id = ?1",
                            rusqlite::params![fixture.operation_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
                    receipt["shutdown_id"] =
                        serde_json::Value::String("b076-different-plan".to_string());
                    connection
                        .execute(
                            "UPDATE operation_records SET receipt = ?1
                             WHERE kind = 'application_quit' AND operation_id = ?2",
                            rusqlite::params![receipt.to_string(), fixture.operation_id],
                        )
                        .unwrap();
                }
                "identity-uniqueness" => {
                    let (binding, commit_id): (Vec<u8>, String) = connection
                        .query_row(
                            "SELECT binding_hmac, commit_id FROM operation_bindings
                             WHERE installation_id = ?1 AND kind = 'application_quit'
                               AND operation_id = ?2 LIMIT 1",
                            rusqlite::params![
                                fixture.store.installation_id(),
                                fixture.operation_id
                            ],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .unwrap();
                    connection
                        .execute(
                            "INSERT INTO operation_bindings
                             (principal, installation_id, kind, caller_request_id,
                              operation_id, binding_hmac, commit_id)
                             VALUES ('b076-duplicate-principal', ?1, 'application_quit',
                                     'b076-duplicate-request', ?2, ?3, ?4)",
                            rusqlite::params![
                                fixture.store.installation_id(),
                                fixture.operation_id,
                                binding,
                                commit_id
                            ],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let internal = b076_current_on_both_surfaces(
                fixture.coordinator.clone(),
                fixture.executor.as_ref(),
                case,
            )
            .await
            .expect_err("B076 authority failure must be public Internal");
            assert_eq!(internal["type"], "internal", "{case}");
        }

        let (_storage_data, _storage_store, storage_coordinator, storage_executor) =
            b076_empty_public_fixture().await;
        let storage_path = crate::adaptor::gateway::local_event_store::layout::StoreLayout::new(
            _storage_data.path(),
        )
        .database_path();
        rusqlite::Connection::open(storage_path)
            .unwrap()
            .execute("DROP TABLE store_metadata", [])
            .unwrap();
        let storage = b076_current_on_both_surfaces(
            storage_coordinator,
            storage_executor.as_ref(),
            "storage-unavailable",
        )
        .await
        .expect_err("B076 storage failure must be public Internal");
        assert_eq!(storage["type"], "internal");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b009_tauri_and_authenticated_websocket_enforce_send_identity_boundaries() {
        use crate::domain::local_event::LocalEventTransactionRepository as _;
        use crate::usecase::agent_session::session::AgentSessionProjectionCodec as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        #[derive(Clone, Copy, Debug)]
        enum Surface {
            Tauri,
            WebSocket,
        }

        let cases = [
            ("one-byte", "a".to_string(), true),
            (
                "one-hundred-twenty-eight-bytes",
                "A0._:-".repeat(21) + "Aa",
                true,
            ),
            ("empty", String::new(), false),
            ("one-hundred-twenty-nine-bytes", "a".repeat(129), false),
            ("non-ascii", "非ascii".to_string(), false),
            ("forbidden-ascii", "bad/id".to_string(), false),
        ];
        assert_eq!(cases[1].1.len(), 128);

        for surface in [Surface::Tauri, Surface::WebSocket] {
            for (ordinal, (label, operation_id, valid)) in cases.iter().enumerate() {
                let data = tempfile::tempdir().unwrap();
                let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                    crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                        data.path().to_path_buf(),
                    ),
                )
                .unwrap();
                let session_store = Arc::new(crate::test_support::build_session_store());
                let repository: Arc<
                    dyn crate::domain::local_event::LocalEventTransactionRepository,
                > = store.clone();
                session_store.set_local_event_repository(
                    repository,
                    store.installation_id().to_string(),
                    Arc::new(
                        crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
                    ),
                );
                let session_id = format!(
                    "b009-{}-{ordinal}",
                    match surface {
                        Surface::Tauri => "tauri",
                        Surface::WebSocket => "websocket",
                    }
                );
                let worktree_path = data.path().to_string_lossy().to_string();
                let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
                    session_id.clone(),
                    &worktree_path,
                    Some("codex".to_string()),
                    crate::domain::agent_session::PermissionMode::Ask,
                    None,
                    false,
                    false,
                    None,
                );
                session.state = crate::usecase::agent_session::session::SessionState::Idle;
                session_store
                    .save_full_session_for_restore(data.path(), &session)
                    .unwrap();
                let before_meta = session_store
                    .get_session_meta(data.path(), &session_id)
                    .unwrap()
                    .unwrap();
                let gate = Arc::new(PublicIdentitySendGate {
                    session_store: session_store.clone(),
                    session_id: session_id.clone(),
                    plan_calls: std::sync::atomic::AtomicUsize::new(0),
                    effects: std::sync::atomic::AtomicUsize::new(0),
                });
                let send = Arc::new(
                    crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                        store.clone(),
                        store.clone(),
                        gate.clone(),
                        store.installation_id().to_string(),
                    ),
                );
                let journal = Arc::new(
                    crate::usecase::agent_session::operation::CallerAttemptJournal::new(
                        store.clone(),
                        store.clone(),
                        store.installation_id().to_string(),
                    ),
                );
                let command = CanonicalSendCommandV1 {
                    target: crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                        chat_session_id: Some(session_id.clone()),
                        worktree_path: worktree_path.clone(),
                    },
                    content: "identity boundary".to_string(),
                    permission_mode: "ask".to_string(),
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                };

                let accepted = match surface {
                    Surface::Tauri => {
                        let result = crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                            store.as_ref(),
                            send.as_ref(),
                            journal.as_ref(),
                            operation_id.clone(),
                            command.clone(),
                        )
                        .await;
                        if *valid {
                            assert!(
                                matches!(result, Ok(SendCommandOutcomeDtoV1::Accepted { .. })),
                                "{surface:?}/{label}: {result:?}"
                            );
                            true
                        } else {
                            let error = result.expect_err("invalid Tauri identity must fail");
                            assert_eq!(
                                serde_json::to_value(error).unwrap()["type"],
                                "invalid_request",
                                "{surface:?}/{label}"
                            );
                            false
                        }
                    }
                    Surface::WebSocket => {
                        let dispatcher: Arc<dyn WsDispatchService> =
                            Arc::new(DurableSendWsDispatcher {
                                store: store.clone(),
                                send: send.clone(),
                                journal: journal.clone(),
                            });
                        let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
                        let mut request = url.as_str().into_client_request().unwrap();
                        request
                            .headers_mut()
                            .insert("authorization", "Bearer b004-token".parse().unwrap());
                        let (mut socket, _) = tokio_tungstenite::connect_async(request)
                            .await
                            .expect("authenticated identity-boundary WebSocket");
                        socket
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::json!({
                                    "type": "request_send",
                                    "id": format!("b009-{ordinal}"),
                                    "operation_id": operation_id,
                                    "command": command,
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                        let response = response_json(&mut socket).await;
                        server.abort();
                        if *valid {
                            assert_eq!(response["status"], "ok", "{surface:?}/{label}: {response}");
                            assert_eq!(response["result"]["type"], "send_outcome");
                            assert_eq!(response["result"]["outcome"]["type"], "accepted");
                            true
                        } else {
                            assert_eq!(
                                response["status"], "error",
                                "{surface:?}/{label}: {response}"
                            );
                            assert_eq!(response["error"]["type"], "invalid_request");
                            false
                        }
                    }
                };
                assert_eq!(accepted, *valid);

                let (_, page, _) = session_store
                    .get_session_with_latest_page(data.path(), &session_id, 32)
                    .unwrap()
                    .expect("independent Idle session remains readable");
                let after_meta = session_store
                    .get_session_meta(data.path(), &session_id)
                    .unwrap()
                    .unwrap();
                let stream = store
                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                        stream_id: crate::domain::local_event::StreamId::agent_session(&session_id)
                            .unwrap(),
                        after: None,
                        limit: 32,
                    })
                    .await
                    .unwrap();
                let accepted_events = stream
                    .events
                    .iter()
                    .filter(|event| matches!(
                        &event.event,
                        crate::domain::local_event::LoadedDomainEvent::Known(event)
                            if matches!(
                                event.as_ref(),
                                crate::domain::local_event::LocalDomainEvent::AgentSession(
                                    crate::domain::agent_session::events::AgentSessionDomainEvent::SendOperationAccepted {
                                        operation_id: saved,
                                        ..
                                    }
                                ) if saved == operation_id
                            )
                    ))
                    .count();
                let started_turns = stream
                    .events
                    .iter()
                    .filter(|event| matches!(
                        &event.event,
                        crate::domain::local_event::LoadedDomainEvent::Known(event)
                            if matches!(
                                event.as_ref(),
                                crate::domain::local_event::LocalDomainEvent::AgentSession(
                                    crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted { .. }
                                )
                            )
                    ))
                    .count();
                let projection = match store
                    .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                        session_id: session_id.clone(),
                    })
                    .await
                    .unwrap()
                {
                    crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                        Some(projection),
                    ) => projection,
                    other => panic!("unexpected session projection: {other:?}"),
                };
                let projection = crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
                    .decode(&projection.projection)
                    .unwrap();
                assert!(projection.pending_send_queue.is_empty());

                if *valid {
                    assert_eq!(
                        after_meta.state,
                        crate::usecase::agent_session::session::SessionState::Active
                    );
                    assert_eq!(gate.plan_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
                    assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
                    assert_eq!(accepted_events, 1, "{surface:?}/{label}");
                    assert_eq!(started_turns, 1, "{surface:?}/{label}");
                    assert_eq!(
                        page.messages
                            .iter()
                            .filter(|message| {
                                message.id == "b009-human"
                                    && message.role
                                        == crate::usecase::agent_session::session::MessageRole::Human
                            })
                            .count(),
                        1,
                        "{surface:?}/{label}"
                    );
                    assert_eq!(
                        page.messages
                            .iter()
                            .filter(|message| {
                                message.id == "b009-human:agent"
                                    && message.role
                                        == crate::usecase::agent_session::session::MessageRole::Agent
                            })
                            .count(),
                        1,
                        "{surface:?}/{label}"
                    );
                    assert_eq!(page.messages.len(), 2, "{surface:?}/{label}");
                    let operation = send
                        .get_operation(LOCAL_API_OPERATION_PRINCIPAL, operation_id)
                        .await
                        .expect("one public send operation");
                    assert_eq!(operation.receipt.operation_id, *operation_id);
                } else {
                    assert_eq!(after_meta.state, before_meta.state, "{surface:?}/{label}");
                    assert_eq!(after_meta.state_revision, before_meta.state_revision);
                    assert!(page.messages.is_empty(), "{surface:?}/{label}");
                    assert!(stream.events.is_empty(), "{surface:?}/{label}");
                    assert_eq!(gate.plan_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
                    assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 0);
                    assert_eq!(accepted_events, 0);
                    assert_eq!(started_turns, 0);
                    assert!(
                        journal
                            .pending_page_for_scope(
                                LOCAL_API_OPERATION_PRINCIPAL,
                                &session_id,
                                8,
                                None,
                            )
                            .await
                            .unwrap()
                            .entries
                            .is_empty()
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn b099_internal_principal_seams_preserve_tauri_and_websocket_secrecy() {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository(
            repository,
            store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let worktree_path = data.path().to_string_lossy().to_string();
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            "b099-session".to_string(),
            &worktree_path,
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session.state = crate::usecase::agent_session::session::SessionState::Idle;
        session_store
            .save_full_session_for_restore(data.path(), &session)
            .unwrap();
        let gate = Arc::new(PublicIdentitySendGate {
            session_store,
            session_id: "b099-session".to_string(),
            plan_calls: std::sync::atomic::AtomicUsize::new(0),
            effects: std::sync::atomic::AtomicUsize::new(0),
        });
        let send = crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
            store.clone(),
            store.clone(),
            gate.clone(),
            store.installation_id().to_string(),
        );
        let journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            store.clone(),
            store.clone(),
            store.installation_id().to_string(),
        );
        let command = CanonicalSendCommandV1 {
            target: crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                chat_session_id: Some("b099-session".to_string()),
                worktree_path,
            },
            content: "principal-secret".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("codex".to_string()),
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };
        let canonical_payload = serde_json::to_string(&command).unwrap();

        let tauri_p1 = crate::adaptor::controller::command::agent_session::session::dispatch_durable_send_for_principal(
            store.as_ref(),
            &send,
            &journal,
            "p-1",
            "b099-operation".to_string(),
            command.clone(),
        )
        .await
        .expect("principal p-1 accepted through the Tauri presenter");
        let websocket_p1 = execute_send_for_principal(
            &send,
            "p-1",
            "b099-operation".to_string(),
            canonical_payload.clone(),
        )
        .await
        .expect("principal p-1 replayed through the WebSocket presenter");
        let AgentSessionWsResultV1::SendOutcome {
            outcome: websocket_p1,
        } = websocket_p1
        else {
            panic!("expected the WebSocket send presenter")
        };
        assert_eq!(
            serde_json::to_value(websocket_p1).unwrap(),
            serde_json::to_value(&tauri_p1).unwrap()
        );

        let tauri_p1_lookup = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
            &send,
            "p-1",
            "b099-operation".to_string(),
        )
        .await
        .unwrap();
        let websocket_p1_lookup =
            get_send_for_principal(&send, "p-1", "b099-operation".to_string())
                .await
                .unwrap();
        let AgentSessionWsResultV1::SendOperation {
            operation: websocket_p1_lookup,
        } = websocket_p1_lookup
        else {
            panic!("expected the WebSocket send-query presenter")
        };
        assert_eq!(
            serde_json::to_value(websocket_p1_lookup).unwrap(),
            serde_json::to_value(tauri_p1_lookup).unwrap()
        );

        let tauri_p2 = crate::adaptor::controller::command::agent_session::session::dispatch_durable_send_for_principal(
            store.as_ref(),
            &send,
            &journal,
            "p-2",
            "b099-operation".to_string(),
            command,
        )
        .await
        .expect_err("the Tauri presenter must not reveal another principal's binding");
        assert_eq!(serde_json::to_value(tauri_p2).unwrap()["type"], "not_found");
        assert!(matches!(
            execute_send_for_principal(
                &send,
                "p-2",
                "b099-operation".to_string(),
                canonical_payload,
            )
            .await,
            Err(OperationApplicationErrorDtoV1::NotFound)
        ));
        let tauri_p2_lookup = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
            &send,
            "p-2",
            "b099-operation".to_string(),
        )
        .await
        .expect_err("the Tauri query presenter must preserve principal secrecy");
        assert_eq!(
            serde_json::to_value(tauri_p2_lookup).unwrap()["type"],
            "not_found"
        );
        assert!(matches!(
            get_send_for_principal(&send, "p-2", "b099-operation".to_string()).await,
            Err(OperationApplicationErrorDtoV1::NotFound)
        ));
        assert_eq!(gate.plan_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);

        let public_request = serde_json::json!({
            "type": "request_send",
            "id": "b099-public-schema",
            "operation_id": "b099-operation",
            "command": serde_json::from_str::<serde_json::Value>(
                &serde_json::to_string(&CanonicalSendCommandV1 {
                    target: crate::adaptor::controller::agent_session_operation_wiring::CanonicalSendTargetV1::Direct {
                        chat_session_id: Some("b099-session".to_string()),
                        worktree_path: "/repo".to_string(),
                    },
                    content: "principal-secret".to_string(),
                    permission_mode: "ask".to_string(),
                    plan_mode: false,
                    backend_id: None,
                    model_id: None,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                })
                .unwrap(),
            )
            .unwrap(),
        });
        assert!(serde_json::from_value::<AgentSessionWsRequestV1>(public_request.clone()).is_ok());
        let mut injected = public_request;
        injected["principal"] = serde_json::json!("p-2");
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(injected).is_err(),
            "the authenticated public request schema must not accept caller-supplied principals"
        );
    }

    #[tokio::test]
    async fn b073_websocket_router_enforces_connection_and_inflight_limits() {
        let dispatcher = RecordingWsDispatcher::new(true);
        let (url, server) = spawn_transport_server(dispatcher.clone()).await;
        let mut sockets = Vec::new();
        for _ in 0..super::super::MAX_AGENT_SESSION_CONNECTIONS {
            sockets.push(
                tokio_tungstenite::connect_async(url.as_str())
                    .await
                    .unwrap()
                    .0,
            );
        }
        let seventeenth = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap_err();
        match seventeenth {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                assert_eq!(
                    response.status(),
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                );
            }
            other => panic!("17th connection returned {other:?}"),
        }

        let socket = &mut sockets[0];
        for ordinal in 0..=MAX_INFLIGHT_REQUESTS {
            socket
                .send(current_request(&format!("outer-{ordinal}")))
                .await
                .unwrap();
        }
        dispatcher.wait_for_calls(MAX_INFLIGHT_REQUESTS).await;
        let capacity = response_json(socket).await;
        assert_eq!(capacity["id"], format!("outer-{MAX_INFLIGHT_REQUESTS}"));
        assert_eq!(capacity["error"]["type"], "capacity_exceeded");
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_INFLIGHT_REQUESTS
        );
        dispatcher.release();
        drop(sockets);
        server.abort();
    }

    #[tokio::test]
    async fn websocket_outer_id_lives_until_delivery_and_releases_afterward() {
        let dispatcher = RecordingWsDispatcher::new(true);
        let (url, server) = spawn_transport_server(dispatcher.clone()).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        socket.send(current_request("same-id")).await.unwrap();
        dispatcher.wait_for_calls(1).await;
        socket.send(current_request("same-id")).await.unwrap();
        let conflict = response_json(&mut socket).await;
        assert_eq!(conflict["error"]["type"], "request_id_conflict");
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        dispatcher.release();
        let first = response_json(&mut socket).await;
        assert_eq!(first["id"], "same-id");
        socket.send(current_request("same-id")).await.unwrap();
        let reused = response_json(&mut socket).await;
        assert_eq!(reused["id"], "same-id");
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b075_b087_authenticated_websocket_stop_validates_identity_and_turn_before_effect() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let cases = [
            (
                "a".to_string(),
                serde_json::json!("1"),
                serde_json::json!("0"),
                true,
            ),
            (
                "a".repeat(128),
                serde_json::json!("1"),
                serde_json::json!("0"),
                true,
            ),
            (
                "turn-max".to_string(),
                serde_json::json!(i64::MAX.to_string()),
                serde_json::json!("0"),
                true,
            ),
            (
                "revision-one".to_string(),
                serde_json::json!("1"),
                serde_json::json!("1"),
                true,
            ),
            (
                "revision-max".to_string(),
                serde_json::json!("1"),
                serde_json::json!(i64::MAX.to_string()),
                true,
            ),
            (
                String::new(),
                serde_json::json!("1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "a".repeat(129),
                serde_json::json!("1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "非ascii".to_string(),
                serde_json::json!("1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "bad/id".to_string(),
                serde_json::json!("1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-empty".to_string(),
                serde_json::json!(""),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-zero".to_string(),
                serde_json::json!("0"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-leading-zero".to_string(),
                serde_json::json!("01"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-plus".to_string(),
                serde_json::json!("+1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-negative".to_string(),
                serde_json::json!("-1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-exponent".to_string(),
                serde_json::json!("1e0"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-unicode".to_string(),
                serde_json::json!("１"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-leading-space".to_string(),
                serde_json::json!(" 1"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-trailing-space".to_string(),
                serde_json::json!("1 "),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-overflow".to_string(),
                serde_json::json!("9223372036854775808"),
                serde_json::json!("0"),
                false,
            ),
            (
                "turn-number".to_string(),
                serde_json::json!(1),
                serde_json::json!("0"),
                false,
            ),
            (
                "revision-empty".to_string(),
                serde_json::json!("1"),
                serde_json::json!(""),
                false,
            ),
            (
                "revision-leading-zero".to_string(),
                serde_json::json!("1"),
                serde_json::json!("01"),
                false,
            ),
            (
                "revision-plus".to_string(),
                serde_json::json!("1"),
                serde_json::json!("+1"),
                false,
            ),
            (
                "revision-negative".to_string(),
                serde_json::json!("1"),
                serde_json::json!("-1"),
                false,
            ),
            (
                "revision-exponent".to_string(),
                serde_json::json!("1"),
                serde_json::json!("1e0"),
                false,
            ),
            (
                "revision-unicode".to_string(),
                serde_json::json!("1"),
                serde_json::json!("１"),
                false,
            ),
            (
                "revision-leading-space".to_string(),
                serde_json::json!("1"),
                serde_json::json!(" 1"),
                false,
            ),
            (
                "revision-trailing-space".to_string(),
                serde_json::json!("1"),
                serde_json::json!("1 "),
                false,
            ),
            (
                "revision-overflow".to_string(),
                serde_json::json!("1"),
                serde_json::json!("9223372036854775808"),
                false,
            ),
            (
                "revision-number".to_string(),
                serde_json::json!("1"),
                serde_json::json!(0),
                false,
            ),
        ];

        for (ordinal, (request_id, turn_id, expected_session_revision, valid)) in
            cases.into_iter().enumerate()
        {
            let data = tempfile::tempdir().unwrap();
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    data.path().to_path_buf(),
                ),
            )
            .unwrap();
            let gate = Arc::new(DurableStopGate {
                turn_id: turn_id.as_str().unwrap_or("1").to_string(),
                session_revision: expected_session_revision
                    .as_str()
                    .and_then(decode_nonnegative_u64_decimal)
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
                    gate.clone(),
                    store.installation_id().to_string(),
                ),
            );
            let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableStopWsDispatcher {
                usecase: usecase.clone(),
            });
            let (url, server) = spawn_authenticated_transport_server(dispatcher).await;
            let mut websocket_request = url.as_str().into_client_request().unwrap();
            websocket_request
                .headers_mut()
                .insert("authorization", "Bearer b004-token".parse().unwrap());
            let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
                .await
                .expect("authenticated Stop WebSocket");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "type": "request_stop",
                        "id": format!("b087-ws-{ordinal}"),
                        "request": {
                            "request_id": request_id,
                            "session_id": "b087-websocket-session",
                            "turn_id": turn_id,
                            "expected_session_revision": expected_session_revision,
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let public = response_json(&mut socket).await;
            if valid {
                assert_eq!(public["status"], "ok", "{public}");
                assert_eq!(public["result"]["type"], "stop_outcome", "{public}");
                assert_eq!(public["result"]["outcome"]["type"], "accepted");
                assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
            } else {
                assert_eq!(public["status"], "error", "{public}");
                assert_eq!(public["error"]["type"], "invalid_request", "{public}");
                assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 0);
                if !request_id.is_empty()
                    && request_id.len() <= 128
                    && request_id.is_ascii()
                    && !request_id.contains('/')
                {
                    assert!(matches!(
                        usecase
                            .get_operation(LOCAL_API_OPERATION_PRINCIPAL, &request_id)
                            .await,
                        Err(crate::usecase::agent_session::operation::StopOperationError::NotFound)
                    ));
                }
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn websocket_disconnect_reconnect_reuses_durable_operation_once() {
        let app_data = tempfile::TempDir::new().expect("WebSocket SQLite app data");
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data.path().to_path_buf(),
            ),
        )
        .expect("open durable WebSocket operation store");
        let gate = Arc::new(DurableStopGate {
            turn_id: "1".to_string(),
            session_revision: 0,
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
                gate.clone(),
                store.installation_id().to_string(),
            ),
        );
        let dispatcher: Arc<dyn WsDispatchService> = Arc::new(DurableStopWsDispatcher {
            usecase: usecase.clone(),
        });
        let (url, server) = spawn_transport_server(dispatcher).await;
        let request = |outer_id: &str| {
            tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "request_stop",
                    "id": outer_id,
                    "request": {
                        "request_id": "stop-durable-1",
                        "session_id": "session-1",
                        "turn_id": "1",
                        "expected_session_revision": "0"
                    }
                })
                .to_string()
                .into(),
            )
        };
        let (mut first, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        first.send(request("outer-before-loss")).await.unwrap();
        drop(first);
        let (mut second, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        second.send(request("outer-after-reconnect")).await.unwrap();
        let replay = response_json(&mut second).await;
        assert_eq!(
            replay["result"]["type"], "stop_outcome",
            "unexpected replay response: {replay}"
        );
        let (_, state) = usecase
            .get_operation(LOCAL_API_OPERATION_PRINCIPAL, "stop-durable-1")
            .await
            .expect("durable Stop identity readback");
        assert!(matches!(
            state,
            crate::usecase::agent_session::operation::StopOperationState::Completed { .. }
        ));
        assert_eq!(gate.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_router_closes_an_oversize_frame_with_1009() {
        let dispatcher = RecordingWsDispatcher::new(false);
        let (url, server) = spawn_transport_server(dispatcher.clone()).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "x".repeat(MAX_MESSAGE_BYTES + 1).into(),
            ))
            .await
            .unwrap();
        match socket.next().await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), CLOSE_MESSAGE_TOO_LARGE);
            }
            other => panic!("oversize frame returned {other:?}"),
        }
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        server.abort();
    }

    #[tokio::test]
    async fn b073_authenticated_websocket_accepts_exact_sixteen_mibibyte_request_and_stays_open() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let dispatcher = RecordingWsDispatcher::new(false);
        let (url, server) = spawn_authenticated_transport_server(dispatcher.clone()).await;
        let mut websocket_request = url.as_str().into_client_request().unwrap();
        websocket_request
            .headers_mut()
            .insert("authorization", "Bearer b004-token".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_request)
            .await
            .expect("authenticated boundary WebSocket");

        let mut boundary = serde_json::json!({
            "type": "get_current_shutdown",
            "id": "b073-request-boundary",
        })
        .to_string();
        boundary.push_str(&" ".repeat(MAX_MESSAGE_BYTES - boundary.len()));
        assert_eq!(boundary.len(), MAX_MESSAGE_BYTES);
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                boundary.into(),
            ))
            .await
            .unwrap();
        let accepted = response_json(&mut socket).await;
        assert_eq!(accepted["id"], "b073-request-boundary");
        assert_eq!(accepted["error"]["type"], "invalid_request");
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        socket
            .send(current_request("b073-after-boundary"))
            .await
            .unwrap();
        let still_open = response_json(&mut socket).await;
        assert_eq!(still_open["id"], "b073-after-boundary");
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_router_closes_outbound_backpressure_with_1013() {
        let dispatcher = RecordingWsDispatcher::new(false);
        let (url, server) = spawn_transport_server_with_budget(dispatcher, 1).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        socket.send(current_request("outbound-full")).await.unwrap();
        match socket.next().await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), CLOSE_TRY_AGAIN_LATER);
            }
            other => panic!("outbound backpressure returned {other:?}"),
        }
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_actual_socket_closes_when_thirty_two_response_queue_is_saturated() {
        let dispatcher = RecordingWsDispatcher::new(true);
        let writer_permits = Arc::new(tokio::sync::Semaphore::new(0));
        let writer_entered = Arc::new(tokio::sync::Notify::new());
        let (url, server) = spawn_transport_server_with_writer_gate(
            dispatcher.clone(),
            MAX_MESSAGE_BYTES,
            Some((writer_permits.clone(), writer_entered.clone())),
        )
        .await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        for ordinal in 0..(MAX_INFLIGHT_REQUESTS + 2) {
            socket
                .send(current_request(&format!("queue-{ordinal}")))
                .await
                .unwrap();
        }
        dispatcher.wait_for_calls(MAX_INFLIGHT_REQUESTS).await;
        writer_entered.notified().await;
        dispatcher.release();
        tokio::task::yield_now().await;
        writer_permits.add_permits(MAX_OUTBOUND_RESPONSES + 2);

        loop {
            match socket.next().await.unwrap().unwrap() {
                tokio_tungstenite::tungstenite::Message::Close(Some(frame)) => {
                    assert_eq!(u16::from(frame.code), CLOSE_TRY_AGAIN_LATER);
                    break;
                }
                tokio_tungstenite::tungstenite::Message::Text(_) => {}
                other => panic!("outbound queue saturation returned {other:?}"),
            }
        }
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_actual_socket_replaces_an_oversized_response() {
        let dispatcher = RecordingWsDispatcher::new(false);
        dispatcher.return_oversized_response();
        let (url, server) = spawn_transport_server(dispatcher).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        socket
            .send(current_request("oversized-response"))
            .await
            .unwrap();
        let response = response_json(&mut socket).await;
        assert_eq!(response["id"], "oversized-response");
        assert_eq!(response["error"]["type"], "response_too_large");
        assert!(response.to_string().len() <= MAX_MESSAGE_BYTES);
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_actual_socket_delivers_the_exact_sixteen_mibibyte_response_boundary() {
        let dispatcher = RecordingWsDispatcher::new(false);
        dispatcher.return_boundary_response();
        let (url, server) = spawn_transport_server(dispatcher).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        socket
            .send(current_request("boundary-response"))
            .await
            .unwrap();
        match socket.next().await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Text(response) => {
                assert_eq!(response.len(), MAX_MESSAGE_BYTES);
                let response: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(response["error"]["type"], "internal");
            }
            other => panic!("response boundary returned {other:?}"),
        }
        server.abort();
    }

    #[tokio::test]
    async fn b073_websocket_router_rejects_the_121st_burst_request() {
        let dispatcher = RecordingWsDispatcher::new(true);
        let (url, server) = spawn_transport_server(dispatcher.clone()).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        for ordinal in 0..121 {
            socket
                .send(current_request(&format!("rate-{ordinal}")))
                .await
                .unwrap();
        }
        let mut saw_rate_limited = false;
        for _ in 0..(121 - MAX_INFLIGHT_REQUESTS) {
            let response = response_json(&mut socket).await;
            if response["id"] == "rate-120" {
                assert_eq!(response["error"]["type"], "rate_limited");
                saw_rate_limited = true;
                break;
            }
        }
        assert!(saw_rate_limited);
        assert_eq!(
            dispatcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_INFLIGHT_REQUESTS
        );
        dispatcher.release();
        server.abort();
    }

    #[test]
    fn route_is_closed_and_session_lifecycle_is_not_a_variant() {
        let session = serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
            "type": "get_session",
            "id": "outer-session",
            "session_id": "session-a",
            "attempt_id": "load-attempt-a"
        }));
        assert!(matches!(
            session,
            Ok(AgentSessionWsRequestV1::GetSession { .. })
        ));
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
                "type": "get_session",
                "id": "outer-session-missing-attempt",
                "session_id": "session-a"
            }))
            .is_err()
        );
        let unknown = serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
            "type": "request_session_lifecycle",
            "id": "outer-1",
            "request": {}
        }));
        assert!(unknown.is_err());
        let recovery = serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
            "type": "get_pending_recovery",
            "id": "outer-2",
            "limit": 32,
            "partition": "owner",
            "owner": null,
            "shutdown_id": null,
            "cursor": null
        }));
        assert!(matches!(
            recovery,
            Ok(AgentSessionWsRequestV1::GetPendingRecovery { .. })
        ));
        let snapshot = serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
            "type": "get_pending_recovery_snapshot",
            "id": "outer-snapshot",
            "shutdown_id": "plan-1",
            "snapshot_id": "snapshot-1",
            "partition": "closed_session",
            "limit": 200,
            "cursor": null
        }));
        assert!(matches!(
            snapshot,
            Ok(AgentSessionWsRequestV1::GetPendingRecoverySnapshot { .. })
        ));
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
                "type": "get_pending_recovery",
                "id": "outer-3"
            }))
            .is_err()
        );
        let feedback = serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
            "type": "list_feedback",
            "id": "outer-4",
            "session_id": "session-a",
            "limit": 32,
            "cursor": null
        }));
        assert!(matches!(
            feedback,
            Ok(AgentSessionWsRequestV1::ListFeedback { .. })
        ));
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
                "type": "list_feedback",
                "id": "outer-5",
                "session_id": "session-a"
            }))
            .is_err()
        );
        assert_eq!(parse_decimal_revision("0"), Some(0));
        assert_eq!(
            parse_decimal_revision(&i64::MAX.to_string()),
            Some(i64::MAX as u64)
        );
        assert_eq!(parse_decimal_revision("01"), None);
        assert_eq!(parse_decimal_revision("9223372036854775808"), None);
        assert!(valid_outer_id("outer-1"));
        assert!(!valid_outer_id(""));
        assert!(!valid_outer_id("contains space"));
    }

    #[test]
    fn b089_unknown_public_snapshot_partition_tag_is_invalid_request() {
        assert!(
            serde_json::from_value::<AgentSessionWsRequestV1>(serde_json::json!({
                "type": "get_pending_recovery_snapshot",
                "id": "outer-snapshot-unknown-partition",
                "shutdown_id": "plan-1",
                "snapshot_id": "snapshot-1",
                "partition": "future_partition",
                "limit": 200,
                "cursor": null
            }))
            .is_err(),
            "an unknown public partition tag must fail closed as InvalidRequest"
        );
    }

    #[test]
    fn websocket_resource_contract_is_explicit_and_separated() {
        assert_eq!(super::super::MAX_AGENT_SESSION_CONNECTIONS, 16);
        assert_eq!(MAX_INFLIGHT_REQUESTS, 32);
        assert_eq!(MAX_OUTBOUND_RESPONSES, 32);
        assert_eq!(MAX_MESSAGE_BYTES, 16 * 1024 * 1024);
        assert_eq!(CLOSE_MESSAGE_TOO_LARGE, 1009);
        assert_eq!(CLOSE_TRY_AGAIN_LATER, 1013);

        let mut budget = RateBudget::new(RATE_PER_SECOND);
        for _ in 0..120 {
            assert!(budget.acquire());
        }
        assert!(!budget.acquire());
    }

    #[test]
    fn websocket_limit_errors_have_distinct_closed_shapes() {
        assert_eq!(
            serde_json::to_value(OperationApplicationErrorDtoV1::RateLimited).unwrap(),
            serde_json::json!({ "type": "rate_limited" })
        );
        assert_eq!(
            serde_json::to_value(OperationApplicationErrorDtoV1::RequestIdConflict).unwrap(),
            serde_json::json!({ "type": "request_id_conflict" })
        );
        assert_eq!(
            serde_json::to_value(OperationApplicationErrorDtoV1::ShutdownInProgress).unwrap(),
            serde_json::json!({ "type": "shutdown_in_progress" })
        );
        assert_eq!(
            serde_json::to_value(OperationApplicationErrorDtoV1::ResponseTooLarge).unwrap(),
            serde_json::json!({ "type": "response_too_large" })
        );
    }

    #[tokio::test]
    async fn transport_admission_and_outbound_queues_enforce_live_limits() {
        let connections = Arc::new(tokio::sync::Semaphore::new(
            super::super::MAX_AGENT_SESSION_CONNECTIONS,
        ));
        let mut connection_permits = Vec::new();
        for _ in 0..super::super::MAX_AGENT_SESSION_CONNECTIONS {
            connection_permits.push(connections.clone().try_acquire_owned().unwrap());
        }
        assert!(connections.clone().try_acquire_owned().is_err());

        let inflight = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_REQUESTS));
        let mut inflight_permits = Vec::new();
        for _ in 0..MAX_INFLIGHT_REQUESTS {
            inflight_permits.push(inflight.clone().try_acquire_owned().unwrap());
        }
        assert!(inflight.clone().try_acquire_owned().is_err());

        let mut ids = HashSet::new();
        assert!(ids.insert("outer-1".to_string()));
        assert!(!ids.insert("outer-1".to_string()));

        let (outbound, _outbound_rx) = tokio::sync::mpsc::channel(MAX_OUTBOUND_RESPONSES);
        let (close, mut close_rx) = tokio::sync::mpsc::unbounded_channel();
        let outbound_bytes = Arc::new(tokio::sync::Semaphore::new(1));
        enqueue_ws_response(
            &outbound,
            &outbound_bytes,
            &close,
            AgentSessionWsResponseV1::Error {
                id: "outer-2".to_string(),
                error: OperationApplicationErrorDtoV1::InvalidRequest,
            },
        );
        assert_eq!(
            close_rx.try_recv().unwrap(),
            (CLOSE_TRY_AGAIN_LATER, "outbound backpressure")
        );

        let mut budget = RateBudget {
            tokens: RATE_BURST,
            updated_at: Instant::now(),
            refill_per_second: RATE_PER_SECOND,
        };
        assert!((0..120).all(|_| budget.acquire()));
        assert!(!budget.acquire());
        assert!("x".repeat(MAX_MESSAGE_BYTES + 1).len() > MAX_MESSAGE_BYTES);
        assert_eq!(CLOSE_MESSAGE_TOO_LARGE, 1009);

        drop(inflight_permits);
        drop(connection_permits);
        assert!(connections.try_acquire_owned().is_ok());
        assert!(inflight.try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn outer_id_reservation_is_owned_by_the_outbound_response() {
        let ids = Arc::new(std::sync::Mutex::new(HashSet::new()));
        ids.lock().unwrap().insert("outer-held".to_string());
        let reservation = OuterRequestIdReservation {
            id: "outer-held".to_string(),
            inflight_ids: Arc::clone(&ids),
        };
        let (outbound, mut outbound_rx) = tokio::sync::mpsc::channel(1);
        let (close, _close_rx) = tokio::sync::mpsc::unbounded_channel();
        let outbound_bytes = Arc::new(tokio::sync::Semaphore::new(MAX_MESSAGE_BYTES));
        enqueue_ws_response_reserved(
            &outbound,
            &outbound_bytes,
            &close,
            AgentSessionWsResponseV1::Error {
                id: "outer-held".to_string(),
                error: OperationApplicationErrorDtoV1::InvalidRequest,
            },
            reservation,
        );
        assert!(ids.lock().unwrap().contains("outer-held"));
        let response = outbound_rx.recv().await.unwrap();
        assert!(ids.lock().unwrap().contains("outer-held"));
        drop(response);
        assert!(!ids.lock().unwrap().contains("outer-held"));

        ids.lock().unwrap().insert("outer-failed".to_string());
        let reservation = OuterRequestIdReservation {
            id: "outer-failed".to_string(),
            inflight_ids: Arc::clone(&ids),
        };
        drop(outbound_rx);
        enqueue_ws_response_reserved(
            &outbound,
            &outbound_bytes,
            &close,
            AgentSessionWsResponseV1::Error {
                id: "outer-failed".to_string(),
                error: OperationApplicationErrorDtoV1::InvalidRequest,
            },
            reservation,
        );
        assert!(!ids.lock().unwrap().contains("outer-failed"));
    }
}
