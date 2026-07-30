//! Semantic validation for caller-supplied operation identities.

use std::fmt;

use crate::domain::agent_session::entities::{InterruptReason, TurnResult};
use crate::domain::agent_session::events::TurnTokenUsage;
use crate::domain::local_event::{
    AgentTerminalKind, ObligationRecord, ObligationStateRecord, ObligationView, PendingPartition,
    Revision, TerminalRecordMutation, TerminalRecordView, TerminalResultRecord,
    WorkflowObligationTerminalOutcome, WorkflowTurnCompletionObligationRecord,
    WorkflowTurnFailureSignalRecord,
};
use crate::domain::workflow::WorkflowNodeContext;

pub const MAX_OPERATION_IDENTITY_BYTES: usize = 128;

const SHA256_INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Standard-library-only SHA-256 used by domain identity contracts.
///
/// The digest algorithm is part of the durable identity vocabulary. Keeping
/// this primitive here lets domain decisions own that vocabulary without
/// depending on a crypto framework.
#[derive(Debug, Clone)]
pub struct Sha256State {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    byte_len: u64,
}

impl Default for Sha256State {
    fn default() -> Self {
        Self {
            state: SHA256_INITIAL_STATE,
            buffer: [0; 64],
            buffer_len: 0,
            byte_len: 0,
        }
    }
}

impl Sha256State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, input: impl AsRef<[u8]>) {
        let mut input = input.as_ref();
        self.byte_len = self.byte_len.wrapping_add(input.len() as u64);

        if self.buffer_len != 0 {
            let available = 64 - self.buffer_len;
            let copied = available.min(input.len());
            self.buffer[self.buffer_len..self.buffer_len + copied]
                .copy_from_slice(&input[..copied]);
            self.buffer_len += copied;
            input = &input[copied..];
            if self.buffer_len == 64 {
                compress_sha256_block(&mut self.state, &self.buffer);
                self.buffer_len = 0;
            } else {
                return;
            }
        }

        while input.len() >= 64 {
            let (block, remainder) = input.split_at(64);
            let block: &[u8; 64] = block.try_into().expect("SHA-256 block has fixed length");
            compress_sha256_block(&mut self.state, block);
            input = remainder;
        }

        self.buffer[..input.len()].copy_from_slice(input);
        self.buffer_len = input.len();
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.byte_len.wrapping_mul(8);
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..].fill(0);
            compress_sha256_block(&mut self.state, &self.buffer);
            self.buffer = [0; 64];
            self.buffer_len = 0;
        }

        self.buffer[self.buffer_len..56].fill(0);
        self.buffer[56..].copy_from_slice(&bit_len.to_be_bytes());
        compress_sha256_block(&mut self.state, &self.buffer);

        let mut digest = [0; 32];
        for (chunk, word) in digest.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

pub fn sha256(input: impl AsRef<[u8]>) -> [u8; 32] {
    let mut hasher = Sha256State::new();
    hasher.update(input);
    hasher.finalize()
}

