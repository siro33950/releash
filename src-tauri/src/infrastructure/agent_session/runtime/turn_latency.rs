use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::app_data_dir::resolve_data_dir;
use crate::other::telemetry::{
    record_agent_turn_duration, AgentTurn, AgentTurnDimensions, ModelFamily, Payload,
    PermissionModeDim, TurnContext, WarmPath,
};
use crate::usecase::agent_session::session::SessionStore;

const MAX_UI_TO_START_MS: f64 = 60.0 * 60.0 * 1000.0;

#[derive(Debug)]
pub(crate) struct TurnLatencyState {
    turn_origin: Instant,
    dims: AgentTurnDimensions,
    query_init_recorded: bool,
    first_sdk_event_recorded: bool,
    first_assistant_event_recorded: bool,
    permission_wait_started_at_by_request: HashMap<String, Instant>,
    complete_recorded: bool,
}

impl TurnLatencyState {
    pub(crate) fn new(dims: AgentTurnDimensions) -> Self {
        Self {
            turn_origin: Instant::now(),
            dims,
            query_init_recorded: false,
            first_sdk_event_recorded: false,
            first_assistant_event_recorded: false,
            permission_wait_started_at_by_request: HashMap::new(),
            complete_recorded: false,
        }
    }
}

pub(crate) fn dimensions_for_session<R: tauri::Runtime>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    chat_session_id: &str,
    permission_mode: &str,
    model_id: Option<&str>,
    resume: bool,
    has_session: bool,
) -> AgentTurnDimensions {
    dimensions(
        permission_mode,
        model_id,
        context_for_session(app, session_store, chat_session_id),
        resume,
        has_session,
        Payload::TauriEvent,
    )
}

pub(crate) fn dimensions_from_metadata(
    permission_mode: &str,
    model_id: Option<&str>,
    is_workflow_step: bool,
    resume: bool,
    has_session: bool,
) -> AgentTurnDimensions {
    dimensions(
        permission_mode,
        model_id,
        TurnContext::from_workflow_step(is_workflow_step),
        resume,
        has_session,
        Payload::TauriEvent,
    )
}

fn dimensions(
    permission_mode: &str,
    model_id: Option<&str>,
    context: TurnContext,
    resume: bool,
    has_session: bool,
    channel: Payload,
) -> AgentTurnDimensions {
    AgentTurnDimensions {
        resume,
        has_session,
        permission_mode: PermissionModeDim::normalize(permission_mode),
        model: ModelFamily::normalize(model_id),
        context,
        channel,
        warm_path: WarmPath::QueryDirect,
    }
}

fn context_for_session<R: tauri::Runtime>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    chat_session_id: &str,
) -> TurnContext {
    let (Some(app), Some(session_store)) = (app, session_store) else {
        return TurnContext::Chat;
    };
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!("Failed to resolve data dir for agent turn telemetry context: {e}");
            return TurnContext::Chat;
        }
    };
    match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(Some(meta)) => TurnContext::from_workflow_step(meta.is_workflow_step_session()),
        Ok(None) => TurnContext::Chat,
        Err(e) => {
            log::warn!("Failed to load session meta for agent turn telemetry context: {e}");
            TurnContext::Chat
        }
    }
}

pub(crate) fn record_ui_to_start_latency(
    permission_mode: &str,
    model_id: Option<&str>,
    is_workflow_step: bool,
    resume: bool,
    has_session: bool,
    client_sent_at_ms: Option<f64>,
    request_received_at_ms: Option<f64>,
) {
    let Some(elapsed) = ui_to_start_elapsed(client_sent_at_ms, request_received_at_ms) else {
        return;
    };
    let dims = dimensions_from_metadata(
        permission_mode,
        model_id,
        is_workflow_step,
        resume,
        has_session,
    );
    record_agent_turn_duration(AgentTurn::UiToStart, &dims, elapsed);
}

pub(crate) fn record_bridge_spawn(dims: &AgentTurnDimensions, elapsed: Duration) {
    record_agent_turn_duration(AgentTurn::BridgeSpawn, dims, elapsed);
}

pub(crate) fn record_sdk_message(state: &mut Option<TurnLatencyState>, msg: &serde_json::Value) {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if matches!(msg_type, "telemetry" | "turn_complete") {
        return;
    }
    record_first_sdk_event_latency(state);
    if message_has_assistant_latency_event(msg) {
        record_first_assistant_event_latency(state);
    }
}

