use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::agent_session::aggregates::backend_recovery_attempt::BackendRecoveryAttempt;
use crate::domain::agent_session::aggregates::provider_establishment::{
    ProviderEstablishmentObservation, ProviderRuntime,
};
use crate::domain::agent_session::aggregates::runtime_admission::{
    queue_drain_is_admitted, QueueDrainFacts, RuntimeSessionAdmission,
};
use crate::domain::agent_session::aggregates::runtime_permission::{
    PermissionWaitDiagnostic, RuntimePermission,
};
use crate::domain::agent_session::aggregates::runtime_progress::{
    RuntimeProgress, RuntimeStallDecision,
};
use crate::domain::agent_session::aggregates::runtime_queue::RuntimeQueuePause;
use crate::domain::agent_session::aggregates::runtime_stream_buffer::{
    PendingStreamFacts, RuntimeStreamBuffer, StreamApplyPlan, StreamFlushBatch,
};
use crate::domain::agent_session::aggregates::runtime_stream_retries::{
    RuntimeStreamRetries, StreamRetryIdentity,
};
use crate::domain::agent_session::aggregates::runtime_stream_sequence::RuntimeStreamSequence;
use crate::domain::agent_session::aggregates::runtime_streaming_delivery::{
    RuntimeStreamingDelivery, StreamEmitFailureDecision,
};
use crate::domain::agent_session::aggregates::runtime_turn::{
    RuntimeFatalObservation, RuntimeTurn, RuntimeTurnOwnership, RuntimeTurnStartCommit,
};
use crate::domain::agent_session::entities::MessagePart as DomainMessagePart;
use crate::domain::agent_session::gateway::AgentSessionRuntime;
pub(crate) use crate::domain::agent_session::value_objects::TurnPhase;
use crate::usecase::agent_session::session::{
    ChatMessage, MessagePart, PermissionRequestMsg, TokenUsage,
};

use super::ports::AgentStreamingDeltaPayload;
use super::queue::QueuedTurnInput;

pub(crate) struct RuntimeSessionState {
    pub backend_id: String,
    pub runtime: Option<Arc<dyn AgentSessionRuntime>>,
    runtime_admission: RuntimeSessionAdmission,
    runtime_turn: RuntimeTurn,
    pub streaming_message_id: Option<String>,
    pub last_agent_message_id: Option<String>,
    stream_buffer: RuntimeStreamBuffer,
    stream_sequence: RuntimeStreamSequence,
    stream_retries: RuntimeStreamRetries<PendingStreamDelta>,
    stream_delivery: RuntimeStreamingDelivery,
    pub last_stream_emit_at: Option<Instant>,
    pub last_stream_persist_at: Option<Instant>,
    /// Provider payload cache only. Durable permission ownership is restored
    /// through `AgentSessionLifecycleRepository` before a response is admitted.
    pub permission_request_cache: Option<PermissionRequestMsg>,
    /// Provider-effect payloads keyed by their canonical queue identity.
    /// Ordering and admission are never inferred from this process-local map.
    pub accepted_input_effects: HashMap<String, QueuedTurnInput>,
    queue_pause: RuntimeQueuePause,
    pub current_turn_input: Option<QueuedTurnInput>,
    pub latest_token_usage: Option<TokenUsage>,
    runtime_progress: RuntimeProgress,
    permission_state: RuntimePermission,
    provider_runtime: ProviderRuntime,
    pub backend_recovery: Option<BackendSessionRecoveryState>,
}

pub(crate) struct BackendSessionRecoveryState {
    pub attempt: BackendRecoveryAttempt,
    pub completion: tokio::sync::watch::Sender<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStreamDelta {
    pub message_id: String,
    pub seq: u64,
    pub snapshot: bool,
    pub parts: Vec<MessagePart>,
    pub message: Option<ChatMessage>,
    /// Final/backend-owned snapshots replace an older retry payload when delivery fails.
    pub authoritative: bool,
}

impl PendingStreamDelta {
    pub(crate) fn to_delta_payload(&self, session_id: &str) -> AgentStreamingDeltaPayload {
        AgentStreamingDeltaPayload {
            chat_session_id: session_id.to_string(),
            message_id: self.message_id.clone(),
            seq: self.seq,
            snapshot: self.snapshot,
            parts: self.parts.clone(),
            message: self.message.clone(),
        }
    }
}

impl StreamRetryIdentity for PendingStreamDelta {
    fn message_id(&self) -> &str {
        &self.message_id
    }

