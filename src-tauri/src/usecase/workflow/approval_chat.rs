//! Approval chat preparation usecase.
//!
//! The usecase owns the sequence for approval chat messages: validate the
//! external input, resolve the runtime target, then validate the instruction
//! against that target.

use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::command::WorkflowRuntimeCommandPreflight;
use crate::usecase::workflow::ports::{ApprovalChatTarget, WorkflowApprovalChatGateway};

#[derive(Clone)]
pub struct WorkflowApprovalChatUsecase {
    runtime: Arc<dyn WorkflowApprovalChatGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowApprovalChatUsecase {
    pub fn new(runtime: Arc<dyn WorkflowApprovalChatGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub async fn prepare_approval_chat(
        &self,
        run_id: &str,
        content: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError> {
        self.preflight.validate_approval_chat(run_id, content)?;
        let target = self.runtime.resolve_approval_chat_target(run_id).await?;
        self.runtime
            .validate_approval_chat_instruction(&target.chat_session_id, content)
            .await?;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        validation_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl WorkflowApprovalChatGateway for FakeRuntimeGateway {
        async fn resolve_approval_chat_target(
            &self,
            _run_id: &str,
        ) -> Result<ApprovalChatTarget, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_target");
            Ok(ApprovalChatTarget {
                chat_session_id: "chat".to_string(),
                worktree_path: "/tmp/wt".to_string(),
            })
        }

        async fn validate_approval_chat_instruction(
            &self,
            _chat_session_id: &str,
            _content: &str,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("validate_instruction");
            if let Some(message) = self.validation_error {
                Err(WorkflowError::validation(message))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn rejects_invalid_input_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowApprovalChatUsecase::new(gateway.clone());

        let err = usecase
            .prepare_approval_chat("not-a-uuid", "ok")
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolves_target_then_validates_instruction() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowApprovalChatUsecase::new(gateway.clone());

        let target = usecase
            .prepare_approval_chat("00000000-0000-0000-0000-000000000001", "revise")
            .await
            .unwrap();

        assert_eq!(target.chat_session_id, "chat");
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["resolve_target", "validate_instruction"]
        );
    }

    #[tokio::test]
    async fn returns_instruction_validation_error() {
        let gateway = Arc::new(FakeRuntimeGateway {
            validation_error: Some("invalid instruction"),
            ..Default::default()
        });
        let usecase = WorkflowApprovalChatUsecase::new(gateway.clone());

        let err = usecase
            .prepare_approval_chat("00000000-0000-0000-0000-000000000001", "revise")
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["resolve_target", "validate_instruction"]
        );
    }
}
