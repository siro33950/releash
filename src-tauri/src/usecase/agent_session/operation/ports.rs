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

pub use crate::domain::agent_session::services::LegacyProviderEstablishRecovery;

/// Runtime-side admission gate for normal sends.
#[async_trait::async_trait]
pub trait SendAcceptancePort: Send + Sync {
    /// Canonical bounded Session repository used by the command usecase for
    /// final admission. Production adapters must provide it; behavior-test
    /// fixtures may omit it when exercising only the #1499 protocol.
    fn lifecycle_repository(
        &self,
    ) -> Option<
        std::sync::Arc<
            dyn crate::domain::agent_session::repository::AgentSessionLifecycleRepository,
        >,
    > {
        None
    }

    /// Resolve the target, canonical input, and allocation identities.
    /// The command usecase applies final Session admission after this returns.
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
pub trait PermissionResponseEffectPort: Send + Sync {
    /// Reports only whether the process-local mirror owns the pending request.
    /// Session/Turn admission is decided by the domain aggregate.
    async fn request_is_runtime_owned(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool, SafeOperationFailure>;

    #[cfg(test)]
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

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleState {
    Open {
        idle: bool,
        active_turn_id: Option<u64>,
    },
    Closed,
    Archived,
}

/// Legacy test fixture shape. Production lifecycle code cannot depend on
/// this DTO; behavior tests convert it to the domain aggregate.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLifecycleSnapshot {
    pub session_revision: i64,
    pub lifecycle: SessionLifecycleState,
    pub queue_paused: bool,
    pub has_runtime: bool,
    pub has_pending_permission: bool,
    pub has_pending_recovery: bool,
    pub has_pending_provider_operation: bool,
}

#[cfg(test)]
impl SessionLifecycleSnapshot {
    pub fn restore_session(
        &self,
        session_id: &str,
    ) -> Result<
        crate::domain::agent_session::aggregates::session::Session,
        crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError,
    > {
        use crate::domain::agent_session::aggregates::session::{
            QueueState, RecoveryFact, Session, SessionRestore,
        };
        use crate::domain::agent_session::entities::Turn;
        use crate::domain::agent_session::value_objects::{SessionState, TurnPhase};

        let (state, current_turn) = match self.lifecycle {
            SessionLifecycleState::Open { active_turn_id, .. } => {
                let current_turn = active_turn_id
                    .map(|turn_id| Turn::restore(turn_id, TurnPhase::Streaming, None));
                (
                    if current_turn.is_some() {
                        SessionState::Active
                    } else {
                        SessionState::Idle
                    },
                    current_turn,
                )
            }
            SessionLifecycleState::Closed => (SessionState::Closed, None),
            SessionLifecycleState::Archived => (SessionState::Archived, None),
        };
        Session::restore(SessionRestore {
            id: session_id.to_string(),
            revision: u64::try_from(self.session_revision).map_err(|_| {
                crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError::Corrupt(
                    "negative session revision".into(),
                )
            })?,
            state,
            has_messages: false,
            has_provider_session: false,
            current_turn,
            last_terminal: None,
            queue: QueueState::restore(Vec::new(), self.queue_paused),
            recovery_fact: if self.has_pending_recovery
                || self.has_pending_permission
                || self.has_pending_provider_operation
            {
                RecoveryFact::Unresolved
            } else {
                RecoveryFact::Resolved
            },
        })
        .map_err(|error| {
            crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError::Corrupt(
                format!("{error:?}"),
            )
        })
    }
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

/// Post-commit runtime effects for session lifecycle commands.
#[async_trait::async_trait]
pub trait SessionLifecycleEffectPort: Send + Sync {
    /// Whether a live runtime requires a post-commit close/archive effect.
    /// Stored provider identity alone does not count as a live runtime.
    async fn has_live_runtime(&self, session_id: &str) -> Result<bool, SafeOperationFailure>;

    /// Resolve provider configuration for a backend switch without mutating
    /// either the provider or durable session state.
    async fn resolve_backend_selection(
        &self,
        backend_id: &str,
    ) -> Result<crate::domain::agent_session::repository::BackendSelection, SafeOperationFailure>
    {
        Ok(crate::domain::agent_session::repository::BackendSelection {
            backend_id: backend_id.to_string(),
            model_id: backend_id.to_string(),
        })
    }

    /// Legacy behavior-test probe. Production projection preparation is
    /// owned by `AgentSessionLifecycleRepository`.
    #[cfg(test)]
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

/// Legacy Stop fixture shape retained for behavior tests. Production
/// admission restores the Session aggregate.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTargetSnapshot {
    pub session_revision: u64,
    pub active_turn_id: String,
    pub queue_paused: bool,
}

#[cfg(test)]
impl StopTargetSnapshot {
    pub fn restore_session(
        &self,
        session_id: &str,
    ) -> Result<
        crate::domain::agent_session::aggregates::session::Session,
        crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError,
    > {
        use crate::domain::agent_session::aggregates::session::{
            QueueState, RecoveryFact, Session, SessionRestore,
        };
        use crate::domain::agent_session::entities::Turn;
        use crate::domain::agent_session::value_objects::SessionState;

        let turn_id = self.active_turn_id.parse::<u64>().map_err(|_| {
            crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError::Corrupt(
                "invalid active turn identity".into(),
            )
        })?;
        Session::restore(SessionRestore {
            id: session_id.to_string(),
            revision: self.session_revision,
            state: SessionState::Active,
            has_messages: true,
            has_provider_session: true,
            current_turn: Some(Turn::start(turn_id)),
            last_terminal: None,
            queue: QueueState::restore(Vec::new(), self.queue_paused),
            recovery_fact: RecoveryFact::Resolved,
        })
        .map_err(|error| {
            crate::domain::agent_session::repository::AgentSessionLifecycleRepositoryError::Corrupt(
                format!("{error:?}"),
            )
        })
    }
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
pub trait StopEffectPort: Send + Sync {
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
