//! Ports the operation usecases require from adapters.
//!
//! The binding authority owns the installation HMAC key (owner-only; never
//! logged, never in DTOs). Gates connect the acceptance contracts to the
//! existing runtime without letting the usecase depend on gateways.

use crate::domain::agent_session::events::SendDisposition;
use crate::domain::local_event::SafeOperationFailure;

/// Gateway-owned canonicalization of a typed recovery result.
///
/// Versioned stored/public encodings and their canonical hash remain behind
/// this port. Usecases supply only closed semantic values.
pub trait RecoveryResultCanonicalizer: Send + Sync {
    fn canonicalize_recovery_result(
        &self,
        _outcome: crate::domain::local_event::RecoveryResultOutcomeRecord,
        _classification: crate::domain::agent_session::events::RecoveryResultClassification,
        _resource_revision: u64,
        _resource_view: crate::domain::local_event::RecoveryResourceViewRecord,
    ) -> Result<crate::domain::local_event::RecoveryResultRecord, ()> {
        Err(())
    }
}

/// Owner-only installation key operations. The key never leaves the
/// implementation; the usecase only sees derived MACs / digests.
pub trait OperationBindingAuthority: RecoveryResultCanonicalizer + Send + Sync {
    /// Keyed MAC (HMAC-SHA256 with the installation key) over binding
    /// material.
    fn mac(&self, message: &[u8]) -> [u8; 32];

    /// Unkeyed content digest (SHA-256) for idempotency payload hashes.
    fn digest(&self, message: &[u8]) -> [u8; 32];

    /// Seal owner-private retry material. `context` is authenticated but not
    /// stored in the envelope, binding ciphertext to principal, generation,
    /// operation kind, and caller identity.
    fn seal_command(&self, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()>;

    /// Open retry material previously produced for the exact same context.
    fn open_command(&self, context: &[u8], envelope: &[u8]) -> Result<Vec<u8>, ()>;
}

/// Plan produced before the send acceptance commit. Producing a plan must
/// not start any provider I/O.
#[derive(Debug, Clone)]
pub struct SendPlan {
    pub session_id: String,
    /// Fully resolved initial shell for a new-session send. Existing-session
    /// sends leave this empty. It is projected in the acceptance batch and is
    /// never pre-written to another store.
    pub initial_session: Option<crate::usecase::agent_session::session::ChatSession>,
    /// Exact canonical projection revision from which disposition and turn
    /// identity were allocated. The acceptance projection must commit with
    /// this guard rather than rebasing the allocation onto newer queue state.
    pub session_projection_guard: crate::domain::local_event::RevisionGuard,
    /// One-shot disposition decided at acceptance time; immutable afterwards.
    pub disposition: SendDisposition,
    /// Opaque reference to the durably saved exact input.
    pub input_ref: String,
    /// Deterministic human message identity and semantic prompt fixed in the
    /// same acceptance batch as the disposition and obligations.
    pub human_message_id: String,
    pub prompt: crate::domain::agent_session::events::PromptInput,
    /// Reserved turn identity for a queued disposition.
    pub reserved_turn_id: Option<String>,
}

/// Identity of the provider effect an accepted send is allowed to start.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedSendEffect {
    pub operation_id: String,
    pub session_id: String,
    pub human_message_id: String,
    /// Assistant projection reserved by the acceptance batch for an
    /// immediately-started turn. Queued sends do not create it yet.
    pub assistant_message_id: Option<String>,
    pub disposition: SendDisposition,
    pub reserved_turn_id: Option<String>,
    /// TurnExecution obligation identifying the turn / queued execution.
    pub execution_obligation_id: String,
    /// Exact accepted input copied into the immutable obligation. Workers
    /// reconstruct provider input from this value after process restart;
    /// no process-local request journal is authoritative.
    pub canonical_payload: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendEffectDispatch {
    /// This call installed the process-local worker that owns the handoff.
    Scheduled,
    /// The exact queued effect already has a process-local handoff owner.
    AlreadyScheduled,
}

/// Adapter-owned classification for ProviderEstablish obligations written by
/// the superseded two-flight send protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProviderEstablishRecovery {
    /// The old establish reservation cannot have submitted this turn's input,
    /// and the turn may continue without replaying that reservation.
    ContinueTurnExecution,
    /// Repeating provider establishment could create another remote identity;
    /// keep the old reservation available for explicit reconciliation.
    RequiresManualResolution,
}

/// Runtime-side admission gate for normal sends.
#[async_trait::async_trait]
pub trait SendAdmissionGate: Send + Sync {
    /// Resolve the target session and decide the one-shot disposition.
    /// Must not start provider I/O; failures reject before commit.
    async fn plan_send(
        &self,
        principal: &str,
        operation_id: &str,
        canonical_payload: &str,
    ) -> Result<SendPlan, SafeOperationFailure>;