    fn sequence(&self) -> u64 {
        self.seq
    }
}

impl RuntimeSessionState {
    pub(crate) fn new(backend_id: String) -> Self {
        Self::with_queue_pause(backend_id, None)
    }

    pub(crate) fn with_queue_pause(backend_id: String, queue_paused_at: Option<f64>) -> Self {
        Self {
            backend_id,
            runtime: None,
            runtime_admission: RuntimeSessionAdmission::default(),
            runtime_turn: RuntimeTurn::default(),
            streaming_message_id: None,
            last_agent_message_id: None,
            stream_buffer: RuntimeStreamBuffer::default(),
            stream_sequence: RuntimeStreamSequence::default(),
            stream_retries: RuntimeStreamRetries::default(),
            stream_delivery: RuntimeStreamingDelivery::default(),
            last_stream_emit_at: None,
            last_stream_persist_at: None,
            permission_request_cache: None,
            accepted_input_effects: HashMap::new(),
            queue_pause: RuntimeQueuePause::restore(queue_paused_at),
            current_turn_input: None,
            latest_token_usage: None,
            runtime_progress: RuntimeProgress::default(),
            permission_state: RuntimePermission::default(),
            provider_runtime: ProviderRuntime::default(),
            backend_recovery: None,
        }
    }

    pub(crate) fn bump_runtime_epoch(&mut self) -> u64 {
        self.provider_runtime.bump_epoch()
    }

    pub(crate) fn runtime_epoch(&self) -> u64 {
        self.provider_runtime.epoch()
    }

    pub(crate) fn owns_runtime_epoch(&self, expected: u64) -> bool {
        self.provider_runtime.owns_epoch(expected)
    }

    pub(crate) fn register_turn_start_intent(&mut self, turn_id: u64, message_id: String) -> u64 {
        self.streaming_message_id = Some(message_id.clone());
        self.runtime_turn.register_start(turn_id)
    }

    pub(crate) fn commit_turn_start(&mut self, message_id: String) {
        self.streaming_message_id = Some(message_id.clone());
        self.last_agent_message_id = Some(message_id);
        self.stream_buffer.reset();
        self.stream_sequence.reset();
        self.stream_retries.reset_regular();
        self.stream_delivery.reset_regular();
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.runtime_turn.clear_trailing_fatal();
        self.clear_pending_permission_request();
        self.current_turn_input = None;
        let now = Instant::now();
        self.runtime_progress.start_turn(now);
    }

    pub(crate) fn reset_for_turn(&mut self, turn_id: u64, message_id: String) {
        self.register_turn_start_intent(turn_id, message_id.clone());
        self.commit_turn_start(message_id);
    }

    pub(crate) fn mark_progress(&mut self, at: Instant) -> bool {
        self.runtime_progress.mark_progress(at)
    }

    pub(crate) fn record_progress(&mut self, at: Instant) -> bool {
        self.runtime_progress.record_progress(at)
    }

    pub(crate) fn last_progress_at(&self) -> Option<Instant> {
        self.runtime_progress.last_progress_at()
    }

    pub(crate) fn turn_started_at(&self) -> Option<Instant> {
        self.runtime_progress.turn_started_at()
    }

    pub(crate) fn clear_stall_observation(&mut self) {
        self.runtime_progress.clear_stall_observation();
    }

    #[cfg(test)]
    pub(crate) fn stall_observation_is_active(&self) -> bool {
        self.runtime_progress.stall_observation_is_active()
    }

