//! Agent turn completion workflow usecase.
//!
//! This module owns the orchestration around an agent turn finishing. The
//! runtime gateway only exposes primitive infrastructure operations.

use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::command::WorkflowRuntimeCommandPreflight;
use crate::usecase::workflow::ports::{
    WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway, WorkflowTurnCompleteNotification,
};

#[derive(Clone)]
pub struct WorkflowTurnCompleteUsecase {
    runtime: Arc<dyn WorkflowTurnCompleteGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowTurnCompleteUsecase {
    pub fn new(runtime: Arc<dyn WorkflowTurnCompleteGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    #[allow(dead_code)] // issues-1301 B-3/G-1: retained for workflow step guards around agent turn completion.
    pub async fn is_session_running(&self, chat_session_id: &str) -> bool {
        self.runtime.is_session_running(chat_session_id).await
    }

    pub async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteNotification,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_turn_complete(&command)?;
        if !self
            .runtime
            .is_session_running(&command.chat_session_id)
            .await
        {
            return Ok(());
        }
        if command.interrupted && command.exit_code == 0 && command.failure_signal.is_none() {
            return Ok(());
        }
        self.runtime
            .complete_turn(WorkflowTurnCompleteCommand {
                chat_session_id: command.chat_session_id,
                exit_code: command.exit_code,
                final_text_parts: command.final_text_parts,
                failure_signal: command.failure_signal,
                token_usage: command.token_usage,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        completed_commands: Mutex<Vec<WorkflowTurnCompleteCommand>>,
        session_running: bool,
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteGateway for FakeRuntimeGateway {
        async fn is_session_running(&self, _chat_session_id: &str) -> bool {
            self.calls.lock().unwrap().push("is_running");
            self.session_running
        }

        async fn complete_turn(
            &self,
            command: WorkflowTurnCompleteCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("complete_turn");
            self.completed_commands.lock().unwrap().push(command);
            Ok(())
        }
    }

    #[tokio::test]
    async fn rejects_invalid_turn_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        let err = usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: " ".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                failure_signal: None,
                token_usage: None,
                interrupted: false,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn returns_when_session_is_not_running() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                failure_signal: None,
                token_usage: None,
                interrupted: false,
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["is_running"]);
    }

    #[tokio::test]
    async fn completes_running_turn() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: vec!["done".to_string()],
                failure_signal: None,
                token_usage: None,
                interrupted: false,
            })
            .await
            .unwrap();

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["is_running", "complete_turn"]
        );
    }

    #[tokio::test]
    async fn interrupted_turn_skips_normal_completion() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                failure_signal: None,
                token_usage: None,
                interrupted: true,
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["is_running"]);
    }

    #[tokio::test]
    async fn runtime_interruption_with_non_zero_exit_is_completed_for_failure_policy() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 124,
                final_text_parts: Vec::new(),
                failure_signal: None,
                token_usage: None,
                interrupted: true,
            })
            .await
            .unwrap();

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["is_running", "complete_turn"]
        );
    }

    #[tokio::test]
    async fn interrupted_turn_with_failure_signal_is_completed_for_failure_policy() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                failure_signal: Some(
                    crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal,
                ),
                token_usage: None,
                interrupted: true,
            })
            .await
            .unwrap();

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["is_running", "complete_turn"]
        );
        assert_eq!(
            gateway.completed_commands.lock().unwrap()[0].failure_signal,
            Some(crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal)
        );
    }
}