pub fn hex_lower(input: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let input = input.as_ref();
    let mut encoded = String::with_capacity(input.len().saturating_mul(2));
    for byte in input {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

/// Incremental builder for durable operation identities.
///
/// The distinction between `update` and `field` is part of the persisted
/// identity contract: `field` length-prefixes one semantic value, while
/// `update` appends fixed-width or otherwise self-delimiting material.
#[derive(Debug, Clone, Default)]
pub struct DurableIdentityBuilder {
    hasher: Sha256State,
}

#[derive(Debug, Clone, Copy)]
pub enum RecoveryPublicationMessageIdentityFacts<'a> {
    Notice {
        recovery_id: &'a str,
        message_id: &'a str,
    },
    Error {
        recovery_id: &'a str,
        message_id: &'a str,
        error: &'a str,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum EventProjectionMetaIdentityFacts<'a> {
    RecoveryStarted {
        expected_generation: u64,
        publication_snapshot: &'a [u8],
        at_bits: u64,
    },
    RecoveryCompleted {
        expected_generation: u64,
        provider_session_generation: u64,
        backend_session_id: &'a str,
        pending_message: RecoveryPublicationMessageIdentityFacts<'a>,
        at_bits: u64,
    },
    RecoveryReadbackCompleted {
        old_provider_session_generation: u64,
        provider_session_generation: u64,
        backend_session_id: &'a str,
        pending_message: RecoveryPublicationMessageIdentityFacts<'a>,
        at_bits: u64,
    },
    RecoveryFailed {
        pending_message: RecoveryPublicationMessageIdentityFacts<'a>,
        at_bits: u64,
    },
    ContextRestoreCompleted {
        expected_provider_session_generation: u64,
        expected_turn_id: Option<u64>,
        reinjected: bool,
        clear_context_carry: bool,
        recovery_restore_required: bool,
        at_bits: u64,
    },
}

pub fn hash_terminal_message_projection_patch(
    identity: &mut DurableIdentityBuilder,
    message_id: &str,
    streaming_final_sequence: u64,
    timestamp_bits: Option<u64>,
    encoded_parts: Option<&[u8]>,
) {
    identity.field(b"terminal_message_patch_v1");
    identity.field(message_id.as_bytes());
    identity.update(streaming_final_sequence.to_be_bytes());
    match timestamp_bits {
        Some(timestamp_bits) => {
            identity.update([1]);
            identity.update(timestamp_bits.to_be_bytes());
        }
        None => identity.update([0]),
    }
    match encoded_parts {
        Some(encoded_parts) => {
            identity.update([1]);
            identity.field(encoded_parts);
        }
        None => identity.update([0]),
    }
}

pub fn hash_event_projection_meta_patch(
    identity: &mut DurableIdentityBuilder,
    facts: EventProjectionMetaIdentityFacts<'_>,
) {
    identity.field(b"event_projection_meta_patch_v1");
    match facts {
        EventProjectionMetaIdentityFacts::RecoveryStarted {
            expected_generation,
            publication_snapshot,
            at_bits,
        } => {
            identity.field(b"recovery_started");
            identity.update(expected_generation.to_be_bytes());
            identity.field(publication_snapshot);
            identity.update(at_bits.to_be_bytes());
        }
        EventProjectionMetaIdentityFacts::RecoveryCompleted {
            expected_generation,
            provider_session_generation,
            backend_session_id,
            pending_message,
            at_bits,
        } => {
            identity.field(b"recovery_completed");
            identity.update(expected_generation.to_be_bytes());
            identity.update(provider_session_generation.to_be_bytes());
            identity.field(backend_session_id.as_bytes());
            hash_recovery_publication_message(identity, pending_message);
            identity.update(at_bits.to_be_bytes());
        }
        EventProjectionMetaIdentityFacts::RecoveryReadbackCompleted {
            old_provider_session_generation,
            provider_session_generation,
            backend_session_id,
            pending_message,
            at_bits,
        } => {
            identity.field(b"recovery_readback_completed");
            identity.update(old_provider_session_generation.to_be_bytes());
            identity.update(provider_session_generation.to_be_bytes());
            identity.field(backend_session_id.as_bytes());
            hash_recovery_publication_message(identity, pending_message);
            identity.update(at_bits.to_be_bytes());
        }
        EventProjectionMetaIdentityFacts::RecoveryFailed {
            pending_message,
            at_bits,
        } => {
            identity.field(b"recovery_failed");
            hash_recovery_publication_message(identity, pending_message);
            identity.update(at_bits.to_be_bytes());
        }
        EventProjectionMetaIdentityFacts::ContextRestoreCompleted {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
            at_bits,
        } => {
            identity.field(b"context_restore_completed");
            identity.update(expected_provider_session_generation.to_be_bytes());
            match expected_turn_id {
                Some(turn_id) => {
                    identity.update([1]);
                    identity.update(turn_id.to_be_bytes());
                }
                None => identity.update([0]),
            }
            identity.update([
                u8::from(reinjected),
                u8::from(clear_context_carry),
                u8::from(recovery_restore_required),
            ]);
            identity.update(at_bits.to_be_bytes());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableCommitIdentity {
    pub digest: [u8; 32],
    pub identity: String,
}

pub fn workflow_turn_completion_consume_commit_identity(
    obligation_id: &str,
    current_revision: i64,
    canonical_mutation_identity: &[u8],
) -> DurableCommitIdentity {
    let digest = sha256(canonical_mutation_identity);
    let mut commit = DurableIdentityBuilder::new();
    commit.field(b"workflow_turn_completion_consume_commit_v1");
    commit.field(obligation_id.as_bytes());
    commit.update(current_revision.to_be_bytes());
    commit.update(digest);
    DurableCommitIdentity {
        digest,
        identity: commit.finalize_hex(),
    }
}

pub fn agent_event_payload_identity<'a>(
    session_id: &str,
    encoded_events: &[u8],
    mutation_identities: impl IntoIterator<Item = &'a [u8]>,
) -> [u8; 32] {
    let mut identity = DurableIdentityBuilder::new();
    identity.field(b"agent_event_commit_identity_v1");
    identity.field(session_id.as_bytes());
    identity.field(encoded_events);
    for mutation in mutation_identities {
        identity.field(mutation);
    }
    identity.finalize()
}

pub fn agent_atomic_event_payload_identity<'a, E>(
    session_id: &str,
    operation_kind: &str,
    encoded_events: &[u8],
    mutation_identities: impl IntoIterator<Item = &'a [u8]>,
    append_projection_facts: impl FnOnce(&mut DurableIdentityBuilder) -> Result<(), E>,
) -> Result<DurableCommitIdentity, E> {
    let mut identity = DurableIdentityBuilder::new();
    identity.field(b"agent_atomic_event_commit_identity_v1");
    identity.field(session_id.as_bytes());
    identity.field(operation_kind.as_bytes());
    identity.field(encoded_events);
    for mutation in mutation_identities {
        identity.field(mutation);
    }
    append_projection_facts(&mut identity)?;
    let digest = identity.finalize();
    Ok(DurableCommitIdentity {
        identity: format!("session-atomic-event-{}", hex_lower(digest)),
        digest,
    })
}

pub fn backend_recovery_readback_participant_identity<'a>(
    stream_id: &str,
    stream_head: i64,
    encoded_events: &[u8],
    sorted_mutation_identities: impl IntoIterator<Item = &'a [u8]>,
) -> [u8; 32] {
    let mut identity = DurableIdentityBuilder::new();
    identity.field(b"backend_recovery_readback_participants_v1");
    identity.field(stream_id.as_bytes());
    identity.update(stream_head.to_be_bytes());
    identity.field(encoded_events);
    for mutation in sorted_mutation_identities {
        identity.field(mutation);
    }
    identity.finalize()
}

pub fn session_projection_binding_identity<'a>(
    encoded_projection_identity: &[u8],
    revision: i64,
    messages: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    mutation_identities: impl IntoIterator<Item = &'a [u8]>,
) -> DurableCommitIdentity {
    let mut identity = DurableIdentityBuilder::new();
    identity.update(sha256(encoded_projection_identity));
    identity.update(revision.to_be_bytes());
    for (message_id, encoded_message) in messages {
        identity.update((message_id.len() as u64).to_be_bytes());
        identity.update(message_id.as_bytes());
        identity.update((encoded_message.len() as u64).to_be_bytes());
        identity.update(encoded_message);
    }
    for mutation in mutation_identities {
        identity.field(mutation);
    }
    let digest = identity.finalize();
    DurableCommitIdentity {
        identity: format!("session-projection-{}", hex_lower(digest)),
        digest,
    }
}

pub fn session_projection_rollback_identity(
    session_id: &str,
    revision: i64,
) -> DurableCommitIdentity {
    let mut identity = DurableIdentityBuilder::new();
    identity.update(b"agent-session-projection-rollback/v1");
    identity.update((session_id.len() as u64).to_be_bytes());
    identity.update(session_id.as_bytes());
    identity.update(revision.to_be_bytes());
    let digest = identity.finalize();
    DurableCommitIdentity {
        identity: format!("session-projection-rollback-{}", hex_lower(digest)),
        digest,
    }
}

fn hash_recovery_publication_message(
    identity: &mut DurableIdentityBuilder,
    facts: RecoveryPublicationMessageIdentityFacts<'_>,
) {
    match facts {
        RecoveryPublicationMessageIdentityFacts::Notice {
            recovery_id,
            message_id,
        } => {
            identity.field(b"notice");
            identity.field(recovery_id.as_bytes());
            identity.field(message_id.as_bytes());
        }
        RecoveryPublicationMessageIdentityFacts::Error {
            recovery_id,
            message_id,
            error,
        } => {
            identity.field(b"error");
            identity.field(recovery_id.as_bytes());
            identity.field(message_id.as_bytes());
            identity.field(error.as_bytes());
        }
    }
}

impl DurableIdentityBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, input: impl AsRef<[u8]>) {
        self.hasher.update(input);
    }

    pub fn field(&mut self, value: impl AsRef<[u8]>) {
        let value = value.as_ref();
        self.hasher.update((value.len() as u64).to_be_bytes());
        self.hasher.update(value);
    }

    pub fn finalize(self) -> [u8; 32] {
        self.hasher.finalize()
    }

    pub fn finalize_hex(self) -> String {
        hex_lower(self.finalize())
    }
}