    pub(crate) fn observe_stall(
        &mut self,
        has_runtime: bool,
        now: Instant,
    ) -> RuntimeStallDecision {
        self.runtime_progress.observe_stall(has_runtime, now)
    }

    pub(crate) fn record_first_backend_event(
        &mut self,
        now: Instant,
    ) -> Option<std::time::Duration> {
        self.runtime_progress
            .record_first_backend_event(self.active_turn_id().is_some(), now)
    }

    pub(crate) fn finish_turn_progress(&mut self) {
        self.runtime_progress.finish_turn();
    }

    #[cfg(test)]
    pub(crate) fn restore_runtime_progress_for_test(
        &mut self,
        last_progress_at: Option<Instant>,
        stall_signal_count: u32,
        stall_recovery_attempts: u32,
        stall_observation_active: bool,
    ) {
        self.runtime_progress.restore_for_test(
            last_progress_at,
            stall_signal_count,
            stall_recovery_attempts,
            stall_observation_active,
        );
    }

    #[cfg(test)]
    pub(crate) fn stall_signal_count_for_test(&self) -> u32 {
        self.runtime_progress.stall_signal_count()
    }

    pub(crate) fn rollback_started_turn(&mut self) {
        self.runtime_turn.rollback_start();
        self.streaming_message_id = None;
        self.clear_pending_permission_request();
        self.current_turn_input = None;
        self.stream_buffer.reset();
        self.stream_retries.reset_regular();
        self.stream_delivery.reset_regular();
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.runtime_turn.clear_trailing_fatal();
        self.runtime_progress.clear_turn();
    }

    pub(crate) fn set_pending_permission_request(&mut self, request: PermissionRequestMsg) -> u64 {
        let request_id = request.id.clone();
        self.permission_request_cache = Some(request);
        self.permission_state
            .begin_wait(request_id, std::time::Instant::now())
    }

    pub(crate) fn clear_pending_permission_request(&mut self) -> u64 {
        self.permission_request_cache = None;
        self.permission_state.clear()
    }

    pub(crate) fn resolve_pending_permission_request(
        &mut self,
        now: std::time::Instant,
    ) -> (u64, Option<std::time::Duration>) {
        self.permission_request_cache = None;
        self.permission_state.resolve(now)
    }

    pub(crate) fn pending_permission_state_revision(&self) -> u64 {
        self.permission_state.revision()
    }

    pub(crate) fn owns_pending_permission_request(&self, request_id: &str) -> bool {
        self.permission_state.owns_pending_request(request_id)
    }

    pub(crate) fn report_permission_request_observed(
        &mut self,
        request_id: &str,
        visible: bool,
        at: std::time::Instant,
    ) {
        self.permission_state
            .report_visibility(request_id, visible, at);
    }

    pub(crate) fn mark_permission_wait_diagnostic_if_due(
        &mut self,
        now: std::time::Instant,
        threshold: std::time::Duration,
        observed_ttl: std::time::Duration,
    ) -> Option<PermissionWaitDiagnostic> {
        self.permission_state
            .mark_diagnostic_if_due(now, threshold, observed_ttl)
    }

    #[cfg(test)]
    pub(crate) fn permission_wait_diagnostic_emitted(&self) -> bool {
        self.permission_state.diagnostic_emitted()
    }

    #[cfg(test)]
    pub(crate) fn visible_permission_request_id(&self) -> Option<&str> {
        self.permission_state.visible_request_id()
    }

    #[cfg(test)]
    pub(crate) fn begin_permission_wait_for_test(
        &mut self,
        request_id: &str,
        started_at: std::time::Instant,
    ) {
        self.permission_state
            .begin_wait(request_id.to_string(), started_at);
    }

    pub(crate) fn active_turn_id(&self) -> Option<u64> {
        self.runtime_turn.active_turn_id()
    }

    pub(crate) fn has_active_turn_lease(&self) -> bool {
        self.runtime_turn.has_active_turn()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.runtime_turn.generation()
    }

    #[cfg(test)]
    pub(crate) fn last_turn_id(&self) -> Option<u64> {
        self.runtime_turn.last_turn_id()
    }