    /// Build the session/message/queue projection participants that must be
    /// committed atomically with the operation receipt and obligations.
    async fn acceptance_state_mutations(
        &self,
        _plan: &SendPlan,
        _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        Ok(Vec::new())
    }

    /// Classify a legacy, already-reserved ProviderEstablish dependency.
    ///
    /// The default is deliberately conservative. Production adapters may
    /// continue only when backend-specific durable state proves that doing so
    /// cannot duplicate a remote provider identity or the accepted turn.
    async fn classify_legacy_provider_establish(
        &self,
        _session_id: &str,
    ) -> Result<LegacyProviderEstablishRecovery, SafeOperationFailure> {
        Ok(LegacyProviderEstablishRecovery::RequiresManualResolution)
    }

    /// Verify that an immediately accepted turn still owns the canonical
    /// active-turn slot immediately before its TurnExecution claim.
    ///
    /// Implementations must read backend-owned durable state. Process-local
    /// runtime phase is not sufficient after restart.
    async fn canonical_immediate_turn_is_current(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<bool, SafeOperationFailure>;

    /// Whether the exact execution is currently owned by this process. This
    /// closes the interval in which startup recovery and a live handoff can
    /// observe the same durable reservation.
    async fn owns_current_process_turn_execution(
        &self,
        _session_id: &str,
        _operation_id: &str,
        _obligation_id: &str,
    ) -> bool {
        false
    }

    /// Hand the provider effect to a background worker for a freshly
    /// committed acceptance. This method must return after scheduling and
    /// must not await provider completion. An error means no worker was
    /// scheduled and no provider I/O was attempted.
    async fn start_provider_effect(
        &self,
        effect: &AcceptedSendEffect,
    ) -> Result<SendEffectDispatch, SafeOperationFailure>;
}

/// Immutable plan fixed before a permission-response acceptance commit.
/// Planning is read-only and must never contact the provider.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionResponsePlan {
    pub session_id: String,
    pub request_id: String,
    pub turn_id: u64,
    pub response: crate::domain::agent_session::entities::PermissionResponse,
    /// Whether the pending request came from the currently loaded runtime.
    /// The post-commit mirror uses this only to emit the correct live delta;
    /// durable state is owned by the acceptance/completion batches.
    pub from_runtime_state: bool,
}

/// Exact provider effect reconstructed from a confirmed durable operation.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedPermissionResponseEffect {
    pub operation_id: String,
    pub obligation_id: String,
    pub plan: PermissionResponsePlan,
}

/// Runtime-side port for permission response operations. Only `execute` may
/// contact the provider, and it receives no caller-owned mutable input.
#[async_trait::async_trait]
pub trait PermissionResponseGate: Send + Sync {
    async fn plan_response(
        &self,
        session_id: &str,
        response: &crate::domain::agent_session::entities::PermissionResponse,
    ) -> Result<PermissionResponsePlan, SafeOperationFailure>;

    async fn completion_state_mutations(
        &self,
        _effect: &AcceptedPermissionResponseEffect,
        _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        Ok(Vec::new())
    }

    /// Pure accepted-effect handoff. Implementations must not reserve, claim,
    /// or complete durable state; the operation usecase owns those commits.
    async fn execute(
        &self,
        effect: &AcceptedPermissionResponseEffect,
    ) -> Result<(), SafeOperationFailure>;

    /// Update process-local mirrors and notifications after the completion
    /// batch is durably confirmed. Failure here never rolls back completion.
    async fn after_completion(&self, _effect: &AcceptedPermissionResponseEffect) {}
}

/// Lifecycle classification of the target session at snapshot time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleState {
    Open {
        idle: bool,
        active_turn_id: Option<u64>,
    },
    Closed,
    Archived,
}

/// Bounded snapshot the lifecycle command validates admission against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLifecycleSnapshot {
    pub session_revision: i64,
    pub lifecycle: SessionLifecycleState,
    pub queue_paused: bool,
    /// Whether a live runtime exists and therefore requires a post-commit
    /// close effect. A stored provider session identity alone is not a live
    /// runtime and must not cause an eager resume merely to close it again.
    pub has_runtime: bool,
    pub has_pending_permission: bool,
    pub has_pending_recovery: bool,
    pub has_pending_provider_operation: bool,
}

/// The runtime effect of an accepted session lifecycle operation.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLifecycleEffect {
    pub operation_id: String,
    pub session_id: String,
    pub action: super::lifecycle::SessionLifecycleAction,
    pub active_turn_id: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerminalParticipants {
    pub events: Vec<crate::domain::agent_session::events::AgentSessionDomainEvent>,
    pub mutations: Vec<crate::domain::local_event::LocalStateMutation>,
}