pub(crate) fn record_bridge_telemetry_message(
    state: &mut Option<TurnLatencyState>,
    active_turn_token: Option<&str>,
    message_turn_token: Option<&str>,
    msg: &Value,
) {
    if msg.get("metric").and_then(|v| v.as_str()) != Some("query_init") {
        return;
    }
    let Some(turn_token) = message_turn_token else {
        return;
    };
    if active_turn_token != Some(turn_token) {
        return;
    }
    let Some(duration_ms) = msg.get("duration_ms").and_then(|v| v.as_f64()) else {
        return;
    };
    if !duration_ms.is_finite() || duration_ms < 0.0 {
        return;
    }
    let Ok(elapsed) = Duration::try_from_secs_f64(duration_ms / 1000.0) else {
        return;
    };
    record_query_init_latency(state, elapsed);
}

pub(crate) fn begin_permission_wait_latency(
    state: &mut Option<TurnLatencyState>,
    request_id: &str,
    started_at: Instant,
) {
    if request_id.is_empty() {
        return;
    }
    let Some(state) = state.as_mut() else {
        return;
    };
    state
        .permission_wait_started_at_by_request
        .entry(request_id.to_string())
        .or_insert(started_at);
}

pub(crate) fn record_permission_wait_latency(
    state: &mut Option<TurnLatencyState>,
    request_id: &str,
) {
    if request_id.is_empty() {
        return;
    }
    let Some(state) = state.as_mut() else {
        return;
    };
    let Some(started_at) = state
        .permission_wait_started_at_by_request
        .remove(request_id)
    else {
        return;
    };
    record_agent_turn_duration(AgentTurn::PermissionWait, &state.dims, started_at.elapsed());
}

pub(crate) fn record_complete_latency(state: &mut Option<TurnLatencyState>) {
    if let Some(current) = state.as_mut() {
        if !current.complete_recorded {
            current.complete_recorded = true;
            record_agent_turn_duration(
                AgentTurn::Complete,
                &current.dims,
                current.turn_origin.elapsed(),
            );
        }
    }
    *state = None;
}

fn record_query_init_latency(state: &mut Option<TurnLatencyState>, elapsed: Duration) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.query_init_recorded {
        return;
    }
    state.query_init_recorded = true;
    record_agent_turn_duration(AgentTurn::QueryInit, &state.dims, elapsed);
}

fn record_first_sdk_event_latency(state: &mut Option<TurnLatencyState>) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.first_sdk_event_recorded {
        return;
    }
    state.first_sdk_event_recorded = true;
    record_agent_turn_duration(
        AgentTurn::FirstSdkEvent,
        &state.dims,
        state.turn_origin.elapsed(),
    );
}

fn record_first_assistant_event_latency(state: &mut Option<TurnLatencyState>) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if state.first_assistant_event_recorded {
        return;
    }
    state.first_assistant_event_recorded = true;
    record_agent_turn_duration(
        AgentTurn::FirstAssistantEvent,
        &state.dims,
        state.turn_origin.elapsed(),
    );
}

fn ui_to_start_elapsed(
    client_sent_at_ms: Option<f64>,
    request_received_at_ms: Option<f64>,
) -> Option<Duration> {
    let client_sent_at_ms = client_sent_at_ms?;
    let request_received_at_ms = request_received_at_ms?;
    if !client_sent_at_ms.is_finite() || !request_received_at_ms.is_finite() {
        return None;
    }
    let elapsed_ms = request_received_at_ms - client_sent_at_ms;
    if !(0.0..=MAX_UI_TO_START_MS).contains(&elapsed_ms) {
        return None;
    }
    Some(Duration::from_secs_f64(elapsed_ms / 1000.0))
}

