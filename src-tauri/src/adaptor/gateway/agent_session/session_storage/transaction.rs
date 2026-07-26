use std::io::BufReader;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::layout::write_json_pretty_atomic_durable;
use super::layout::{
    meta_event_transaction_file_in_dir, meta_file_in_dir, sync_file_and_parent, sync_parent_dir,
    validate_meta, write_json_pretty_atomic,
};
use super::stored_event_v1::StoredAgentSessionEventV1;
use super::FileSessionStorage;
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::SessionMeta;

const META_EVENT_TRANSACTION_VERSION: u32 = 1;

#[derive(Debug)]
pub(super) enum TransactionApplyError {
    Corrupt(String),
    Retryable(String),
}

impl TransactionApplyError {
    pub(super) fn corrupt(message: impl Into<String>) -> Self {
        Self::Corrupt(message.into())
    }

    pub(super) fn retryable(message: impl Into<String>) -> Self {
        Self::Retryable(message.into())
    }

    pub(super) fn is_corrupt(&self) -> bool {
        matches!(self, Self::Corrupt(_))
    }

    pub(super) fn into_message(self) -> String {
        match self {
            Self::Corrupt(message) | Self::Retryable(message) => message,
        }
    }
}

impl std::fmt::Display for TransactionApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corrupt(message) | Self::Retryable(message) => formatter.write_str(message),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionApplyStep {
    Events,
    Meta,
    Cleanup,
}

#[cfg(test)]
pub(super) type TransactionApplyHook =
    std::sync::Arc<dyn Fn(bool, TransactionApplyStep) -> Result<(), String> + Send + Sync>;