    pub(crate) fn owns_generation(&self, generation: u64) -> bool {
        self.runtime_turn.owns_generation(generation)
    }

    pub(crate) fn matches_generation(&self, generation: u64) -> bool {
        self.runtime_turn.matches_generation(generation)
    }

    pub(crate) fn owns_optional_generation(&self, expected: Option<u64>) -> bool {
        self.runtime_turn.owns_optional_generation(expected)
    }

    pub(crate) fn owns_turn(&self, generation: u64, turn_id: u64) -> bool {
        self.runtime_turn.owns_turn(generation, turn_id)
    }

    pub(crate) fn owns_active_turn_id(&self, turn_id: u64) -> bool {
        self.runtime_turn.owns_active_turn_id(turn_id)
    }

    pub(crate) fn request_interrupt(&mut self) -> RuntimeTurnOwnership {
        self.runtime_turn.request_interrupt()
    }

    pub(crate) fn clear_interrupt_request(&mut self) {
        self.runtime_turn.clear_interrupt_request();
    }

    pub(crate) fn admits_trailing_fatal_wait(&self, has_message: bool) -> bool {
        self.runtime_turn.admits_trailing_fatal_wait(has_message)
    }

    pub(crate) fn defer_trailing_fatal(&mut self, message: Option<String>) {
        self.runtime_turn.defer_trailing_fatal(message);
    }

    pub(crate) fn observe_fatal(&mut self, message: &str) -> RuntimeFatalObservation {
        self.runtime_turn.observe_fatal(message)
    }

    pub(crate) fn interrupt_requested_for_current(&self) -> bool {
        self.runtime_turn.interrupt_requested_for_current()
    }

    pub(crate) fn interrupt_requested_for_optional_generation(
        &self,
        expected: Option<u64>,
    ) -> bool {
        self.runtime_turn
            .interrupt_requested_for_optional_generation(expected)
    }

    #[cfg(test)]
    pub(crate) fn repeated_interrupt(&self, generation: u64) -> bool {
        self.runtime_turn
            .repeated_interrupt(generation, self.queue_is_paused())
    }

    pub(crate) fn admits_provider_effect(&self, generation: u64) -> bool {
        self.runtime_turn
            .admits_provider_effect(generation, self.queue_is_paused())
    }

    pub(crate) fn should_rollback_start(&self, generation: u64) -> bool {
        self.runtime_turn.should_rollback_start(generation)
    }

    pub(crate) fn decide_start_commit(
        &self,
        generation: u64,
        turn_id: u64,
    ) -> RuntimeTurnStartCommit {
        self.runtime_turn
            .decide_start_commit(generation, turn_id, self.queue_is_paused())
    }

    pub(crate) fn mark_terminal(&mut self, turn_id: u64) -> RuntimeTurnOwnership {
        self.runtime_turn.mark_terminal(turn_id)
    }

    pub(crate) fn seal_terminal(&mut self, turn_id: u64) -> RuntimeTurnOwnership {
        self.runtime_turn.seal_terminal(turn_id)
    }

    pub(crate) fn terminal_matches_current_or_last(&self) -> bool {
        self.runtime_turn.terminal_matches_current_or_last()
    }

    /// Read-model formatting for process-local notifications. This is not
    /// consulted by command admission.
    pub(crate) fn projected_turn_phase(&self) -> TurnPhase {
        crate::domain::agent_session::services::project_runtime_turn_phase(
            self.has_active_turn_lease(),
            self.permission_request_cache.is_some(),
        )
    }

    pub(crate) fn release_turn_lease(&mut self) {
        self.runtime_turn.release();
    }

    pub(crate) fn observe_canonical_turn_identity(&mut self, turn_id: u64) {
        self.runtime_turn.observe_canonical_identity(turn_id);
    }