fn message_has_assistant_latency_event(msg: &serde_json::Value) -> bool {
    match msg.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "stream_event" => {
            let Some(event) = msg.get("event") else {
                return false;
            };
            match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "content_block_delta" => event
                    .get("delta")
                    .and_then(|delta| delta.get("type"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|delta_type| {
                        matches!(delta_type, "text_delta" | "thinking_delta")
                    }),
                "content_block_start" => event
                    .get("content_block")
                    .and_then(|block| block.get("type"))
                    .and_then(|v| v.as_str())
                    .is_some_and(is_assistant_tool_content_block),
                _ => false,
            }
        }
        "assistant" => msg
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_array())
            .is_some_and(|content| {
                content.iter().any(|block| {
                    block
                        .get("type")
                        .and_then(|v| v.as_str())
                        .is_some_and(|block_type| {
                            matches!(block_type, "text" | "thinking")
                                || is_assistant_tool_content_block(block_type)
                        })
                })
            }),
        _ => false,
    }
}

fn is_assistant_tool_content_block(block_type: &str) -> bool {
    matches!(block_type, "tool_use" | "server_tool_use" | "mcp_tool_use")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_turn_dimensions() -> AgentTurnDimensions {
        dimensions_from_metadata("edit", Some("claude-sonnet-4-6"), false, false, true)
    }

    fn turn_latency_records() -> Vec<crate::other::telemetry::TestMetricRecord> {
        crate::other::telemetry::test_metric_records()
            .into_iter()
            .filter(|record| record.name == "releash.agent.turn.duration_ms")
            .collect()
    }

    fn operation_count(
        records: &[crate::other::telemetry::TestMetricRecord],
        operation: &str,
    ) -> usize {
        records
            .iter()
            .filter(|record| {
                record.attributes.iter().any(|(key, value)| {
                    key == crate::other::telemetry::attributes::KEY_OPERATION && value == operation
                })
            })
            .count()
    }

    fn record_has_attr(
        record: &crate::other::telemetry::TestMetricRecord,
        key: &str,
        value: &str,
    ) -> bool {
        record
            .attributes
            .iter()
            .any(|(record_key, record_value)| record_key == key && record_value == value)
    }

    fn record_all_turn_section_latencies(state: &mut Option<TurnLatencyState>) {
        record_bridge_telemetry_message(
            state,
            Some("turn-1"),
            Some("turn-1"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": 42.0,
                "turn_token": "turn-1"
            }),
        );
        record_sdk_message(
            state,
            &serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": { "type": "text_delta", "text": "hello" }
                }
            }),
        );
        begin_permission_wait_latency(state, "req-1", Instant::now() - Duration::from_millis(10));
        record_permission_wait_latency(state, "req-1");
        record_complete_latency(state);
    }

    fn setup_test_telemetry() -> crate::other::telemetry::TestTelemetryGuard {
        let guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        guard
    }

    #[test]
    fn records_first_events_and_complete_once() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": { "type": "text_delta", "text": "secret user text" }
            }
        });

        record_sdk_message(&mut state, &msg);
        record_sdk_message(&mut state, &msg);
        record_complete_latency(&mut state);
        record_complete_latency(&mut state);

        let records = turn_latency_records();
        assert_eq!(operation_count(&records, "agent.turn.first_sdk_event"), 1);
        assert_eq!(
            operation_count(&records, "agent.turn.first_assistant_event"),
            1
        );
        assert_eq!(operation_count(&records, "agent.turn.complete"), 1);
        let allowed_keys = AgentTurnDimensions::ALLOWED_ATTRIBUTE_KEYS;
        for record in records {
            assert!(record.attributes.iter().all(|(key, value)| {
                allowed_keys.contains(&key.as_str()) && value != "secret user text"
            }));
        }
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn records_tool_first_content_block_start_once() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));
        let tool_start = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": {
                    "type": "tool_use",
                    "id": "tool-1",
                    "name": "Read",
                    "input": {"file_path": "secret user path"}
                }
            }
        });
        let input_delta = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "input_json_delta", "partial_json": "{\"file_path\":\"secret\"}"}
            }
        });
        let assistant_message = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "tool-1",
                    "name": "Read",
                    "input": {"file_path": "secret user path"}
                }]
            }
        });

        record_sdk_message(&mut state, &tool_start);
        record_sdk_message(&mut state, &input_delta);
        record_sdk_message(&mut state, &assistant_message);

        let records = turn_latency_records();
        assert_eq!(operation_count(&records, "agent.turn.first_sdk_event"), 1);
        assert_eq!(
            operation_count(&records, "agent.turn.first_assistant_event"),
            1
        );
        let allowed_keys = AgentTurnDimensions::ALLOWED_ATTRIBUTE_KEYS;
        for record in records {
            assert!(record.attributes.iter().all(|(key, value)| {
                allowed_keys.contains(&key.as_str()) && value != "secret user path"
            }));
        }
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn query_init_telemetry_records_only_matching_turn_token() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));

        record_bridge_telemetry_message(
            &mut state,
            Some("turn-1"),
            Some("turn-1"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": 42.0,
                "turn_token": "turn-1"
            }),
        );
        record_bridge_telemetry_message(
            &mut state,
            Some("turn-1"),
            Some("stale"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": 100.0,
                "turn_token": "stale"
            }),
        );
        record_bridge_telemetry_message(
            &mut state,
            Some("turn-1"),
            Some("turn-1"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": -1.0,
                "turn_token": "turn-1"
            }),
        );

        let records = turn_latency_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 42.0);
        assert!(records[0].attributes.iter().any(|(key, value)| {
            key == crate::other::telemetry::attributes::KEY_OPERATION
                && value == "agent.turn.query_init"
        }));
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn query_init_telemetry_discards_overlarge_duration_without_panicking() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));

        record_bridge_telemetry_message(
            &mut state,
            Some("turn-1"),
            Some("turn-1"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": 1e30,
                "turn_token": "turn-1"
            }),
        );

        assert!(turn_latency_records().is_empty());

        record_bridge_telemetry_message(
            &mut state,
            Some("turn-1"),
            Some("turn-1"),
            &serde_json::json!({
                "type": "telemetry",
                "metric": "query_init",
                "duration_ms": 42.0,
                "turn_token": "turn-1"
            }),
        );

        let records = turn_latency_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 42.0);
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn record_bridge_spawn_records_bridge_spawn_operation() {
        let _guard = setup_test_telemetry();
        let dims = test_turn_dimensions();

        record_bridge_spawn(&dims, Duration::from_millis(17));

        let records = turn_latency_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 17.0);
        assert!(records[0].attributes.iter().any(|(key, value)| {
            key == crate::other::telemetry::attributes::KEY_OPERATION
                && value == "agent.turn.bridge_spawn"
        }));
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn permission_wait_latency_records_request_to_response_once() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));

        begin_permission_wait_latency(&mut state, "req-1", Instant::now());
        record_permission_wait_latency(&mut state, "req-1");
        record_permission_wait_latency(&mut state, "req-1");

        let records = turn_latency_records();
        let permission_wait_records = records
            .iter()
            .filter(|record| {
                record.attributes.iter().any(|(key, value)| {
                    key == crate::other::telemetry::attributes::KEY_OPERATION
                        && value == "agent.turn.permission_wait"
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(permission_wait_records.len(), 1);
        let allowed_keys = AgentTurnDimensions::ALLOWED_ATTRIBUTE_KEYS;
        assert!(permission_wait_records[0]
            .attributes
            .iter()
            .all(|(key, value)| {
                allowed_keys.contains(&key.as_str())
                    && value != "req-1"
                    && value != "secret user text"
            }));
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn permission_wait_latency_records_each_request_id_pair() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));

        begin_permission_wait_latency(&mut state, "req-1", Instant::now());
        begin_permission_wait_latency(&mut state, "req-2", Instant::now());
        record_permission_wait_latency(&mut state, "req-1");
        record_permission_wait_latency(&mut state, "req-2");
        record_permission_wait_latency(&mut state, "req-2");

        let records = turn_latency_records();
        let permission_wait_records = records
            .iter()
            .filter(|record| {
                record.attributes.iter().any(|(key, value)| {
                    key == crate::other::telemetry::attributes::KEY_OPERATION
                        && value == "agent.turn.permission_wait"
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(permission_wait_records.len(), 2);
        let allowed_keys = AgentTurnDimensions::ALLOWED_ATTRIBUTE_KEYS;
        for record in permission_wait_records {
            assert!(record.attributes.iter().all(|(key, value)| {
                allowed_keys.contains(&key.as_str())
                    && value != "req-1"
                    && value != "req-2"
                    && value != "secret user text"
            }));
        }
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn permission_wait_latency_uses_caller_start_and_keeps_first_request_start() {
        let _guard = setup_test_telemetry();
        let mut state = Some(TurnLatencyState::new(test_turn_dimensions()));
        let first_started_at = Instant::now() - Duration::from_millis(50);

        begin_permission_wait_latency(&mut state, "req-1", first_started_at);
        begin_permission_wait_latency(&mut state, "req-1", Instant::now());
        record_permission_wait_latency(&mut state, "req-1");

        let records = turn_latency_records();
        assert_eq!(operation_count(&records, "agent.turn.permission_wait"), 1);
        assert!(
            records[0].value >= 50.0,
            "permission_wait must use the first caller-provided request timestamp"
        );
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn ui_to_start_elapsed_rejects_missing_future_and_extreme_values() {
        assert_eq!(
            ui_to_start_elapsed(Some(1_000.0), Some(1_250.0)),
            Some(Duration::from_millis(250))
        );
        assert_eq!(ui_to_start_elapsed(None, Some(1_250.0)), None);
        assert_eq!(ui_to_start_elapsed(Some(1_250.0), Some(1_000.0)), None);
        assert_eq!(
            ui_to_start_elapsed(Some(0.0), Some(MAX_UI_TO_START_MS + 1.0)),
            None
        );
        assert_eq!(ui_to_start_elapsed(Some(f64::NAN), Some(1_000.0)), None);
    }

    #[test]
    fn ui_to_start_records_metadata_dimensions() {
        let _guard = setup_test_telemetry();

        record_ui_to_start_latency(
            "edit",
            Some("claude-sonnet-4-6"),
            true,
            false,
            true,
            Some(1_000.0),
            Some(1_250.0),
        );

        let records = turn_latency_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 250.0);
        let has_attr = |key: &str, value: &str| {
            records[0]
                .attributes
                .iter()
                .any(|(record_key, record_value)| record_key == key && record_value == value)
        };
        assert!(has_attr(
            crate::other::telemetry::attributes::KEY_OPERATION,
            "agent.turn.ui_to_start"
        ));
        assert!(has_attr("releash.agent.context", "workflow_step"));
        assert!(has_attr("releash.agent.model", "sonnet"));
        assert!(has_attr("releash.agent.resume", "false"));
        assert!(has_attr("releash.agent.has_session", "true"));
        crate::other::telemetry::reset_test_metrics();
    }

    #[test]
    fn dimensions_preserve_independent_resume_and_has_session_values() {
        let dims = dimensions_from_metadata("edit", None, false, false, true);

        assert!(!dims.resume);
        assert!(dims.has_session);

        let attrs = dims.to_metric_attrs(AgentTurn::Complete.operation());
        let has_attr = |key: &str, value: &str| {
            attrs
                .iter()
                .any(|attr| attr.key.as_str() == key && attr.value.to_string() == value)
        };
        assert!(has_attr("releash.agent.resume", "false"));
        assert!(has_attr("releash.agent.has_session", "true"));
    }

    #[test]
    fn turn_section_metrics_record_resume_dimensions_for_resumed_and_initial_turns() {
        let _guard = setup_test_telemetry();
        let cases = [
            (
                dimensions_from_metadata("edit", Some("claude-sonnet-4-6"), false, true, true),
                "true",
                "true",
            ),
            (
                dimensions_from_metadata("edit", Some("claude-sonnet-4-6"), false, false, false),
                "false",
                "false",
            ),
        ];

        for (dims, expected_resume, expected_has_session) in cases {
            crate::other::telemetry::reset_test_metrics();
            crate::other::telemetry::set_performance_configured(true);
            crate::other::telemetry::set_performance_enabled(true);
            let mut state = Some(TurnLatencyState::new(dims));

            record_all_turn_section_latencies(&mut state);

            let records = turn_latency_records();
            assert_eq!(records.len(), 5);
            for metric in [
                AgentTurn::QueryInit,
                AgentTurn::FirstSdkEvent,
                AgentTurn::FirstAssistantEvent,
                AgentTurn::PermissionWait,
                AgentTurn::Complete,
            ] {
                assert_eq!(operation_count(&records, metric.operation()), 1);
            }
            for record in records {
                assert!(record_has_attr(
                    &record,
                    crate::other::telemetry::attributes::KEY_AGENT_RESUME,
                    expected_resume
                ));
                assert!(record_has_attr(
                    &record,
                    crate::other::telemetry::attributes::KEY_AGENT_HAS_SESSION,
                    expected_has_session
                ));
            }
        }
        crate::other::telemetry::reset_test_metrics();
    }
}
