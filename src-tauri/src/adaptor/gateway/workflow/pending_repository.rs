use std::path::PathBuf;

use crate::adaptor::gateway::workflow::pending_command::{
    PendingCommand, PendingCommandEntry, PendingCommandPayload, PendingCommandStore,
};
use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::{
    PendingRuntimeCommand, PendingRuntimeCommandOutcome, PendingRuntimeCommandPayload,
    PendingWorkflowCommand, PendingWorkflowCommandRepository,
};
use crate::usecase::workflow::runtime_command::WorkflowRuntimeUsecase;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct PendingWorkflowCommandFileRepository {
    data_dir: PathBuf,
}

impl PendingWorkflowCommandFileRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    fn store(&self) -> PendingCommandStore {
        PendingCommandStore::new(&self.data_dir)
    }
}

impl PendingWorkflowCommandRepository for PendingWorkflowCommandFileRepository {
    fn write_pending(&self, command: PendingWorkflowCommand) -> Result<(), WorkflowError> {
        let payload: PendingCommandPayload =
            serde_json::from_value(command.payload).map_err(|e| {
                WorkflowError::validation(format!("invalid pending workflow command payload: {e}"))
            })?;
        self.store()
            .write_pending(&PendingCommand {
                id: command.command_id,
                execution_id: command.execution_id,
                payload,
                requested_at: command.requested_at,
            })
            .map(|_| ())
            .map_err(|e| WorkflowError::external(e.to_string()))
    }

    #[cfg(test)]
    fn list_pending(&self) -> Result<Vec<PendingWorkflowCommand>, WorkflowError> {
        self.store()
            .list_pending()
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .into_iter()
            .map(|entry| {
                let payload = serde_json::to_value(entry.command.payload)
                    .map_err(|e| WorkflowError::external(e.to_string()))?;
                Ok(PendingWorkflowCommand {
                    command_id: entry.command.id,
                    execution_id: entry.command.execution_id,
                    requested_at: entry.command.requested_at,
                    payload,
                })
            })
            .collect()
    }

    #[cfg(test)]
    fn mark_processed(&self, command_id: &str) -> Result<(), WorkflowError> {
        let store = self.store();
        for entry in store
            .list_pending()
            .map_err(|e| WorkflowError::external(e.to_string()))?
        {
            if entry.command.id != command_id {
                continue;
            }
            if let Some(claim) = store
                .claim_pending(&entry)
                .map_err(|e| WorkflowError::external(e.to_string()))?
            {
                store
                    .mark_processed(&claim.entry)
                    .map_err(|e| WorkflowError::external(e.to_string()))?;
            }
            return Ok(());
        }
        Ok(())
    }
}

pub(crate) async fn process_pending_workflow_command_entry(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    store: &PendingCommandStore,
    entry: PendingCommandEntry,
) {
    let entry_id = entry.command.id.clone();
    let execution_id = entry.command.execution_id.clone();
    let claimed = match store.claim_pending(&entry) {
        Ok(Some(claimed)) => claimed,
        Ok(None) => return,
        Err(e) => {
            log::warn!("pending command claim failed: id={entry_id} execution_id={execution_id} reason={e}");
            return;
        }
    };

    let command = pending_command_to_runtime_command(claimed.entry.command.clone());
    match runtime.dispatch_pending_command(command).await {
        PendingRuntimeCommandOutcome::Accepted => {
            log::info!("pending command dispatched: id={entry_id} execution_id={execution_id}");
            if let Err(e) = store.mark_processed(&claimed.entry) {
                log::warn!(
                    "Failed to mark pending command processed: id={entry_id} execution_id={execution_id} reason={e}"
                );
            }
        }
        PendingRuntimeCommandOutcome::RejectedFinal(reason) => {
            log::warn!(
                "pending command dispatch rejected: id={entry_id} execution_id={execution_id} reason={reason}"
            );
            if let Err(e) = store.mark_processed(&claimed.entry) {
                log::warn!(
                    "Failed to mark rejected pending command processed: id={entry_id} execution_id={execution_id} reason={e}"
                );
            }
        }
        PendingRuntimeCommandOutcome::RetryableFailure(reason) => {
            log::warn!(
                "pending command dispatch retryable failure: id={entry_id} execution_id={execution_id} reason={reason}"
            );
            if let Err(e) = store.release_claim(&claimed.entry) {
                log::warn!(
                    "Failed to release pending command claim: id={entry_id} execution_id={execution_id} reason={e}"
                );
            }
        }
    }
}

fn pending_command_to_runtime_command(pending: PendingCommand) -> PendingRuntimeCommand {
    PendingRuntimeCommand {
        execution_id: pending.execution_id,
        request_id: pending.id,
        requested_at: pending.requested_at,
        payload: pending_payload_to_runtime_payload(pending.payload),
    }
}

fn pending_payload_to_runtime_payload(
    payload: PendingCommandPayload,
) -> PendingRuntimeCommandPayload {
    match payload {
        PendingCommandPayload::Approve {
            node_name,
            node_execution_id,
            comment,
        } => PendingRuntimeCommandPayload::Approve {
            node_name,
            node_execution_id,
            comment,
        },
        PendingCommandPayload::Abort { node_name } => {
            PendingRuntimeCommandPayload::Abort { node_name }
        }
        PendingCommandPayload::SubmitOutput {
            node_name,
            node_execution_id,
            contract,
            artifact,
        } => PendingRuntimeCommandPayload::SubmitOutput {
            node_name,
            node_execution_id,
            contract,
            artifact,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_and_lists_existing_pending_command_shape() {
        let tmp = TempDir::new().unwrap();
        let repo = PendingWorkflowCommandFileRepository::new(tmp.path());
        let command_id = "00000000-0000-4000-8000-000000000010";
        let execution_id = "00000000-0000-4000-8000-000000000011";

        repo.write_pending(PendingWorkflowCommand {
            command_id: command_id.to_string(),
            execution_id: execution_id.to_string(),
            requested_at: 1.0,
            payload: serde_json::json!({
                "kind": "approve",
                "node_name": "review",
                "comment": "ok"
            }),
        })
        .unwrap();

        let pending = repo.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].command_id, command_id);
        assert_eq!(pending[0].payload["kind"], "approve");
    }

    #[test]
    fn mark_processed_claims_and_moves_pending_entry() {
        let tmp = TempDir::new().unwrap();
        let repo = PendingWorkflowCommandFileRepository::new(tmp.path());
        let command_id = "00000000-0000-4000-8000-000000000020";
        repo.write_pending(PendingWorkflowCommand {
            command_id: command_id.to_string(),
            execution_id: "00000000-0000-4000-8000-000000000021".to_string(),
            requested_at: 1.0,
            payload: serde_json::json!({"kind": "abort"}),
        })
        .unwrap();

        repo.mark_processed(command_id).unwrap();

        assert!(repo.list_pending().unwrap().is_empty());
        assert!(tmp
            .path()
            .join("workflow_pending")
            .join("processed")
            .join(format!("{command_id}.json"))
            .exists());
    }
}
