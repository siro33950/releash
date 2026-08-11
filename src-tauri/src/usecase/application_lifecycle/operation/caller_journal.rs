//! Built-in Tauri caller attempt journal (owner-private local outbox).
//!
//! The exact command is saved to the Rust-owned `caller_attempts` family
//! before usecase dispatch. If it cannot be saved the command is
//! `RejectedBeforeCommit` with zero effects; if the save result is unknown
//! the same identity resolves as `OutcomeUnknown`. The entry is cleared only
//! after an Accepted receipt or a deterministic pre-commit rejection is
//! confirmed. This is not a public prepared lifecycle: there is no prepared
//! list, no content resolver, and no public read surface.

use std::sync::Arc;

use crate::domain::local_event::{
    CallerAttemptMutation, CallerAttemptResolution, CallerOperationKey, CommitBatchError,
    CommitBatchResult, CommitIdentity, CommitResolution, IdempotencyBinding, LocalAtomicBatch,
    LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    OperationBindingMutation, OperationKind, Revision, RevisionGuard,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::OperationBindingAuthority;
use super::record::hex_encode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerJournalError {
    InvalidRequest,
    /// Shutdown admission closed after the controller preflight and before
    /// the journal transaction acquired the writer lock.
    ShutdownInProgress,
    /// Journal write failed before any commit; the caller command must be
    /// rejected with zero effects.
    RejectedBeforeCommit,
    /// The journal write result is unknown; the caller must resolve the same
    /// identity instead of retrying under a new one.
    OutcomeUnknown,
    /// Same request identity already journaled with a different exact
    /// command.
    PayloadConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerJournalOutcome {
    Recorded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCallerAttempt {
    pub kind: OperationKind,
    pub caller_request_id: String,
    pub operation_id: Option<String>,
    pub resolution: CallerAttemptResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCallerAttemptPage {
    pub entries: Vec<PendingCallerAttempt>,
    pub next_cursor: Option<String>,
}

pub struct CallerAttemptJournal {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    installation_id: String,
}

pub struct BoundCallerOperation<'a> {
    pub operation_id: &'a str,
    pub binding_hmac: [u8; 32],
}

struct AttemptCommit<'a> {
    principal: &'a str,
    kind: OperationKind,
    caller_request_id: &'a str,
    exact_command: &'a [u8],
    scope_id: Option<&'a str>,
    resolution: CallerAttemptResolution,
    expected: RevisionGuard,
    revision: i64,
    step: &'a str,
}

impl CallerAttemptJournal {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            installation_id,
        }
    }

    fn key(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
    ) -> CallerOperationKey {
        CallerOperationKey {
            principal: principal.to_string(),
            installation_id: self.installation_id.clone(),
            kind,
            caller_request_id: caller_request_id.to_string(),
        }
    }

    fn commit_id(
        &self,
        label: &str,
        kind: OperationKind,
        caller_request_id: &str,
        step: &str,
    ) -> CommitIdentity {
        let digest = self.authority.digest(
            format!(
                "caller-attempt\0{label}\0{}\0{caller_request_id}\0{step}",
                kind.label()
            )
            .as_bytes(),
        );
        CommitIdentity::parse(&hex_encode(&digest))
            .expect("hex digest is always a valid commit identity")
    }

    fn seal_context(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
    ) -> Vec<u8> {
        format!(
            "caller-attempt-command/v1\0{principal}\0{}\0{}\0{caller_request_id}",
            self.installation_id,
            kind.label()
        )
        .into_bytes()
    }

    fn command_hash(&self, exact_command: &[u8]) -> [u8; 32] {
        let mut material = b"caller-attempt-command-hash/v1\0".to_vec();
        material.extend_from_slice(exact_command);
        self.authority.mac(&material)
    }

    async fn commit_attempt(&self, attempt: AttemptCommit<'_>) -> Result<(), CallerJournalError> {
        let AttemptCommit {
            principal,
            kind,
            caller_request_id,
            exact_command,
            scope_id,
            resolution,
            expected,
            revision,
            step,
        } = attempt;
        let command_hash = self.command_hash(exact_command);
        let owner_key = self.authority.digest(
            format!(
                "caller-attempt-owner/v1\0{principal}\0{}\0{}\0{caller_request_id}",
                self.installation_id,
                kind.label()
            )
            .as_bytes(),
        );
        let payload_hash = self.authority.digest(
            format!(
                "caller-attempt-state/v1\0{}\0{}\0{}\0{}",
                hex_encode(&command_hash),
                resolution.label(),
                revision,
                step
            )
            .as_bytes(),
        );
        let sealed_command = if resolution == CallerAttemptResolution::Cleared {
            Vec::new()
        } else {
            self.authority
                .seal_command(
                    &self.seal_context(principal, kind, caller_request_id),
                    exact_command,
                )
                .map_err(|_| CallerJournalError::RejectedBeforeCommit)?
        };
        let mutation = CallerAttemptMutation {
            key: self.key(principal, kind, caller_request_id),
            scope_id: scope_id.map(str::to_string),
            command_hash,
            sealed_command,
            resolution,
            expected,
            revision: Revision::new(revision).map_err(|_| CallerJournalError::InvalidRequest)?,
        };
        let batch = LocalAtomicBatch {
            commit_id: self.commit_id(principal, kind, caller_request_id, step),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: if matches!(expected, RevisionGuard::Expected(_)) {
                    crate::domain::local_event::CommitOperationKind::OperationProgress
                } else {
                    kind.into()
                },
                idempotency_key: format!("attempt.{}.{}", hex_encode(&owner_key), step),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::CallerAttempt(mutation)],
        };
        match self.repository.commit_batch(batch).await {
            Ok(_) => Ok(()),
            Err(CommitBatchError::PayloadConflict) => Err(CallerJournalError::PayloadConflict),
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(CallerJournalError::ShutdownInProgress)
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => Err(CallerJournalError::OutcomeUnknown),
            Err(_) => Err(CallerJournalError::RejectedBeforeCommit),
        }
    }

    async fn lookup(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
    ) -> Result<Option<crate::domain::local_event::CallerAttemptView>, CallerJournalError> {
        let result = self
            .repository
            .query(LocalEventQuery::CallerAttemptByIdentity {
                key: self.key(principal, kind, caller_request_id),
            })
            .await
            // A read failure cannot prove that the caller attempt is absent.
            // Treating it as a pre-commit rejection would allow a caller to
            // discard this identity even though an earlier journal write may
            // already have committed.
            .map_err(|_| CallerJournalError::OutcomeUnknown)?;
        let LocalEventQueryResult::CallerAttemptByIdentity(value) = result else {
            return Err(CallerJournalError::OutcomeUnknown);
        };
        Ok(value)
    }

    /// Persist the owner-private exact command and its public deterministic
    /// operation locator in one transaction. Application quit uses this at
    /// the first-writer boundary so both Tauri and WebSocket callers retain
    /// the same durable ambiguity anchor if acceptance readback is lost.
    pub async fn record_bound_attempt_scoped(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
        exact_command: &[u8],
        scope_id: Option<&str>,
        bound: BoundCallerOperation<'_>,
    ) -> Result<CallerJournalOutcome, CallerJournalError> {
        let BoundCallerOperation {
            operation_id,
            binding_hmac,
        } = bound;
        if validate_operation_identity(caller_request_id).is_err()
            || validate_operation_identity(operation_id).is_err()
            || scope_id.is_some_and(|scope| scope.is_empty() || scope.len() > 256)
        {
            return Err(CallerJournalError::InvalidRequest);
        }
        let command_hash = self.command_hash(exact_command);
        if let Some(saved) = self.lookup(principal, kind, caller_request_id).await? {
            if !constant_time_eq_32(&saved.command_hash, &command_hash)
                || saved
                    .operation_id
                    .as_deref()
                    .is_some_and(|saved| saved != operation_id)
            {
                return Err(CallerJournalError::PayloadConflict);
            }
            if saved.operation_id.is_some() {
                return Ok(CallerJournalOutcome::Recorded);
            }
        }

        let key = self.key(principal, kind, caller_request_id);
        let sealed_command = self
            .authority
            .seal_command(
                &self.seal_context(principal, kind, caller_request_id),
                exact_command,
            )
            .map_err(|_| CallerJournalError::RejectedBeforeCommit)?;
        let owner_key = self.authority.digest(
            format!(
                "caller-attempt-bound-owner/v1\0{principal}\0{}\0{}\0{caller_request_id}\0{operation_id}",
                self.installation_id,
                kind.label()
            )
            .as_bytes(),
        );
        let payload_hash = self.authority.digest(
            format!(
                "caller-attempt-bound-state/v1\0{}\0{}\0{}",
                hex_encode(&command_hash),
                operation_id,
                hex_encode(&binding_hmac)
            )
            .as_bytes(),
        );
        let commit_id = self.commit_id(principal, kind, caller_request_id, "bound-pending");
        let batch = LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: kind.into(),
                idempotency_key: format!("attempt.{}.bound", hex_encode(&owner_key)),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::CallerAttempt(CallerAttemptMutation {
                    key: key.clone(),
                    scope_id: scope_id.map(str::to_string),
                    command_hash,
                    sealed_command,
                    resolution: CallerAttemptResolution::Pending,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).expect("zero revision"),
                }),
                LocalStateMutation::OperationBinding(OperationBindingMutation {
                    key,
                    operation_id: operation_id.to_string(),
                    binding_hmac,
                }),
            ],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                Ok(CallerJournalOutcome::Recorded)
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                match self.repository.resolve_commit(commit_id).await {
                    Ok(CommitResolution::Committed(_)) => Ok(CallerJournalOutcome::Recorded),
                    Ok(CommitResolution::NotCommitted) => {
                        Err(CallerJournalError::RejectedBeforeCommit)
                    }
                    Err(_) => Err(CallerJournalError::OutcomeUnknown),
                }
            }
            Err(CommitBatchError::PayloadConflict) => {
                let saved = self.lookup(principal, kind, caller_request_id).await?;
                if saved.as_ref().is_some_and(|saved| {
                    constant_time_eq_32(&saved.command_hash, &command_hash)
                        && saved.operation_id.as_deref() == Some(operation_id)
                }) {
                    Ok(CallerJournalOutcome::Recorded)
                } else {
                    Err(CallerJournalError::PayloadConflict)
                }
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(CallerJournalError::ShutdownInProgress)
            }
            Err(_) => Err(CallerJournalError::RejectedBeforeCommit),
        }
    }

    pub(crate) fn open_attempt_command(
        &self,
        attempt: &crate::domain::local_event::CallerAttemptView,
    ) -> Result<Vec<u8>, CallerJournalError> {
        let exact_command = self
            .authority
            .open_command(
                &self.seal_context(
                    &attempt.key.principal,
                    attempt.key.kind,
                    &attempt.key.caller_request_id,
                ),
                &attempt.sealed_command,
            )
            .map_err(|_| CallerJournalError::RejectedBeforeCommit)?;
        if !constant_time_eq_32(&attempt.command_hash, &self.command_hash(&exact_command)) {
            return Err(CallerJournalError::PayloadConflict);
        }
        Ok(exact_command)
    }

    /// Cursor-paged owner-private supervision feed. The cursor is bound to
    /// principal, generation and scope so it cannot be moved between callers.
    pub async fn pending_page_for_scope(
        &self,
        principal: &str,
        scope_id: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<PendingCallerAttemptPage, CallerJournalError> {
        if principal.is_empty() || scope_id.is_empty() || limit == 0 || limit > 128 {
            return Err(CallerJournalError::InvalidRequest);
        }
        let (after_kind, after_caller_request_id) = cursor
            .map(|cursor| self.decode_page_cursor(principal, scope_id, cursor))
            .transpose()?
            .map_or((None, None), |(kind, id)| (Some(kind), Some(id)));
        let result = self
            .repository
            .query(LocalEventQuery::CallerAttemptPage {
                principal: principal.to_string(),
                installation_id: self.installation_id.clone(),
                scope_id: scope_id.to_string(),
                limit,
                after_kind,
                after_caller_request_id,
            })
            .await
            .map_err(|_| CallerJournalError::RejectedBeforeCommit)?;
        let LocalEventQueryResult::CallerAttemptPage(entries) = result else {
            return Err(CallerJournalError::RejectedBeforeCommit);
        };
        let next_cursor = (entries.len() == limit)
            .then(|| entries.last())
            .flatten()
            .map(|entry| self.encode_page_cursor(principal, scope_id, &entry.key));
        let entries = entries
            .into_iter()
            .map(|entry| PendingCallerAttempt {
                kind: entry.key.kind,
                caller_request_id: entry.key.caller_request_id,
                operation_id: entry.operation_id,
                resolution: entry.resolution,
            })
            .collect();
        Ok(PendingCallerAttemptPage {
            entries,
            next_cursor,
        })
    }

    fn encode_page_cursor(
        &self,
        principal: &str,
        scope_id: &str,
        key: &CallerOperationKey,
    ) -> String {
        let payload = format!("{}\0{}", key.kind.label(), key.caller_request_id);
        let binding = format!(
            "caller-attempt-page/v1\0{principal}\0{}\0{scope_id}\0{payload}",
            self.installation_id
        );
        format!(
            "{}.{}",
            hex::encode(payload.as_bytes()),
            hex_encode(&self.authority.mac(binding.as_bytes()))
        )
    }

    fn decode_page_cursor(
        &self,
        principal: &str,
        scope_id: &str,
        cursor: &str,
    ) -> Result<(OperationKind, String), CallerJournalError> {
        let (payload_hex, mac_hex) = cursor
            .split_once('.')
            .ok_or(CallerJournalError::InvalidRequest)?;
        let payload = hex::decode(payload_hex).map_err(|_| CallerJournalError::InvalidRequest)?;
        let payload = String::from_utf8(payload).map_err(|_| CallerJournalError::InvalidRequest)?;
        let binding = format!(
            "caller-attempt-page/v1\0{principal}\0{}\0{scope_id}\0{payload}",
            self.installation_id
        );
        let supplied = hex::decode(mac_hex).map_err(|_| CallerJournalError::InvalidRequest)?;
        let supplied: [u8; 32] = supplied
            .try_into()
            .map_err(|_| CallerJournalError::InvalidRequest)?;
        if !constant_time_eq_32(&self.authority.mac(binding.as_bytes()), &supplied) {
            return Err(CallerJournalError::InvalidRequest);
        }
        let (kind, caller_request_id) = payload
            .split_once('\0')
            .ok_or(CallerJournalError::InvalidRequest)?;
        let kind = OperationKind::parse(kind).ok_or(CallerJournalError::InvalidRequest)?;
        validate_operation_identity(caller_request_id)
            .map_err(|_| CallerJournalError::InvalidRequest)?;
        Ok((kind, caller_request_id.to_string()))
    }

    async fn resolve_attempt(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
        exact_command: &[u8],
        accepted: bool,
        absent_is_noop: bool,
    ) -> Result<bool, CallerJournalError> {
        if validate_operation_identity(caller_request_id).is_err() {
            return Err(CallerJournalError::InvalidRequest);
        }
        let command_hash = self.command_hash(exact_command);
        let Some(saved) = self.lookup(principal, kind, caller_request_id).await? else {
            return if absent_is_noop {
                Ok(false)
            } else {
                Err(CallerJournalError::RejectedBeforeCommit)
            };
        };
        if !constant_time_eq_32(&saved.command_hash, &command_hash) {
            return Err(CallerJournalError::PayloadConflict);
        }
        if saved.resolution == CallerAttemptResolution::Cleared {
            return Ok(true);
        }
        match (saved.resolution, accepted) {
            (CallerAttemptResolution::Accepted, true)
            | (CallerAttemptResolution::RejectedBeforeCommit, false)
            | (CallerAttemptResolution::Pending, _) => {}
            (CallerAttemptResolution::Accepted, false)
            | (CallerAttemptResolution::RejectedBeforeCommit, true) => {
                return Err(CallerJournalError::PayloadConflict)
            }
            (CallerAttemptResolution::Cleared, _) => unreachable!("handled above"),
        }
        let resolution = if accepted {
            CallerAttemptResolution::Accepted
        } else {
            CallerAttemptResolution::RejectedBeforeCommit
        };
        let mut revision = saved.revision;
        if saved.resolution == CallerAttemptResolution::Pending {
            let resolved_revision = revision
                .next()
                .ok_or(CallerJournalError::RejectedBeforeCommit)?;
            self.commit_attempt(AttemptCommit {
                principal,
                kind,
                caller_request_id,
                exact_command,
                scope_id: saved.scope_id.as_deref(),
                resolution,
                expected: RevisionGuard::Expected(revision),
                revision: resolved_revision.value(),
                step: "resolved",
            })
            .await?;
            revision = resolved_revision;
        }
        // Accepted attempts remain discoverable until a caller acknowledgement
        // or retention pass clears them. Clearing here would lose the only
        // backend-owned identity when an Accepted response is dropped.
        if accepted {
            return Ok(true);
        }
        let cleared_revision = revision
            .next()
            .ok_or(CallerJournalError::RejectedBeforeCommit)?;
        self.commit_attempt(AttemptCommit {
            principal,
            kind,
            caller_request_id,
            exact_command,
            scope_id: saved.scope_id.as_deref(),
            resolution: CallerAttemptResolution::Cleared,
            expected: RevisionGuard::Expected(revision),
            revision: cleared_revision.value(),
            step: "cleared",
        })
        .await?;
        Ok(true)
    }

    /// Clear the journal entry after the dispatch outcome is confirmed as
    /// Accepted or as a deterministic pre-commit rejection. Never called for
    /// an unknown outcome: the entry must survive for reconnection.
    pub async fn clear_attempt(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
        exact_command: &[u8],
        accepted: bool,
    ) -> Result<(), CallerJournalError> {
        self.resolve_attempt(
            principal,
            kind,
            caller_request_id,
            exact_command,
            accepted,
            false,
        )
        .await
        .map(|_| ())
    }

    /// A renderer acknowledgement ends retry-payload retention. The durable
    /// operation binding/receipt remains queryable; only the confidential
    /// caller outbox material is removed.
    pub async fn acknowledge_attempt(
        &self,
        principal: &str,
        kind: OperationKind,
        caller_request_id: &str,
    ) -> Result<(), CallerJournalError> {
        let saved = self
            .lookup(principal, kind, caller_request_id)
            .await?
            .ok_or(CallerJournalError::RejectedBeforeCommit)?;
        if saved.resolution == CallerAttemptResolution::Cleared {
            return Ok(());
        }
        if !matches!(
            saved.resolution,
            CallerAttemptResolution::Accepted | CallerAttemptResolution::RejectedBeforeCommit
        ) {
            return Err(CallerJournalError::InvalidRequest);
        }
        let exact_command = self
            .authority
            .open_command(
                &self.seal_context(principal, kind, caller_request_id),
                &saved.sealed_command,
            )
            .map_err(|_| CallerJournalError::RejectedBeforeCommit)?;
        if !constant_time_eq_32(&saved.command_hash, &self.command_hash(&exact_command)) {
            return Err(CallerJournalError::PayloadConflict);
        }
        let revision = saved
            .revision
            .next()
            .ok_or(CallerJournalError::RejectedBeforeCommit)?;
        self.commit_attempt(AttemptCommit {
            principal,
            kind,
            caller_request_id,
            exact_command: &exact_command,
            scope_id: saved.scope_id.as_deref(),
            resolution: CallerAttemptResolution::Cleared,
            expected: RevisionGuard::Expected(saved.revision),
            revision: revision.value(),
            step: "acknowledged",
        })
        .await
    }
}
