use std::io::BufReader;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::layout::{
    event_log_file_in_dir, meta_event_transaction_file_in_dir, meta_file_in_dir,
    sync_file_and_parent, sync_parent_dir, validate_meta, write_json_pretty_atomic,
    write_json_pretty_atomic_durable,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::SessionMeta;

const META_EVENT_TRANSACTION_VERSION: u32 = 1;

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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionMetaEventTransaction {
    pub(super) version: u32,
    pub(super) session_id: String,
    pub(super) base_event_count: usize,
    pub(super) meta: SessionMeta,
    pub(super) events: Vec<AgentSessionEvent>,
}

impl SessionMetaEventTransaction {
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

impl FileSessionStorage {
    pub(super) fn commit_meta_event_transaction(
        &self,
        dir: &Path,
        transaction: &SessionMetaEventTransaction,
    ) -> Result<(), String> {
        let path = meta_event_transaction_file_in_dir(dir);
        write_json_pretty_atomic_durable(&path, transaction, "session meta/event transaction")?;
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
    ) -> Result<(), String> {
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
                return Err(format!(
                    "Failed to open session meta/event transaction: {error}"
                ));
            }
        };
        let transaction: SessionMetaEventTransaction =
            serde_json::from_reader(BufReader::new(file)).map_err(|error| {
                format!("Failed to parse session meta/event transaction: {error}")
            })?;
        if transaction.version != META_EVENT_TRANSACTION_VERSION {
            return Err(format!(
                "Unsupported session meta/event transaction version: {}",
                transaction.version
            ));
        }
        if transaction.session_id != expected_session_id {
            return Err("Session meta/event transaction id mismatch".to_string());
        }
        let meta = validate_meta(transaction.meta.clone(), expected_session_id)?;
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
            return Err("Session event log is shorter than the transaction base".to_string());
        }
        let appended_count = current_events.len() - transaction.base_event_count;
        if appended_count > transaction.events.len()
            || current_events[transaction.base_event_count..]
                != transaction.events[..appended_count]
        {
            return Err("Session event log diverged from the committed transaction".to_string());
        }
        for event in &transaction.events[appended_count..] {
            #[cfg(test)]
            if let Some(hook) = self.transaction_apply_hook.read().clone() {
                hook(is_recovery_completion, TransactionApplyStep::Events)?;
            }
            self.append_session_event_to_dir(dir, event)?;
        }
        if !transaction.events.is_empty() {
            sync_file_and_parent(&event_log_file_in_dir(dir), "session event log")?;
        }
        #[cfg(test)]
        if let Some(hook) = self.transaction_apply_hook.read().clone() {
            hook(is_recovery_completion, TransactionApplyStep::Meta)?;
        }
        write_json_pretty_atomic(&meta_file_in_dir(dir), &meta, "session meta")?;
        sync_file_and_parent(&meta_file_in_dir(dir), "session meta")?;
        #[cfg(test)]
        if let Some(hook) = self.transaction_apply_hook.read().clone() {
            hook(is_recovery_completion, TransactionApplyStep::Cleanup)?;
        }
        std::fs::remove_file(&path)
            .map_err(|error| format!("Failed to finish session meta/event transaction: {error}"))?;
        sync_parent_dir(&path, "session meta/event transaction")?;
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
