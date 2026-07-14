mod abort_execution;
mod approval;
mod pending;
mod preflight;
mod start_execution;
mod submit_output;

pub use abort_execution::AbortExecutionCommand;
pub(crate) use abort_execution::WorkflowAbortExecutionUsecase;
pub use approval::ApprovalCommand;
pub(crate) use approval::WorkflowApprovalUsecase;
pub use pending::WorkflowPendingCommandUsecase;
pub(crate) use pending::WorkflowPendingRuntimeCommandUsecase;
pub(crate) use preflight::WorkflowRuntimeCommandPreflight;
pub(crate) use start_execution::WorkflowStartExecutionUsecase;
pub use start_execution::{ResolvedStartExecutionCommand, StartExecutionCommand};
pub use submit_output::SubmitOutputCommand;
pub(crate) use submit_output::WorkflowSubmitOutputUsecase;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition, WorkflowError};
    use crate::usecase::workflow::ports::{
        PendingRuntimeCommand, PendingRuntimeCommandOutcome, PendingRuntimeCommandPayload,
        WorkflowAbortExecutionGateway, WorkflowApprovalGateway,
        WorkflowPendingRuntimeCommandGateway, WorkflowStartExecutionGateway,
        WorkflowSubmitOutputGateway,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait::async_trait]
    impl WorkflowStartExecutionGateway for FakeRuntimeGateway {
        async fn resolve_start_execution_worktree(
            &self,
            worktree_path: String,
        ) -> Result<String, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_worktree");
            Ok(worktree_path)
        }

        async fn resolve_start_execution_workflow(
            &self,
            _workflow_file_stem: &str,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_workflow");
            Ok(WorkflowDefinition::default())
        }

        async fn start_resolved_execution(
            &self,
            _command: ResolvedStartExecutionCommand,
        ) -> Result<String, WorkflowError> {
            self.calls.lock().unwrap().push("start");
            Ok("00000000-0000-0000-0000-000000000001".to_string())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowAbortExecutionGateway for FakeRuntimeGateway {
        async fn abort_execution(
            &self,
            _command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("abort");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowApprovalGateway for FakeRuntimeGateway {
        async fn resolve_approval(&self, _command: ApprovalCommand) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("approval");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowSubmitOutputGateway for FakeRuntimeGateway {
        async fn submit_output(&self, _command: SubmitOutputCommand) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("submit");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowPendingRuntimeCommandGateway for FakeRuntimeGateway {
        async fn dispatch_pending_command(
            &self,
            _command: PendingRuntimeCommand,
        ) -> PendingRuntimeCommandOutcome {
            self.calls.lock().unwrap().push("pending");
            PendingRuntimeCommandOutcome::Accepted
        }
    }

    fn valid_execution_id() -> String {
        "00000000-0000-0000-0000-000000000001".to_string()
    }

    #[tokio::test]
    async fn command_usecases_delegate_valid_commands() {
        let gateway = Arc::new(FakeRuntimeGateway::default());

        WorkflowStartExecutionUsecase::new(gateway.clone())
            .execute(StartExecutionCommand {
                workflow_file_stem: "wf".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                request: None,
                created_from: ExecutionOrigin::DesktopUi,
                permission_mode: "ask".to_string(),
            })
            .await
            .unwrap();
        WorkflowAbortExecutionUsecase::new(gateway.clone())
            .execute(AbortExecutionCommand {
                execution_id: valid_execution_id(),
                expected_node_name: None,
            })
            .await
            .unwrap();
        WorkflowApprovalUsecase::new(gateway.clone())
            .execute(ApprovalCommand {
                execution_id: valid_execution_id(),
                node_name: "review".to_string(),
                node_execution_id: None,
                comment: None,
            })
            .await
            .unwrap();
        WorkflowSubmitOutputUsecase::new(gateway.clone())
            .execute(SubmitOutputCommand {
                execution_id: valid_execution_id(),
                node_name: "review".to_string(),
                node_execution_id: None,
                contract: "review-fix-tasks".to_string(),
                artifact: serde_json::json!({}),
            })
            .await
            .unwrap();
        let outcome = WorkflowPendingRuntimeCommandUsecase::new(gateway.clone())
            .dispatch(PendingRuntimeCommand {
                execution_id: valid_execution_id(),
                request_id: "00000000-0000-0000-0000-000000000002".to_string(),
                requested_at: 1.0,
                payload: PendingRuntimeCommandPayload::Abort { node_name: None },
            })
            .await;

        assert_eq!(outcome, PendingRuntimeCommandOutcome::Accepted);
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            [
                "resolve_worktree",
                "resolve_workflow",
                "start",
                "abort",
                "approval",
                "submit",
                "pending"
            ]
        );
    }

    #[tokio::test]
    async fn command_usecases_reject_invalid_commands_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());

        assert!(WorkflowStartExecutionUsecase::new(gateway.clone())
            .execute(StartExecutionCommand {
                workflow_file_stem: "bad name!".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                request: None,
                created_from: ExecutionOrigin::DesktopUi,
                permission_mode: "ask".to_string(),
            })
            .await
            .is_err());
        assert!(WorkflowAbortExecutionUsecase::new(gateway.clone())
            .execute(AbortExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
                expected_node_name: None,
            })
            .await
            .is_err());
        assert!(WorkflowApprovalUsecase::new(gateway.clone())
            .execute(ApprovalCommand {
                execution_id: valid_execution_id(),
                node_name: " ".to_string(),
                node_execution_id: None,
                comment: None,
            })
            .await
            .is_err());
        assert!(WorkflowSubmitOutputUsecase::new(gateway.clone())
            .execute(SubmitOutputCommand {
                execution_id: valid_execution_id(),
                node_name: "review".to_string(),
                node_execution_id: None,
                contract: " ".to_string(),
                artifact: serde_json::json!({}),
            })
            .await
            .is_err());
        let outcome = WorkflowPendingRuntimeCommandUsecase::new(gateway.clone())
            .dispatch(PendingRuntimeCommand {
                execution_id: "not-a-uuid".to_string(),
                request_id: "00000000-0000-0000-0000-000000000002".to_string(),
                requested_at: 1.0,
                payload: PendingRuntimeCommandPayload::Abort { node_name: None },
            })
            .await;
        assert!(matches!(
            outcome,
            PendingRuntimeCommandOutcome::RejectedFinal(reason)
                if reason.contains("invalid execution_id")
        ));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