    #[cfg(test)]
    pub(crate) fn install_turn_lease_for_test(&mut self, phase: TurnPhase) {
        if phase.has_active_turn() {
            self.runtime_turn
                .observe_canonical_identity(self.runtime_turn.last_turn_id().unwrap_or(1));
        } else {
            self.runtime_turn.release();
        }
    }

    pub(crate) fn queue_is_paused(&self) -> bool {
        self.queue_pause.is_paused()
    }

    pub(crate) fn queue_paused_at(&self) -> Option<f64> {
        self.queue_pause.paused_at()
    }

    pub(crate) fn pause_queue_at(&mut self, at: f64) {
        self.queue_pause.pause(at);
    }

    pub(crate) fn replace_queue_pause(&mut self, paused_at: Option<f64>) {
        self.queue_pause.replace(paused_at);
    }

    pub(crate) fn merge_durable_queue_pause(&mut self, paused_at: Option<f64>) {
        self.queue_pause.merge_durable_observation(paused_at);
    }

    pub(crate) fn resume_queue_if_matches(&mut self, expected_paused_at: f64) -> bool {
        self.queue_pause.resume_if_matches(expected_paused_at)
    }

    pub(crate) fn begin_closing(&mut self) {
        self.runtime_admission.begin_closing();
    }

    pub(crate) fn cancel_closing(&mut self) {
        self.runtime_admission.cancel_closing();
    }

    pub(crate) fn is_closing(&self) -> bool {
        self.runtime_admission.is_closing()
    }

    pub(crate) fn accepts_work(&self) -> bool {
        self.runtime_admission.accepts_work()
    }

    pub(crate) fn stream_emit_is_suppressed(&self) -> bool {
        self.stream_delivery.regular_is_suppressed()
    }

    #[cfg(test)]
    pub(crate) fn stream_emit_failure_count(&self) -> u32 {
        self.stream_delivery.regular_failure_count()
    }

    #[cfg(test)]
    pub(crate) fn stream_flush_is_scheduled(&self) -> bool {
        self.stream_delivery.regular_flush_is_scheduled()
    }

    pub(crate) fn schedule_stream_flush(&mut self) -> bool {
        self.stream_delivery.schedule_regular_flush()
    }

    pub(crate) fn clear_stream_flush_schedule(&mut self) {
        self.stream_delivery.clear_regular_flush_schedule();
    }

    pub(crate) fn record_stream_emit_success(&mut self) {
        self.stream_delivery.record_regular_success();
    }

    pub(crate) fn reset_stream_delivery(&mut self) {
        self.stream_delivery.reset_regular();
    }

    pub(crate) fn finish_stream_delivery(&mut self) {
        self.stream_delivery.finish_regular_turn();
    }

    pub(crate) fn canonical_streaming_parts(&self) -> &[DomainMessagePart] {
        self.stream_buffer.canonical_parts()
    }

    pub(crate) fn reset_stream_buffer(&mut self) {
        self.stream_buffer.reset();
    }

    pub(crate) fn persisted_streaming_parts(&self) -> &[MessagePart] {
        self.stream_buffer.persisted_parts()
    }

    pub(crate) fn prepare_stream_apply(
        &self,
        delta: &[DomainMessagePart],
        immediate: bool,
    ) -> StreamApplyPlan {
        self.stream_buffer
            .prepare_apply(delta, immediate, self.stream_sequence())
    }

    pub(crate) fn commit_persisted_stream(
        &mut self,
        candidate_parts: Vec<DomainMessagePart>,
        persisted_parts: Vec<MessagePart>,
        delta: &[DomainMessagePart],
        requires_snapshot: bool,
    ) {
        self.stream_buffer.commit_persisted(
            candidate_parts,
            persisted_parts,
            delta,
            requires_snapshot,
        );
    }

    pub(crate) fn pending_stream_facts(&self) -> PendingStreamFacts {
        self.stream_buffer.pending_facts()
    }

    pub(crate) fn take_pending_stream_flush(&mut self) -> Option<StreamFlushBatch> {
        self.stream_buffer.take_flush(self.stream_sequence())
    }