#[cfg(test)]
fn parse_sha256_hex(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        decoded[index] = (high << 4) | low;
    }
    Some(decoded)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTerminalIdentity {
    pub terminal_kind: AgentTerminalKind,
    pub terminal_identity: String,
    pub participant_digest: [u8; 32],
}

pub fn runtime_terminal_identity(
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    streaming_final_sequence: u64,
    completed_at_bits: u64,
    encoded_events: &[u8],
    result: &TurnResult,
) -> RuntimeTerminalIdentity {
    let terminal_kind = match result {
        TurnResult::Completed { .. } => AgentTerminalKind::Completed,
        TurnResult::Failed { .. } => AgentTerminalKind::Crash,
        TurnResult::Interrupted { reason, .. } => match reason {
            InterruptReason::Abort => AgentTerminalKind::Abort,
            InterruptReason::Timeout => AgentTerminalKind::Timeout,
            InterruptReason::Crash => AgentTerminalKind::Crash,
            InterruptReason::SessionClosed => AgentTerminalKind::SessionClosed,
        },
    };
    let mut identity = DurableIdentityBuilder::new();
    identity.update(session_id.as_bytes());
    identity.update(turn_id.to_be_bytes());
    identity.update(message_id.as_bytes());
    identity.update(streaming_final_sequence.to_be_bytes());
    identity.update(completed_at_bits.to_be_bytes());
    identity.field(encoded_events);
    let participant_digest = identity.finalize();
    RuntimeTerminalIdentity {
        terminal_kind,
        terminal_identity: format!("runtime-terminal-{}", hex_lower(participant_digest)),
        participant_digest,
    }
}