/// Runtime-side gate for session lifecycle commands.
#[async_trait::async_trait]
pub trait SessionLifecycleGate: Send + Sync {
    /// Read the current bounded snapshot of the target session. `Err` maps
    /// to a pre-acceptance `Failed { failure }` rejection with zero effects.
    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionLifecycleSnapshot, SafeOperationFailure>;

    /// Derive all bounded session/message/permission/queue projection rows
    /// for the acceptance event set. Active close/archive terminal closure is
    /// prepared here but committed only by the lifecycle usecase batch.
    async fn acceptance_state_mutations(
        &self,
        _session_id: &str,
        _action: &super::lifecycle::SessionLifecycleAction,
        _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        Ok(Vec::new())
    }

    /// Add operation-owned participants when lifecycle acceptance itself is
    /// the terminal winner for an active turn (Close / ArchiveOpen).
    async fn terminal_participants(
        &self,
        _terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<TerminalParticipants, SafeOperationFailure> {
        Ok(TerminalParticipants::default())
    }

    /// Execute the runtime side of an accepted operation (close runtime,
    /// archive, switch backend). Called only after the acceptance commit.
    async fn execute(&self, effect: &SessionLifecycleEffect) -> Result<(), SafeOperationFailure>;
}

/// Bounded runtime snapshot used to accept a Stop without starting an effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTargetSnapshot {
    pub session_revision: u64,
    pub active_turn_id: String,
    pub queue_paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedStopEffect {
    pub operation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub obligation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopEffectObservation {
    pub terminal_reason: Option<crate::domain::agent_session::events::InterruptReason>,
}

#[async_trait::async_trait]
pub trait StopAdmissionGate: Send + Sync {
    async fn target_snapshot(
        &self,
        session_id: &str,
    ) -> Result<StopTargetSnapshot, SafeOperationFailure>;

    /// Build the session/message/queue projection participants for Stop
    /// acceptance. The returned mutations are committed in the same batch as
    /// `TurnInterruptRequested`, `QueuePaused`, the receipt, and obligation.
    async fn acceptance_state_mutations(
        &self,
        _session_id: &str,
        _expected_session_revision: u64,
        _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Option<Vec<crate::domain::local_event::LocalStateMutation>>, SafeOperationFailure>
    {
        Ok(Some(Vec::new()))
    }

    /// Build the final message/session/permission/queue projection
    /// participants for the terminal candidate. The terminal CAS and Stop
    /// resolution remain owned by the Stop usecase; this gate only derives
    /// bounded projection rows from the supplied canonical events.
    async fn terminal_state_mutations(
        &self,
        _session_id: &str,
        _events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        Ok(Vec::new())
    }

    /// Add non-Stop operation participants (currently the accepted send)
    /// when Stop itself supplies the winning terminal record. Stop's own
    /// record, resolution, and obligation are already prepared by the Stop
    /// usecase and therefore are deliberately not returned here.
    async fn terminal_participants(
        &self,
        _terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<TerminalParticipants, SafeOperationFailure> {
        Ok(TerminalParticipants::default())
    }

    /// Dispatches the provider interrupt after durable acceptance. A successful
    /// return confirms only the transport handoff; it is not itself a terminal
    /// observation. The Stop usecase owns the fixed ten-second deadline and
    /// waits for the durable runtime terminal winner before competing with
    /// Timeout.
    async fn interrupt(
        &self,
        effect: &AcceptedStopEffect,
    ) -> Result<StopEffectObservation, SafeOperationFailure>;

    /// Converges process-local runtime state only after the Stop-owned Timeout
    /// terminal has won its durable CAS. Implementations must fence this cleanup
    /// to the accepted `(session, turn)` so a newer turn cannot be closed.
    async fn timeout_terminal_committed(&self, _effect: &AcceptedStopEffect) {}
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEffectRequest {
    pub action_id: String,
    pub obligation_id: String,
    /// Revision and owner observed when the backend issued the action. The
    /// production adapter re-reads both immediately before handoff so a
    /// target/ownership change is rejected without starting an effect.
    pub origin_revision: u64,
    pub expected_owner: Option<String>,
    pub action: crate::domain::agent_session::events::RecoveryActionKind,
    pub immutable_obligation: crate::domain::local_event::ObligationRecord,
    /// Backend-owned proof captured with the obligation.  This is never
    /// populated from a public request.
    pub authoritative_observation: Option<AuthoritativeEffectObservation>,
}

/// Stable external-effect identity used by recovery readback.  It is fixed
/// by the accepting obligation and is never reconstructed from mutable
/// runtime/session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableRecoveryEffectIdentity(String);

impl StableRecoveryEffectIdentity {
    pub fn parse(value: String) -> Result<Self, ()> {
        if value.is_empty()
            || value.len() > 512
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopRecoveryReadbackRequest {
    pub effect_identity: StableRecoveryEffectIdentity,
    pub operation_id: String,
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCloseRecoveryReadbackRequest {
    pub effect_identity: StableRecoveryEffectIdentity,
    pub operation_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRecoveryReadbackRequest {
    pub effect_identity: StableRecoveryEffectIdentity,
    pub session_id: String,
    pub recovery_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRecoveryReadbackKind {
    TurnExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRecoveryReadbackRequest {
    pub effect_identity: StableRecoveryEffectIdentity,
    pub operation_id: String,
    pub session_id: String,
    pub kind: SendRecoveryReadbackKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeEffectObservation {
    pub effect_identity: String,
    pub origin_revision: u64,
    pub result_hash: [u8; 32],
    pub safe_view: String,
    /// Closed backend classification carried by the signed observation.
    /// A public label without this field is not sufficient evidence for
    /// `UseObservedResult` or `CancelIfSafe`.
    pub classification: crate::domain::agent_session::events::RecoveryResultClassification,
    /// Kind-specific policy proof.  Only a `ConfirmedNoEffect` observation
    /// with this bit set may expose `CancelIfSafe`.
    pub cancellable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryOwnerBatch {
    /// Stream heads read while preparing the owner events. A concurrent owner
    /// commit invalidates the whole recovery finish rather than publishing a
    /// projection/event split.
    pub expected_heads: Vec<crate::domain::local_event::ExpectedStreamHead>,
    pub events: Vec<crate::domain::local_event::UncommittedDomainEvent>,
    pub canonical_events: Vec<u8>,
    /// Stable digest of the exact event/head participant set, prepared by the
    /// owner codec rather than reconstructed from Rust Debug output.
    pub participant_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEffectResult {
    pub classification: crate::domain::agent_session::events::RecoveryResultClassification,
    /// Safe, bounded result view. Provider payloads, paths and secrets must
    /// never be returned through this seam.
    pub safe_result: String,
    /// Owner-projection changes justified by this typed readback. The recovery
    /// usecase commits these in the same batch as the action result and
    /// obligation transition. An empty vector means the read owner was already
    /// at the observed durable state.
    pub owner_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    /// Optional event-stream participants. When present, the recovery usecase
    /// commits these together with owner mutations and the action/source
    /// obligation closure in one LocalAtomicBatch.
    pub owner_batch: Option<RecoveryOwnerBatch>,
}

#[async_trait::async_trait]
pub trait StopRecoveryReadbackPort: Send + Sync {
    async fn read_stop(
        &self,
        request: &StopRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure>;
}

#[async_trait::async_trait]
pub trait SessionCloseRecoveryReadbackPort: Send + Sync {
    async fn read_session_close(
        &self,
        request: &SessionCloseRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure>;
}

#[async_trait::async_trait]
pub trait BackendRecoveryReadbackPort: Send + Sync {
    async fn read_backend_recovery(
        &self,
        request: &BackendRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure>;
}

#[async_trait::async_trait]
pub trait SendRecoveryReadbackPort: Send + Sync {
    async fn read_send(
        &self,
        request: &SendRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEffectHandoff {
    Ready,
    TargetRevisionChanged,
}

#[async_trait::async_trait]
pub trait RecoveryEffectExecutor: Send + Sync {
    /// Whether this exact obligation is owned by a live effect in the current
    /// process. Recovery discovery uses this only for the mutable live index;
    /// frozen shutdown snapshots never consult process-local ownership.
    async fn owns_current_process_effect(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &crate::domain::local_event::ObligationRecord,
    ) -> bool {
        false
    }

    /// Whether this production composition can decode and read back the exact
    /// obligation family.  Capability discovery calls this before advertising
    /// `ReadAgain`; the conservative default is deliberately unsupported.
    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &crate::domain::local_event::ObligationRecord,
    ) -> bool {
        false
    }

    /// Last bounded target guard immediately before an action attempt is
    /// reserved.  Returning `TargetRevisionChanged` is a closed, effect-zero
    /// rejection; callers must refresh the backend-issued action identity.
    async fn validate_handoff(
        &self,
        _request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectHandoff, SafeOperationFailure> {
        Ok(RecoveryEffectHandoff::Ready)
    }

    async fn execute(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure>;

    /// Runs only after the recovery action and obligation result are durable.
    /// Kept as a compatibility seam while effect-specific work is migrated to
    /// typed readback ports; production `ReadAgain` implementations do not use
    /// it to mutate owner state.
    async fn after_commit(
        &self,
        _request: &RecoveryEffectRequest,
        _classification: crate::domain::agent_session::events::RecoveryResultClassification,
    ) {
    }
}