    pub(crate) fn quarantine_stream_after_persist_failure(&mut self) {
        self.stream_buffer.quarantine_after_persist_failure();
    }

    pub(crate) fn stop_pending_stream_delivery(&mut self) {
        self.stream_buffer.stop_delivery();
    }

    pub(crate) fn fallback_pending_stream_to_snapshot(&mut self) {
        self.stream_buffer.fallback_to_snapshot();
    }

    pub(crate) fn patch_stream_permission_response(
        &mut self,
        response: &crate::domain::agent_session::entities::PermissionResponse,
    ) -> bool {
        self.stream_buffer.patch_permission_response(response)
    }

    pub(crate) fn has_coalesced_stream_retry(&self) -> bool {
        self.stream_retries.has_coalesced()
    }

    pub(crate) fn take_coalesced_stream_retry(&mut self) -> Option<PendingStreamDelta> {
        self.stream_retries.take_coalesced()
    }

    pub(crate) fn replace_coalesced_stream_retry(&mut self, retry: Option<PendingStreamDelta>) {
        self.stream_retries.replace_coalesced(retry);
    }

    pub(crate) fn clear_coalesced_stream_retry(&mut self) {
        self.stream_retries.clear_coalesced();
    }

    pub(crate) fn prepare_authoritative_stream_retry(&mut self, message_id: &str) {
        self.stream_retries.prepare_authoritative(message_id);
    }

    pub(crate) fn authoritative_stream_retries_are_empty(&self) -> bool {
        self.stream_retries.authoritative_is_empty()
    }

    pub(crate) fn upsert_authoritative_stream_retry(&mut self, retry: PendingStreamDelta) {
        self.stream_retries.upsert_authoritative(retry);
    }

    pub(crate) fn authoritative_stream_retry_front(&self) -> Option<&PendingStreamDelta> {
        self.stream_retries.authoritative_front()
    }

    pub(crate) fn acknowledge_authoritative_stream_retry(
        &mut self,
        message_id: &str,
        sequence: u64,
    ) -> bool {
        self.stream_retries
            .acknowledge_authoritative_front(message_id, sequence)
    }

    pub(crate) fn clear_authoritative_stream_retries(&mut self) {
        self.stream_retries.clear_authoritative();
    }

    #[cfg(test)]
    pub(crate) fn authoritative_stream_retry_count(&self) -> usize {
        self.stream_retries.authoritative_len()
    }

    pub(crate) fn owns_stream_target(&self, turn_id: u64, message_id: &str) -> bool {
        crate::domain::agent_session::services::stream_target_is_current(
            self.active_turn_id(),
            self.streaming_message_id
                .as_deref()
                .or(self.last_agent_message_id.as_deref()),
            turn_id,
            message_id,
        )
    }

    #[cfg(test)]
    pub(crate) fn restore_stream_buffer_for_test(
        &mut self,
        canonical_parts: Vec<DomainMessagePart>,
        persisted_parts: Vec<MessagePart>,
        snapshot_pending: bool,
    ) {
        self.stream_buffer
            .restore_for_test(canonical_parts, persisted_parts, snapshot_pending);
    }

    pub(crate) fn stream_sequence(&self) -> u64 {
        self.stream_sequence.current()
    }

    pub(crate) fn next_stream_sequence(&self) -> u64 {
        self.stream_sequence.next()
    }

    pub(crate) fn advance_stream_sequence(&mut self) -> u64 {
        self.stream_sequence.advance()
    }

    pub(crate) fn reset_stream_sequence(&mut self) {
        self.stream_sequence.reset();
    }

    pub(crate) fn observe_emitted_stream_sequence(&mut self, sequence: u64) {
        self.stream_sequence.observe_emitted(sequence);
    }

    pub(crate) fn record_stream_emit_failure(
        &mut self,
        has_retry: bool,
    ) -> StreamEmitFailureDecision {
        self.stream_delivery.record_regular_failure(has_retry)
    }

    pub(crate) fn authoritative_stream_flush_is_scheduled(&self) -> bool {
        self.stream_delivery.authoritative_flush_is_scheduled()
    }

