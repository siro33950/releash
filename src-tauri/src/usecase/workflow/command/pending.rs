use std::sync::Arc;

use crate::domain::workflow::WorkflowError;

use crate::usecase::workflow::ports::{
    PendingRuntimeCommand, PendingRuntimeCommandOutcome, PendingWorkflowCommand,
    PendingWorkflowCommandRepository, WorkflowPendingRuntimeCommandGateway,
};

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Clone)]
pub struct WorkflowPendingCommandUsecase {
    pending_commands: Arc<dyn PendingWorkflowCommandRepository>,
}

impl WorkflowPendingCommandUsecase {
    pub fn new(pending_commands: Arc<dyn PendingWorkflowCommandRepository>) -> Self {
        Self { pending_commands }
    }

    pub fn enqueue_pending_command(
        &self,
        command: PendingWorkflowCommand,
    ) -> Result<(), WorkflowError> {
        self.pending_commands.write_pending(command)
    }
}

#[derive(Clone)]
pub(crate) struct WorkflowPendingRuntimeCommandUsecase {
    runtime: Arc<dyn WorkflowPendingRuntimeCommandGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowPendingRuntimeCommandUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowPendingRuntimeCommandGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn dispatch(
        &self,
        command: PendingRuntimeCommand,
    ) -> PendingRuntimeCommandOutcome {
        if let Err(err) = self.preflight.validate_pending_runtime_command(&command) {
            return PendingRuntimeCommandOutcome::RejectedFinal(err.to_string());
        }
        self.runtime.dispatch_pending_command(command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePendingRepository {
        pending: Mutex<Vec<PendingWorkflowCommand>>,
    }

    impl PendingWorkflowCommandRepository for FakePendingRepository {
        fn write_pending(&self, command: PendingWorkflowCommand) -> Result<(), WorkflowError> {
            self.pending.lock().unwrap().push(command);
            Ok(())
        }

        fn list_pending(&self) -> Result<Vec<PendingWorkflowCommand>, WorkflowError> {
            Ok(self.pending.lock().unwrap().clone())
        }

        fn mark_processed(&self, _command_id: &str) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    #[test]
    fn enqueue_pending_command_delegates_to_pending_repository() {
        let repository = Arc::new(FakePendingRepository::default());
        let usecase = WorkflowPendingCommandUsecase::new(repository.clone());

        usecase
            .enqueue_pending_command(PendingWorkflowCommand {
                command_id: "cmd-1".to_string(),
                execution_id: "00000000-0000-4000-8000-000000000034".to_string(),
                requested_at: 1.0,
                payload: serde_json::json!({"kind":"approve"}),
            })
            .unwrap();

        assert_eq!(repository.pending.lock().unwrap().len(), 1);
    }
}
