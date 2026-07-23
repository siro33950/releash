use std::sync::{Arc, Mutex};

use crate::adaptor::gateway::agent_session::session_storage::{
    AgentSessionProjectionCodecV1, FileSessionStorage,
};
use crate::adaptor::gateway::local_event_store::agent_session_codec::AgentSessionEventCodec;
use crate::adaptor::gateway::local_event_store::clock::FakeStoreClock;
use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::value_objects::PermissionMode;
use crate::infrastructure::agent_session::fixtures::{
    assert_golden, pretty_json, projection_backend, replay_backend, FixtureBackend,
    ProjectionFixture, ReplayedFixture,
};
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::runtime::SendAgentMessageRequest;
use crate::usecase::agent_session::session::{MessageRole, SessionStore};
use serde::Serialize;

const ASSISTANT_MESSAGE_ID: &str = "<ASSISTANT_MESSAGE_ID>";
const PROMPT_MESSAGE_ID: &str = "<PROMPT_MESSAGE_ID>";
const FIXTURE_TIMESTAMP: f64 = 1_700_000_000.0;

#[derive(Serialize)]
struct ReadModelSnapshot {
    messages: Vec<crate::adaptor::gateway::agent_session::session_storage::StoredChatMessageV1>,
    status: StatusSnapshot,
    error_reason: Option<String>,
    queue_paused_at: Option<f64>,
    workflow_turn_complete: Option<WorkflowTurnCompleteSnapshot>,
    tool_retries: Vec<ToolRetrySnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend_recovery: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StatusSnapshot {
    session_state: &'static str,
    turn_phase: crate::usecase::agent_session::status::TurnPhase,
}

#[derive(Serialize)]
struct WorkflowTurnCompleteSnapshot {
    turn_id: u64,
    exit_code: i64,
    final_text_parts: Vec<String>,
    failure_signal: Option<&'static str>,
    token_usage: Option<TokenUsageSnapshot>,
    interrupted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsageSnapshot {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Serialize)]
struct ToolRetrySnapshot {
    turn_id: u64,
    tool_use_id: String,
    attempt: u32,
}

#[tokio::test]
async fn claude_fixture_matches_read_model_golden() {
    for fixture in projection_backend(FixtureBackend::Claude) {
        assert_fixture_read_model(fixture).await;
    }
}

#[tokio::test]
async fn codex_fixture_matches_read_model_golden() {
    for fixture in projection_backend(FixtureBackend::Codex) {
        assert_fixture_read_model(fixture).await;
    }
}

#[tokio::test]
async fn b047_claude_wire_converter_runtime_sqlite_reopen_matches_read_model_golden() {
    for fixture in replay_backend(FixtureBackend::Claude) {
        assert_wire_fixture_read_model(fixture).await;
    }
}

#[tokio::test]
async fn b047_codex_wire_converter_runtime_sqlite_reopen_matches_read_model_golden() {
    for fixture in replay_backend(FixtureBackend::Codex) {
        assert_wire_fixture_read_model(fixture).await;
    }
}

async fn assert_fixture_read_model(fixture: ProjectionFixture) {
    let golden_path = fixture.read_model_golden_path();
    assert_runtime_event_read_model(
        fixture.backend,
        fixture.name,
        fixture.events,
        golden_path,
        false,
    )
    .await;
}

async fn assert_wire_fixture_read_model(fixture: ReplayedFixture) {
    let golden_path = fixture.read_model_golden_path();
    assert_runtime_event_read_model(
        fixture.backend,
        fixture.name,
        fixture.events,
        golden_path,
        true,
    )
    .await;
}

async fn assert_runtime_event_read_model(
    backend: FixtureBackend,
    fixture_name: String,
    events: Vec<crate::domain::agent_session::gateway::AgentRuntimeEvent>,
    golden_path: std::path::PathBuf,
    assert_reopened_projection: bool,
) {
    let fixture_label = format!("{backend:?}/{fixture_name}");
    let temp = tempfile::tempdir().expect("wire replay app data");
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(AgentSessionEventCodec));
    let local_store = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: temp.path().to_path_buf(),
        clock: Arc::new(FakeStoreClock::at(1_000)),
        registry: Arc::new(registry),
        fault: Arc::new(FaultInjector::new()),
    })
    .expect("wire replay SQLite store");
    let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
    let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
        local_store.clone();
    session_store.set_local_event_repository_with_projection_codec(
        repository,
        local_store.generation_id().to_string(),
        Arc::new(AgentSessionProjectionCodecV1),
    );
    let projected = Arc::new(Mutex::new(None));
    session_store.set_projected_read_model_hook_for_test({
        let projected = Arc::clone(&projected);
        Arc::new(move |_, model| {
            if model.workflow_turn_complete.is_some() {
                *projected.lock().unwrap() = Some(model.clone());
            }
        })
    });
    let (usecase, controller) =
        super::build_agent_runtime_usecase_with_controller(Arc::clone(&session_store), temp.path());
    let backend_id = match backend {
        FixtureBackend::Claude => "claude",
        FixtureBackend::Codex => "codex",
    };
    let response = usecase
        .send_message(SendAgentMessageRequest {
            chat_session_id: None,
            worktree_path: temp.path().to_string_lossy().to_string(),
            content: "<USER_MESSAGE>".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            backend_id: Some(backend_id.to_string()),
            model_id: None,
            images: None,
            mentions: None,
            editor_context: None,
        })
        .await
        .expect("start fixture turn");
    let session_id = response.session.id;
    let prompt_message_id = response.human_message.id;
    let assistant_message_id = response.agent_message.expect("assistant message").id;

    for event in events {
        controller
            .emit(&session_id, event)
            .unwrap_or_else(|error| panic!("{fixture_label} runtime replay failed: {error}"));
    }
    let read_model = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(model) = projected.lock().unwrap().clone() {
                break model;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{fixture_label} did not reach the production terminal projector"));
    let sqlite_events = session_store
        .load_session_events(temp.path(), &session_id)
        .expect("read fixture events from SQLite");
    assert!(sqlite_events.iter().any(|event| matches!(
        event,
        crate::usecase::agent_session::event_log::AgentSessionEvent::TurnCompleted { .. }
            | crate::usecase::agent_session::event_log::AgentSessionEvent::TurnInterrupted { .. }
    )));
    let read_model =
        normalize_runtime_identities(read_model, &prompt_message_id, &assistant_message_id);
    let live_snapshot = pretty_json(&read_model_snapshot(&read_model));
    assert_golden(&golden_path, &live_snapshot);

    if assert_reopened_projection {
        let reopened_read_store =
            LocalEventReadStore::open(temp.path()).expect("reopen wire replay SQLite reader");
        let reopened_session_store = SessionStore::new(Arc::new(FileSessionStorage::default()));
        let generation_id = reopened_read_store.generation_id().to_string();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            reopened_read_store;
        reopened_session_store.set_local_event_repository_with_projection_codec(
            repository,
            generation_id,
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let reopened_events = reopened_session_store
            .load_session_events(temp.path(), &session_id)
            .expect("read fixture events after reopening SQLite");
        let reopened_read_model = normalize_runtime_identities(
            TurnEventLog::from_events(reopened_events).project(),
            &prompt_message_id,
            &assistant_message_id,
        );
        let reopened_snapshot = pretty_json(&read_model_snapshot(&reopened_read_model));
        assert_eq!(
            reopened_snapshot, live_snapshot,
            "{fixture_label} reopened SQLite projection diverged from the live projection"
        );
        assert_golden(&golden_path, &reopened_snapshot);
    }
}

fn normalize_runtime_identities(
    mut read_model: crate::usecase::agent_session::event_log::SessionReadModel,
    prompt_message_id: &str,
    assistant_message_id: &str,
) -> crate::usecase::agent_session::event_log::SessionReadModel {
    for message in &mut read_model.messages {
        if message.id == prompt_message_id && message.role == MessageRole::Human {
            message.id = PROMPT_MESSAGE_ID.to_string();
        } else if message.id == assistant_message_id && message.role == MessageRole::Agent {
            message.id = ASSISTANT_MESSAGE_ID.to_string();
        }
        message.timestamp = FIXTURE_TIMESTAMP;
        message.streaming_final_seq = 0;
    }
    read_model
}

fn read_model_snapshot(
    read_model: &crate::usecase::agent_session::event_log::SessionReadModel,
) -> ReadModelSnapshot {
    use crate::usecase::agent_session::event_log::{
        AgentTurnFailureSignal, BackendSessionRecoveryProjection,
    };

    let workflow_turn_complete =
        read_model
            .workflow_turn_complete
            .as_ref()
            .map(|input| WorkflowTurnCompleteSnapshot {
                turn_id: input.turn_id,
                exit_code: input.exit_code,
                final_text_parts: input.final_text_parts.clone(),
                failure_signal: input.failure_signal.map(|signal| match signal {
                    AgentTurnFailureSignal::ModelRefusal => "ModelRefusal",
                }),
                token_usage: input.token_usage.map(|usage| TokenUsageSnapshot {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                }),
                interrupted: input.interrupted,
            });
    let backend_recovery = read_model
        .backend_recovery
        .as_ref()
        .map(|recovery| match recovery {
            BackendSessionRecoveryProjection::Recovering {
                recovery_id,
                old_provider_session_generation,
                reason,
            } => serde_json::json!({"Recovering": {
                "recovery_id": recovery_id,
                "old_provider_session_generation": old_provider_session_generation,
                "reason": format!("{reason:?}"),
            }}),
            BackendSessionRecoveryProjection::ReconciliationRequired { recovery_id, error } => {
                serde_json::json!({"ReconciliationRequired": {
                    "recovery_id": recovery_id,
                    "error": error,
                }})
            }
        });

    ReadModelSnapshot {
        messages: read_model
            .messages
            .iter()
            .map(crate::adaptor::gateway::agent_session::session_storage::StoredChatMessageV1::from)
            .collect(),
        status: StatusSnapshot {
            session_state: match read_model.status.session_state {
                crate::usecase::agent_session::session::SessionState::Active => "active",
                crate::usecase::agent_session::session::SessionState::Idle => "idle",
                crate::usecase::agent_session::session::SessionState::Done => "done",
                crate::usecase::agent_session::session::SessionState::Error => "error",
                crate::usecase::agent_session::session::SessionState::Closed => "closed",
                crate::usecase::agent_session::session::SessionState::Archived => "archived",
            },
            turn_phase: read_model.status.turn_phase,
        },
        error_reason: read_model.error_reason.clone(),
        queue_paused_at: read_model.queue_paused_at,
        workflow_turn_complete,
        tool_retries: read_model
            .tool_retries
            .iter()
            .map(|retry| ToolRetrySnapshot {
                turn_id: retry.turn_id,
                tool_use_id: retry.tool_use_id.clone(),
                attempt: retry.attempt,
            })
            .collect(),
        backend_recovery,
    }
}