    pub(crate) fn schedule_authoritative_stream_flush(&mut self) -> bool {
        self.stream_delivery.schedule_authoritative_flush()
    }

    pub(crate) fn clear_authoritative_stream_flush_schedule(&mut self) {
        self.stream_delivery.clear_authoritative_flush_schedule();
    }

    pub(crate) fn record_authoritative_stream_emit_success(&mut self) {
        self.stream_delivery.record_authoritative_success();
    }

    pub(crate) fn record_authoritative_stream_emit_failure(&mut self) -> StreamEmitFailureDecision {
        self.stream_delivery.record_authoritative_failure()
    }

    #[cfg(test)]
    pub(crate) fn suppress_stream_emit_for_test(&mut self) {
        self.stream_delivery.suppress_regular_for_test();
    }

    pub(crate) fn admits_queue_drain(&self) -> bool {
        queue_drain_is_admitted(QueueDrainFacts {
            closing: self.is_closing(),
            backend_recovery_active: self.backend_recovery.is_some(),
            active_turn: self.has_active_turn_lease(),
            queue_paused: self.queue_is_paused(),
        })
    }

    pub(crate) fn provider_session_is_established(&self) -> bool {
        self.provider_runtime.session_is_established()
    }

    pub(crate) fn has_pending_provider_establishment(&self) -> bool {
        self.provider_runtime.has_pending_establishment()
    }

    pub(crate) fn provider_establishment_is_current(&self, observation_id: &str) -> bool {
        self.provider_runtime
            .establishment_is_current(observation_id)
    }

    pub(crate) fn observe_provider_establishment(
        &mut self,
        observation_id: &str,
    ) -> ProviderEstablishmentObservation {
        self.provider_runtime.observe_establishment(observation_id)
    }

    pub(crate) fn clear_provider_establishment_if_current(&mut self, observation_id: &str) -> bool {
        self.provider_runtime
            .clear_establishment_if_current(observation_id)
    }

    pub(crate) fn settle_provider_establishment_if_current(
        &mut self,
        observation_id: &str,
    ) -> bool {
        self.provider_runtime
            .settle_establishment_if_current(observation_id)
    }

    pub(crate) fn mark_provider_session_established(&mut self) {
        self.provider_runtime.mark_session_established();
    }
}

pub(crate) type RuntimeSessionMap = HashMap<String, RuntimeSessionState>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_pause_is_initialized_false_and_survives_turn_state_changes() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        assert!(!state.queue_is_paused());

        state.pause_queue_at(1.0);
        state.reset_for_turn(1, "message-1".to_string());
        assert!(state.queue_is_paused());
        assert_eq!(state.queue_paused_at(), Some(1.0));

        state.rollback_started_turn();
        assert!(state.queue_is_paused());
        assert_eq!(state.queue_paused_at(), Some(1.0));
    }

    #[test]
    fn durable_queue_pause_hydrates_runtime_state() {
        let state = RuntimeSessionState::with_queue_pause("codex".to_string(), Some(42.0));

        assert!(state.queue_is_paused());
        assert_eq!(state.queue_paused_at(), Some(42.0));
    }

    #[test]
    fn turn_start_intent_registers_ownership_before_committing_turn_state() {
        let mut state = RuntimeSessionState::new("codex".to_string());

        let generation = state.register_turn_start_intent(7, "message-7".to_string());

        assert_eq!(generation, 1);
        assert_eq!(state.projected_turn_phase(), TurnPhase::Streaming);
        assert_eq!(state.active_turn_id(), Some(7));
        assert_eq!(state.last_turn_id(), Some(7));
        assert_eq!(state.streaming_message_id.as_deref(), Some("message-7"));
        assert!(state.turn_started_at().is_none());
        assert!(state.last_progress_at().is_none());

        state.commit_turn_start("message-7".to_string());

        assert_eq!(state.generation(), generation);
        assert!(state.turn_started_at().is_some());
        assert!(state.last_progress_at().is_some());
    }
}
