//! Agent turn completion workflow usecase.
//!
//! This module owns the orchestration around an agent turn finishing. The
//! runtime gateway only exposes primitive infrastructure operations.

use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::command::WorkflowRuntimeCommandPreflight;
use crate::usecase::workflow::ports::{WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway};

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

    pub async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteCommand,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_turn_complete(&command)?;
        if !self
            .runtime
            .is_session_running(&command.chat_session_id)
            .await
        {
            return Ok(());
        }
        self.runtime.pickup_pending_submit_outputs().await;
        self.runtime.complete_turn(command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        session_running: bool,
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteGateway for FakeRuntimeGateway {
        async fn is_session_running(&self, _chat_session_id: &str) -> bool {
            self.calls.lock().unwrap().push("is_running");
            self.session_running
        }

        async fn pickup_pending_submit_outputs(&self) {
            self.calls.lock().unwrap().push("pickup_pending");
        }

        async fn complete_turn(
            &self,
            _command: WorkflowTurnCompleteCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("complete_turn");
            Ok(())
        }
    }

    #[tokio::test]
    async fn rejects_invalid_turn_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        let err = usecase
            .complete_turn(WorkflowTurnCompleteCommand {
                chat_session_id: " ".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                token_usage: None,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn returns_without_pickup_when_session_is_not_running() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteCommand {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                token_usage: None,
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["is_running"]);
    }

    #[tokio::test]
    async fn pickups_pending_submit_outputs_before_completing_turn() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowTurnCompleteUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteCommand {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: vec!["done".to_string()],
                token_usage: None,
            })
            .await
            .unwrap();

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["is_running", "pickup_pending", "complete_turn"]
        );
    }
}