#[derive(Debug)]
pub(super) struct SessionMetaEventTransaction {
    pub(super) version: u32,
    pub(super) session_id: String,
    pub(super) base_event_count: usize,
    pub(super) meta: SessionMeta,
    pub(super) events: Vec<AgentSessionEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSessionMetaEventTransactionV1 {
    version: u32,
    session_id: String,
    base_event_count: usize,
    meta: SessionMeta,
    events: Vec<StoredAgentSessionEventV1>,
}

impl From<&SessionMetaEventTransaction> for StoredSessionMetaEventTransactionV1 {
    fn from(value: &SessionMetaEventTransaction) -> Self {
        Self {
            version: value.version,
            session_id: value.session_id.clone(),
            base_event_count: value.base_event_count,
            meta: value.meta.clone(),
            events: value.events.iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<StoredSessionMetaEventTransactionV1> for SessionMetaEventTransaction {
    type Error = String;

    fn try_from(value: StoredSessionMetaEventTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            version: value.version,
            session_id: value.session_id,
            base_event_count: value.base_event_count,
            meta: value.meta,
            events: value
                .events
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl SessionMetaEventTransaction {
    #[cfg(test)]
    pub(super) fn new(
        session_id: &str,
        base_event_count: usize,
        meta: SessionMeta,
        events: &[AgentSessionEvent],
    ) -> Self {
        Self {
            version: META_EVENT_TRANSACTION_VERSION,
            session_id: session_id.to_string(),
            base_event_count,
            meta,
            events: events.to_vec(),
        }
    }
}

#[cfg(test)]
pub(super) fn encode_transaction_v1(
    transaction: &SessionMetaEventTransaction,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(&StoredSessionMetaEventTransactionV1::from(transaction))
}

impl FileSessionStorage {
    #[cfg(test)]
    pub(super) fn commit_meta_event_transaction(
        &self,
        dir: &Path,
        transaction: &SessionMetaEventTransaction,
    ) -> Result<(), String> {
        let path = meta_event_transaction_file_in_dir(dir);
        write_json_pretty_atomic_durable(
            &path,
            &StoredSessionMetaEventTransactionV1::from(transaction),
            "session meta/event transaction",
        )?;
        match self.apply_committed_meta_event_transaction(dir, &transaction.session_id) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.materialization_pending_sessions
                    .write()
                    .insert(transaction.session_id.clone());
                log::warn!(
                    "session transaction for {} committed with materialization pending: {error}",
                    transaction.session_id
                );
                Ok(())
            }
        }
    }

    pub(super) fn apply_committed_meta_event_transaction(
        &self,
        dir: &Path,
        expected_session_id: &str,
    ) -> Result<(), TransactionApplyError> {
        let path = meta_event_transaction_file_in_dir(dir);
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.materialization_pending_sessions
                    .write()
                    .remove(expected_session_id);
                return Ok(());
            }
            Err(error) => {
                return Err(TransactionApplyError::retryable(format!(
                    "Failed to open session meta/event transaction: {error}"
                )));
            }
        };
        let stored: StoredSessionMetaEventTransactionV1 =
            serde_json::from_reader(BufReader::new(file)).map_err(|error| {
                TransactionApplyError::corrupt(format!(
                    "Failed to parse session meta/event transaction: {error}"
                ))
            })?;
        let transaction: SessionMetaEventTransaction =
            stored.try_into().map_err(TransactionApplyError::corrupt)?;
        if transaction.version != META_EVENT_TRANSACTION_VERSION {
            return Err(TransactionApplyError::corrupt(format!(
                "Unsupported session meta/event transaction version: {}",
                transaction.version
            )));
        }
        if transaction.session_id != expected_session_id {
            return Err(TransactionApplyError::corrupt(
                "Session meta/event transaction id mismatch",
            ));
        }
        let meta = validate_meta(transaction.meta.clone(), expected_session_id)
            .map_err(TransactionApplyError::corrupt)?;
        #[cfg(test)]
        let is_recovery_completion = transaction.events.iter().any(|event| {
            matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            )
        });
        // An interrupted in-place append can leave a valid event prefix without the
        // closing array bracket (or with a partial trailing event). Repair that
        // prefix durably before deciding which committed events still need to be
        // materialized, so a later crash always restarts from canonical JSON.
        let current_events = self.canonicalize_session_events_from_dir(dir)?;
        if current_events.len() < transaction.base_event_count {
            return Err(TransactionApplyError::corrupt(
                "Session event log is shorter than the transaction base",
            ));
        }
        let appended_count = current_events.len() - transaction.base_event_count;
        if appended_count > transaction.events.len()
            || current_events[transaction.base_event_count..]
                != transaction.events[..appended_count]
        {
            return Err(TransactionApplyError::corrupt(
                "Session event log diverged from the committed transaction",
            ));
        }
        for event in &transaction.events[appended_count..] {
            #[cfg(test)]
            if let Some(hook) = self.transaction_apply_hook.read().clone() {
                hook(is_recovery_completion, TransactionApplyStep::Events)
                    .map_err(TransactionApplyError::retryable)?;
            }
            self.append_session_event_to_dir(dir, event)
                .map_err(TransactionApplyError::retryable)?;
        }
        if !transaction.events.is_empty() {
            sync_file_and_parent(&Self::event_append_file_in_dir(dir), "session event log")
                .map_err(TransactionApplyError::retryable)?;
        }
        #[cfg(test)]
        if let Some(hook) = self.transaction_apply_hook.read().clone() {
            hook(is_recovery_completion, TransactionApplyStep::Meta)
                .map_err(TransactionApplyError::retryable)?;
        }
        write_json_pretty_atomic(&meta_file_in_dir(dir), &meta, "session meta")
            .map_err(TransactionApplyError::retryable)?;
        sync_file_and_parent(&meta_file_in_dir(dir), "session meta")
            .map_err(TransactionApplyError::retryable)?;
        #[cfg(test)]
        if let Some(hook) = self.transaction_apply_hook.read().clone() {
            hook(is_recovery_completion, TransactionApplyStep::Cleanup)
                .map_err(TransactionApplyError::retryable)?;
        }
        std::fs::remove_file(&path).map_err(|error| {
            TransactionApplyError::retryable(format!(
                "Failed to finish session meta/event transaction: {error}"
            ))
        })?;
        sync_parent_dir(&path, "session meta/event transaction")
            .map_err(TransactionApplyError::retryable)?;
        self.materialization_pending_sessions
            .write()
            .remove(expected_session_id);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_transaction_apply_hook_for_test(&self, hook: Option<TransactionApplyHook>) {
        *self.transaction_apply_hook.write() = hook;
    }
}