pub fn session_closed_terminal_identity_material(
    operation_id: &str,
    session_id: &str,
    turn_id: u64,
) -> Vec<u8> {
    format!("session-closed-terminal-semantic/v1\0{operation_id}\0{session_id}\0{turn_id}")
        .into_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnCompletionIdentity {
    pub notification_digest: [u8; 32],
    pub notification_sha256: String,
    pub obligation_id: String,
    pub ordered_key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkflowTurnCompletionIdentityFacts<'a> {
    pub session_id: &'a str,
    pub workflow_context: &'a WorkflowNodeContext,
    pub terminal_identity: &'a str,
    pub message_id: &'a str,
    pub turn_id: u64,
    pub exit_code: i64,
    pub final_text_parts: &'a [String],
    pub failure_signal: Option<WorkflowTurnFailureSignalRecord>,
    pub token_usage: Option<TurnTokenUsage>,
    pub interrupted: bool,
}

pub fn workflow_turn_completion_identity(
    facts: WorkflowTurnCompletionIdentityFacts<'_>,
) -> WorkflowTurnCompletionIdentity {
    let mut identity = DurableIdentityBuilder::new();
    identity.field(b"workflow_turn_completion_notification_identity_v1");
    identity.field(facts.session_id.as_bytes());
    identity.field(facts.workflow_context.execution_id.as_bytes());
    identity.field(facts.workflow_context.node_execution_id.as_bytes());
    identity.field(facts.workflow_context.workflow_name.as_bytes());
    identity.field(facts.workflow_context.node_name.as_bytes());
    identity.update(facts.workflow_context.attempt.to_be_bytes());
    identity_optional_text(
        &mut identity,
        facts.workflow_context.parent_node_name.as_deref(),
    );
    identity_optional_u32(&mut identity, facts.workflow_context.parent_attempt);
    identity.update(facts.workflow_context.order.to_be_bytes());
    identity_optional_u64(&mut identity, facts.workflow_context.startup_timeout_secs);
    identity_optional_u32(&mut identity, facts.workflow_context.startup_max_retries);
    identity_optional_u64(&mut identity, facts.workflow_context.stale_timeout_secs);
    identity.field(facts.terminal_identity.as_bytes());
    identity.field(facts.message_id.as_bytes());
    identity.update(facts.turn_id.to_be_bytes());
    identity.update(facts.exit_code.to_be_bytes());
    identity.update((facts.final_text_parts.len() as u64).to_be_bytes());
    for part in facts.final_text_parts {
        identity.field(part.as_bytes());
    }
    match facts.failure_signal {
        Some(WorkflowTurnFailureSignalRecord::ModelRefusal) => {
            identity.update([1]);
            identity.field(b"model_refusal");
        }
        None => identity.update([0]),
    }
    match facts.token_usage {
        Some(usage) => {
            identity.update([1]);
            identity.update(usage.input_tokens.to_be_bytes());
            identity.update(usage.output_tokens.to_be_bytes());
        }
        None => identity.update([0]),
    }
    identity.update([u8::from(facts.interrupted)]);
    let notification_digest = identity.finalize();
    let notification_sha256 = hex_lower(notification_digest);
    WorkflowTurnCompletionIdentity {
        obligation_id: workflow_turn_completion_obligation_id(&notification_sha256),
        ordered_key: workflow_turn_completion_ordered_key(facts.turn_id, &notification_sha256),
        notification_digest,
        notification_sha256,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnCompletionBindingRejection {
    TerminalIdentityMismatch,
}

pub fn decide_workflow_turn_completion_identity(
    terminal: &TerminalRecordMutation,
    facts: WorkflowTurnCompletionIdentityFacts<'_>,
) -> Result<WorkflowTurnCompletionIdentity, WorkflowTurnCompletionBindingRejection> {
    if terminal.session_id != facts.session_id
        || terminal.turn_id.parse::<u64>().ok() != Some(facts.turn_id)
    {
        return Err(WorkflowTurnCompletionBindingRejection::TerminalIdentityMismatch);
    }
    Ok(workflow_turn_completion_identity(facts))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPendingWorkflowTurnCompletion {
    pub session_id: String,
    pub turn_id: u64,
    pub terminal_identity: String,
    pub notification_digest: [u8; 32],
    pub workflow_context: WorkflowNodeContext,
    pub message_id: String,
    pub exit_code: i64,
    pub failure_signal: Option<WorkflowTurnFailureSignalRecord>,
    pub token_usage: Option<TurnTokenUsage>,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWorkflowTurnCompletionRejection {
    IncompatibleRecord,
    InvalidTurnIdentity,
    IndexIdentityMismatch,
    TerminalIdentityMismatch,
    MessageIdentityMismatch,
    NotificationBindingMismatch,
}

pub fn validate_pending_workflow_turn_completion(
    obligation_id: &str,
    owner: &str,
    ordered_key: &str,
    partition: PendingPartition,
    has_shutdown_plan: bool,
    record: &ObligationRecord,
) -> Result<ValidatedPendingWorkflowTurnCompletion, PendingWorkflowTurnCompletionRejection> {
    let ObligationRecord::WorkflowTurnCompletion {
        session_id,
        turn_id,
        terminal_identity,
        notification_sha256,
        detail:
            WorkflowTurnCompletionObligationRecord::Pending {
                workflow_context,
                message_id,
                exit_code,
                failure_signal,
                token_usage,
                interrupted,
            },
        state: ObligationStateRecord::Pending,
    } = record
    else {
        return Err(PendingWorkflowTurnCompletionRejection::IncompatibleRecord);
    };
    let turn_id = turn_id
        .parse::<u64>()
        .map_err(|_| PendingWorkflowTurnCompletionRejection::InvalidTurnIdentity)?;
    let notification_digest = *notification_sha256;
    let notification_sha256 = hex_lower(notification_digest);
    if owner != session_id
        || partition != PendingPartition::Owner
        || has_shutdown_plan
        || obligation_id != workflow_turn_completion_obligation_id(&notification_sha256)
        || ordered_key != workflow_turn_completion_ordered_key(turn_id, &notification_sha256)
    {
        return Err(PendingWorkflowTurnCompletionRejection::IndexIdentityMismatch);
    }
    Ok(ValidatedPendingWorkflowTurnCompletion {
        session_id: session_id.clone(),
        turn_id,
        terminal_identity: terminal_identity.clone(),
        notification_digest,
        workflow_context: workflow_context.as_ref().clone(),
        message_id: message_id.clone(),
        exit_code: *exit_code,
        failure_signal: *failure_signal,
        token_usage: *token_usage,
        interrupted: *interrupted,
    })
}

pub fn validate_workflow_turn_completion_terminal(
    terminal: &TerminalRecordView,
    pending: &ValidatedPendingWorkflowTurnCompletion,
) -> Result<(), PendingWorkflowTurnCompletionRejection> {
    if terminal.session_id != pending.session_id
        || terminal.turn_id.parse::<u64>().ok() != Some(pending.turn_id)
        || terminal.terminal_identity != pending.terminal_identity
    {
        return Err(PendingWorkflowTurnCompletionRejection::TerminalIdentityMismatch);
    }
    if !matches!(
        &terminal.result,
        TerminalResultRecord::AgentTurn {
            session_id,
            turn_id,
            message_id,
            ..
        } if session_id == &pending.session_id
            && turn_id.parse::<u64>().ok() == Some(pending.turn_id)
            && message_id == &pending.message_id
    ) {
        return Err(PendingWorkflowTurnCompletionRejection::MessageIdentityMismatch);
    }
    Ok(())
}

pub fn validate_workflow_turn_completion_notification(
    pending: &ValidatedPendingWorkflowTurnCompletion,
    final_text_parts: &[String],
) -> Result<WorkflowTurnCompletionIdentity, PendingWorkflowTurnCompletionRejection> {
    let identity = workflow_turn_completion_identity(WorkflowTurnCompletionIdentityFacts {
        session_id: &pending.session_id,
        workflow_context: &pending.workflow_context,
        terminal_identity: &pending.terminal_identity,
        message_id: &pending.message_id,
        turn_id: pending.turn_id,
        exit_code: pending.exit_code,
        final_text_parts,
        failure_signal: pending.failure_signal,
        token_usage: pending.token_usage,
        interrupted: pending.interrupted,
    });
    if identity.notification_digest != pending.notification_digest {
        return Err(PendingWorkflowTurnCompletionRejection::NotificationBindingMismatch);
    }
    Ok(identity)
}

#[derive(Debug, Clone, Copy)]
pub struct WorkflowTurnCompletionSettlementFacts<'a> {
    pub obligation_id: &'a str,
    pub revision: Revision,
    pub session_id: &'a str,
    pub workflow_context: &'a WorkflowNodeContext,
    pub terminal_identity: &'a str,
    pub message_id: &'a str,
    pub turn_id: u64,
    pub exit_code: i64,
    pub final_text_parts: &'a [String],
    pub failure_signal: Option<WorkflowTurnFailureSignalRecord>,
    pub token_usage: Option<TurnTokenUsage>,
    pub interrupted: bool,
    pub notification_sha256: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowTurnCompletionSettlementDecision {
    AlreadySettled,
    Apply {
        notification_digest: [u8; 32],
        detail: WorkflowTurnCompletionObligationRecord,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnCompletionSettlementRejection {
    ConsumeBindingMismatch,
    CompletedObligationMismatch,
    PendingObligationMismatch,
    IncompatibleObligation,
    AlreadyTerminal,
}

pub fn decide_workflow_turn_completion_settlement(
    facts: WorkflowTurnCompletionSettlementFacts<'_>,
    current: &ObligationView,
    outcome: WorkflowObligationTerminalOutcome,
    settled_at_bits: u64,
) -> Result<WorkflowTurnCompletionSettlementDecision, WorkflowTurnCompletionSettlementRejection> {
    let identity = workflow_turn_completion_identity(WorkflowTurnCompletionIdentityFacts {
        session_id: facts.session_id,
        workflow_context: facts.workflow_context,
        terminal_identity: facts.terminal_identity,
        message_id: facts.message_id,
        turn_id: facts.turn_id,
        exit_code: facts.exit_code,
        final_text_parts: facts.final_text_parts,
        failure_signal: facts.failure_signal,
        token_usage: facts.token_usage,
        interrupted: facts.interrupted,
    });
    if identity.notification_sha256 != facts.notification_sha256
        || identity.obligation_id != facts.obligation_id
    {
        return Err(WorkflowTurnCompletionSettlementRejection::ConsumeBindingMismatch);
    }

    let pending_detail = match &current.record {
        ObligationRecord::WorkflowTurnCompletion {
            session_id,
            turn_id,
            terminal_identity,
            notification_sha256,
            detail,
            state: ObligationStateRecord::Completed,
        } if detail.terminal_outcome().is_some() => {
            if session_id == facts.session_id
                && turn_id.parse::<u64>().ok() == Some(facts.turn_id)
                && terminal_identity == facts.terminal_identity
                && notification_sha256 == &identity.notification_digest
                && current.pending.is_none()
            {
                return Ok(WorkflowTurnCompletionSettlementDecision::AlreadySettled);
            }
            return Err(WorkflowTurnCompletionSettlementRejection::CompletedObligationMismatch);
        }
        ObligationRecord::WorkflowTurnCompletion {
            session_id,
            turn_id,
            terminal_identity,
            notification_sha256,
            detail:
                detail @ WorkflowTurnCompletionObligationRecord::Pending {
                    workflow_context,
                    message_id,
                    exit_code,
                    failure_signal,
                    token_usage,
                    interrupted,
                },
            state: ObligationStateRecord::Pending,
        } => {
            if session_id != facts.session_id
                || workflow_context.as_ref() != facts.workflow_context
                || turn_id.parse::<u64>().ok() != Some(facts.turn_id)
                || terminal_identity != facts.terminal_identity
                || message_id != facts.message_id
                || *exit_code != facts.exit_code
                || *failure_signal != facts.failure_signal
                || *token_usage != facts.token_usage
                || *interrupted != facts.interrupted
                || notification_sha256 != &identity.notification_digest
                || current.pending.is_none()
                || current.revision != facts.revision
            {
                return Err(WorkflowTurnCompletionSettlementRejection::PendingObligationMismatch);
            }
            detail
        }
        _ => {
            return Err(WorkflowTurnCompletionSettlementRejection::IncompatibleObligation);
        }
    };
    let detail = pending_detail
        .settle(outcome, settled_at_bits)
        .map_err(|_| WorkflowTurnCompletionSettlementRejection::AlreadyTerminal)?;
    Ok(WorkflowTurnCompletionSettlementDecision::Apply {
        notification_digest: identity.notification_digest,
        detail,
    })
}

pub fn workflow_turn_completion_obligation_id(notification_sha256: &str) -> String {
    format!("workflow-turn-complete:{notification_sha256}")
}

pub fn workflow_turn_completion_ordered_key(turn_id: u64, notification_sha256: &str) -> String {
    format!("workflow_turn_complete:{turn_id:020}:{notification_sha256}")
}

pub fn workflow_turn_completion_ordered_key_prefix(turn_id: Option<u64>) -> String {
    match turn_id {
        Some(turn_id) => format!("workflow_turn_complete:{turn_id:020}:"),
        None => "workflow_turn_complete:".to_string(),
    }
}

fn identity_optional_text(identity: &mut DurableIdentityBuilder, value: Option<&str>) {
    match value {
        Some(value) => {
            identity.update([1]);
            identity.field(value.as_bytes());
        }
        None => identity.update([0]),
    }
}

fn identity_optional_u32(identity: &mut DurableIdentityBuilder, value: Option<u32>) {
    match value {
        Some(value) => {
            identity.update([1]);
            identity.update(value.to_be_bytes());
        }
        None => identity.update([0]),
    }
}

fn identity_optional_u64(identity: &mut DurableIdentityBuilder, value: Option<u64>) {
    match value {
        Some(value) => {
            identity.update([1]);
            identity.update(value.to_be_bytes());
        }
        None => identity.update([0]),
    }
}

#[cfg(test)]
fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn compress_sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 64];
    for (index, chunk) in block.chunks_exact(4).enumerate() {
        schedule[index] = u32::from_be_bytes(chunk.try_into().expect("word has fixed length"));
    }
    for index in 16..64 {
        let s0 = schedule[index - 15].rotate_right(7)
            ^ schedule[index - 15].rotate_right(18)
            ^ (schedule[index - 15] >> 3);
        let s1 = schedule[index - 2].rotate_right(17)
            ^ schedule[index - 2].rotate_right(19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(sum1)
            .wrapping_add(choice)
            .wrapping_add(SHA256_ROUND_CONSTANTS[index])
            .wrapping_add(schedule[index]);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = sum0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (current, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *current = current.wrapping_add(value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationIdentityError {
    Empty,
    TooLong { max: usize },
    InvalidCharacter,
}

impl fmt::Display for OperationIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "operation identity must not be empty"),
            Self::TooLong { max } => write!(f, "operation identity exceeds {max} bytes"),
            Self::InvalidCharacter => {
                write!(
                    f,
                    "operation identity contains a character outside [A-Za-z0-9._:-]"
                )
            }
        }
    }
}

impl std::error::Error for OperationIdentityError {}

pub fn validate_operation_identity(raw: &str) -> Result<(), OperationIdentityError> {
    if raw.is_empty() {
        return Err(OperationIdentityError::Empty);
    }
    if raw.len() > MAX_OPERATION_IDENTITY_BYTES {
        return Err(OperationIdentityError::TooLong {
            max: MAX_OPERATION_IDENTITY_BYTES,
        });
    }
    if raw
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')))
    {
        return Err(OperationIdentityError::InvalidCharacter);
    }
    Ok(())
}

pub fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut diff: u8 = 0;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_identity_accepts_only_the_closed_vocabulary() {
        assert!(validate_operation_identity("send.1_owner:retry-2").is_ok());
        assert_eq!(
            validate_operation_identity(""),
            Err(OperationIdentityError::Empty)
        );
        assert_eq!(
            validate_operation_identity("contains space"),
            Err(OperationIdentityError::InvalidCharacter)
        );
        assert_eq!(
            validate_operation_identity(&"a".repeat(MAX_OPERATION_IDENTITY_BYTES + 1)),
            Err(OperationIdentityError::TooLong {
                max: MAX_OPERATION_IDENTITY_BYTES
            })
        );
    }

    #[test]
    fn sha256_matches_the_standard_known_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
        assert_eq!(hex_lower([0x00, 0xab, 0xff]), "00abff");
        assert_eq!(
            parse_sha256_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            Some(sha256(b"abc"))
        );
        assert_eq!(parse_sha256_hex("xyz"), None);
    }

    #[test]
    fn durable_identity_fields_are_length_delimited() {
        let mut identity = DurableIdentityBuilder::new();
        identity.field(b"ab");
        identity.field(b"c");
        let separated = identity.finalize();

        let mut different = DurableIdentityBuilder::new();
        different.field(b"a");
        different.field(b"bc");
        assert_ne!(separated, different.finalize());
    }

    #[test]
    fn runtime_terminal_classification_and_identity_are_domain_owned() {
        let completed = runtime_terminal_identity(
            "session",
            7,
            "message",
            9,
            11_f64.to_bits(),
            b"events",
            &TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
        );
        assert_eq!(completed.terminal_kind, AgentTerminalKind::Completed);
        assert!(completed.terminal_identity.starts_with("runtime-terminal-"));

        let interrupted = runtime_terminal_identity(
            "session",
            7,
            "message",
            9,
            11_f64.to_bits(),
            b"events",
            &TurnResult::Interrupted {
                reason: InterruptReason::Timeout,
                error: None,
            },
        );
        assert_eq!(interrupted.terminal_kind, AgentTerminalKind::Timeout);
        assert_eq!(completed.participant_digest, interrupted.participant_digest);
    }

    #[test]
    fn workflow_completion_identity_binds_domain_facts() {
        let context = WorkflowNodeContext {
            execution_id: "execution".into(),
            node_execution_id: "node-execution".into(),
            workflow_name: "workflow".into(),
            node_name: "node".into(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 2,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };
        let facts = WorkflowTurnCompletionIdentityFacts {
            session_id: "session",
            workflow_context: &context,
            terminal_identity: "terminal",
            message_id: "message",
            turn_id: 3,
            exit_code: 0,
            final_text_parts: &["done".into()],
            failure_signal: None,
            token_usage: None,
            interrupted: false,
        };
        let first = workflow_turn_completion_identity(facts);
        let changed = workflow_turn_completion_identity(WorkflowTurnCompletionIdentityFacts {
            message_id: "other",
            ..facts
        });
        assert_ne!(first.notification_digest, changed.notification_digest);
        assert_eq!(
            first.obligation_id,
            format!("workflow-turn-complete:{}", first.notification_sha256)
        );
        assert_eq!(
            first.ordered_key,
            format!(
                "workflow_turn_complete:{:020}:{}",
                facts.turn_id, first.notification_sha256
            )
        );
    }

    #[test]
    fn workflow_completion_pending_participants_are_validated_in_domain() {
        let context = WorkflowNodeContext {
            execution_id: "execution".into(),
            node_execution_id: "node-execution".into(),
            workflow_name: "workflow".into(),
            node_name: "node".into(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 2,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };
        let final_text_parts = vec!["done".to_string()];
        let identity = workflow_turn_completion_identity(WorkflowTurnCompletionIdentityFacts {
            session_id: "session",
            workflow_context: &context,
            terminal_identity: "terminal",
            message_id: "message",
            turn_id: 3,
            exit_code: 0,
            final_text_parts: &final_text_parts,
            failure_signal: None,
            token_usage: None,
            interrupted: false,
        });
        let record = ObligationRecord::WorkflowTurnCompletion {
            session_id: "session".into(),
            turn_id: "3".into(),
            terminal_identity: "terminal".into(),
            notification_sha256: identity.notification_digest,
            detail: WorkflowTurnCompletionObligationRecord::Pending {
                workflow_context: Box::new(context),
                message_id: "message".into(),
                exit_code: 0,
                failure_signal: None,
                token_usage: None,
                interrupted: false,
            },
            state: ObligationStateRecord::Pending,
        };
        let pending = validate_pending_workflow_turn_completion(
            &identity.obligation_id,
            "session",
            &identity.ordered_key,
            PendingPartition::Owner,
            false,
            &record,
        )
        .unwrap();
        assert_eq!(
            validate_workflow_turn_completion_notification(&pending, &final_text_parts),
            Ok(identity)
        );
        assert_eq!(
            validate_pending_workflow_turn_completion(
                &pending
                    .notification_digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
                "other",
                "wrong",
                PendingPartition::Owner,
                false,
                &record,
            ),
            Err(PendingWorkflowTurnCompletionRejection::IndexIdentityMismatch)
        );
    }

    #[test]
    fn durable_commit_identities_bind_their_domain_participants() {
        let event = agent_event_payload_identity(
            "session",
            b"events",
            [b"mutation-a".as_slice(), b"mutation-b".as_slice()],
        );
        assert_ne!(
            event,
            agent_event_payload_identity(
                "session",
                b"events",
                [b"mutation-b".as_slice(), b"mutation-a".as_slice()],
            )
        );

        let atomic = agent_atomic_event_payload_identity(
            "session",
            "send",
            b"events",
            [b"mutation".as_slice()],
            |identity| {
                identity.field(b"projection");
                Ok::<(), ()>(())
            },
        )
        .unwrap();
        assert!(atomic.identity.starts_with("session-atomic-event-"));

        let readback = backend_recovery_readback_participant_identity(
            "agent-session:session",
            4,
            b"events",
            [b"mutation-a".as_slice(), b"mutation-b".as_slice()],
        );
        assert_ne!(
            readback,
            backend_recovery_readback_participant_identity(
                "agent-session:session",
                5,
                b"events",
                [b"mutation-a".as_slice(), b"mutation-b".as_slice()],
            )
        );

        let projection = session_projection_binding_identity(
            b"projection",
            7,
            [("message", b"message-projection".as_slice())],
            [b"mutation".as_slice()],
        );
        assert!(projection.identity.starts_with("session-projection-"));
        assert_ne!(
            projection,
            session_projection_binding_identity(
                b"projection",
                8,
                [("message", b"message-projection".as_slice())],
                [b"mutation".as_slice()],
            )
        );

        let rollback = session_projection_rollback_identity("session", 7);
        assert!(rollback
            .identity
            .starts_with("session-projection-rollback-"));
        assert_ne!(rollback, session_projection_rollback_identity("session", 8));

        let consumed =
            workflow_turn_completion_consume_commit_identity("obligation", 3, b"mutation");
        assert_eq!(consumed.digest, sha256(b"mutation"));
    }
}
