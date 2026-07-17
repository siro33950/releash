use crate::domain::agent_session::entities::{
    merge_part, InterruptReason as DomainInterruptReason, MessagePart as DomainMessagePart,
    TokenUsage as DomainTokenUsage, TurnResult as DomainTurnResult,
    TurnStopReason as DomainTurnStopReason,
};
use crate::domain::agent_session::gateway::AgentRuntimeEvent;
use crate::infrastructure::agent_session::fixtures::{
    assert_golden, pretty_json, replay_backend, FixtureBackend, ReplayedFixture,
};
use crate::usecase::agent_session::event_log::{
    AgentSessionEvent, InterruptReason, PartEventMode, PromptInput, TurnEventLog, TurnStopReason,
    TurnTokenUsage,
};
use crate::usecase::agent_session::runtime::event_apply::parts_from_domain;

const TURN_ID: u64 = 1;
const ASSISTANT_MESSAGE_ID: &str = "<ASSISTANT_MESSAGE_ID>";

#[test]
fn claude_fixture_matches_read_model_golden() {
    for fixture in replay_backend(FixtureBackend::Claude) {
        assert_fixture_read_model(fixture);
    }
}

#[test]
fn codex_fixture_matches_read_model_golden() {
    for fixture in replay_backend(FixtureBackend::Codex) {
        assert_fixture_read_model(fixture);
    }
}

fn assert_fixture_read_model(fixture: ReplayedFixture) {
    let golden_path = fixture.read_model_golden_path();
    let fixture_label = format!("{:?}/{}", fixture.backend, fixture.name);
    let mut log = started_turn_log();
    let mut final_domain_parts = Vec::new();
    let mut latest_usage = None;
    let mut completed = false;

    for event in fixture.events {
        match event {
            AgentRuntimeEvent::PartsMerged(parts) => {
                apply_domain_parts(&mut log, &mut final_domain_parts, parts);
            }
            AgentRuntimeEvent::PermissionRequested(request) => {
                apply_domain_parts(
                    &mut log,
                    &mut final_domain_parts,
                    vec![DomainMessagePart::Permission { request }],
                );
            }
            AgentRuntimeEvent::TokenUsageUpdated(usage) => latest_usage = Some(usage),
            AgentRuntimeEvent::TurnCompleted(result) => {
                append_final_parts(&mut log, &final_domain_parts);
                append_terminal_event(&mut log, result, latest_usage);
                completed = true;
            }
            AgentRuntimeEvent::Fatal { message } => {
                panic!("{fixture_label} emitted unsupported fatal event: {message}")
            }
            AgentRuntimeEvent::SessionEstablished { .. }
            | AgentRuntimeEvent::BackendSessionCleared
            | AgentRuntimeEvent::PermissionModeChanged(_)
            | AgentRuntimeEvent::SlashCommandsUpdated(_)
            | AgentRuntimeEvent::KeepAlive => {}
        }
    }

    assert!(completed, "{fixture_label} did not complete its turn");
    let read_model = log.project();
    assert_golden(&golden_path, &pretty_json(&read_model));
}

fn apply_domain_parts(
    log: &mut TurnEventLog,
    final_domain_parts: &mut Vec<DomainMessagePart>,
    parts: Vec<DomainMessagePart>,
) {
    for part in &parts {
        merge_part(final_domain_parts, part.clone());
    }
    let durable_parts = parts_from_domain(parts);
    log.append_part_events(
        TURN_ID,
        ASSISTANT_MESSAGE_ID,
        &durable_parts,
        PartEventMode::DurableOnly,
    );
}

fn started_turn_log() -> TurnEventLog {
    let mut log = TurnEventLog::default();
    log.begin_turn(
        TURN_ID,
        "<PROMPT_MESSAGE_ID>".to_string(),
        ASSISTANT_MESSAGE_ID.to_string(),
        PromptInput {
            content: "<USER_MESSAGE>".to_string(),
            mentions: Vec::new(),
            attachment_refs: Vec::new(),
            parts: Vec::new(),
        },
        1_700_000_000.0,
    );
    log
}

fn append_final_parts(log: &mut TurnEventLog, final_domain_parts: &[DomainMessagePart]) {
    log.append(AgentSessionEvent::FinalPartsRecorded {
        turn_id: TURN_ID,
        message_id: ASSISTANT_MESSAGE_ID.to_string(),
        parts: parts_from_domain(final_domain_parts.to_vec()),
    });
}

fn append_terminal_event(
    log: &mut TurnEventLog,
    result: DomainTurnResult,
    latest_usage: Option<DomainTokenUsage>,
) {
    match result {
        DomainTurnResult::Completed {
            stop_reason,
            token_usage,
        } => log.append(AgentSessionEvent::TurnCompleted {
            turn_id: TURN_ID,
            exit_code: 0,
            stop_reason: stop_reason.map(map_stop_reason),
            token_usage: token_usage.or(latest_usage).map(map_token_usage),
        }),
        DomainTurnResult::Failed { token_usage, .. } => {
            log.append(AgentSessionEvent::TurnCompleted {
                turn_id: TURN_ID,
                exit_code: 1,
                stop_reason: None,
                token_usage: token_usage.or(latest_usage).map(map_token_usage),
            });
        }
        DomainTurnResult::Interrupted { reason, error } => log.finalize(
            TURN_ID,
            map_interrupt_reason(reason),
            error,
            interrupt_exit_code(reason),
        ),
    }
}

fn map_stop_reason(reason: DomainTurnStopReason) -> TurnStopReason {
    match reason {
        DomainTurnStopReason::Refusal => TurnStopReason::Refusal,
    }
}

fn map_token_usage(usage: DomainTokenUsage) -> TurnTokenUsage {
    TurnTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn map_interrupt_reason(reason: DomainInterruptReason) -> InterruptReason {
    match reason {
        DomainInterruptReason::Abort => InterruptReason::Abort,
        DomainInterruptReason::Timeout => InterruptReason::Timeout,
        DomainInterruptReason::Crash => InterruptReason::Crash,
    }
}

fn interrupt_exit_code(reason: DomainInterruptReason) -> i64 {
    match reason {
        DomainInterruptReason::Abort => 0,
        DomainInterruptReason::Timeout => 124,
        DomainInterruptReason::Crash => 1,
    }
}
