mod external_agent;
mod model_selection;
mod permission;
mod process_registry;
mod recovery;
mod sdk_message;
mod session_lifecycle;
mod session_persistence;
mod shared;
mod skills;
mod stream_emit;
mod turn_event_log;

pub(crate) use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;
pub(crate) use external_agent::{
    close_external_agent_process, finish_external_pending_message_turn_start,
    prepare_external_pending_message_turn, register_external_agent_process,
    start_external_agent_turn_state, ExternalAgentTurnStart, ExternalBridgeMessageState,
};
pub(crate) use model_selection::set_agent_model;
pub(crate) use permission::{respond_agent_permission, set_agent_permission_mode};
#[cfg(test)]
pub(crate) use process_registry::make_test_agent_process;
pub(crate) use process_registry::{AgentProcessMap, TurnPhase};
#[cfg(test)]
pub(crate) use process_registry::{BridgeState, PendingMessage};
#[cfg(unix)]
pub(crate) use recovery::{cleanup_orphan_processes, OrphanCleanupReport};
pub(crate) use recovery::{wait_for_startup_orphan_cleanup, CleanupGate};
pub(crate) use sdk_message::handle_external_bridge_message;
pub(crate) use session_lifecycle::{
    cancel_agent_queued_turn_internal, close_agent_session_internal, close_all_agent_sessions,
    get_session, get_session_page_internal_with_data_dir, init_agent_sessions,
    interrupt_agent_query, send_agent_message_internal, set_session_backend,
    start_agent_session_internal, start_agent_turn_internal_locked, CancelQueuedTurnResponse,
    InitSessionsResponse, SendMessageResponse,
};
pub(crate) use session_persistence::{
    persist_context_carry_failed_after_init_error, persist_context_carry_state,
};
pub(crate) use shared::{
    is_agent_step_runtime_busy, notify_status_transition, session_specific_env_overrides,
    write_bridge_command, CLAUDE_BACKEND_ID, CODEX_BACKEND_ID,
    DEFER_AGENT_SESSION_ID_PERSIST_ON_READY,
};
pub(crate) use skills::{
    prepare_image_attachment, prepare_image_attachments_from_paths, scan_agent_skills_inner,
};
