mod abort_execution;
mod approval;
mod preflight;
mod resume_execution;
mod start_execution;
mod stop_execution;
mod submit_output;

pub use abort_execution::AbortExecutionCommand;
pub(crate) use abort_execution::WorkflowAbortExecutionUsecase;
pub use approval::ApprovalCommand;
pub(crate) use approval::WorkflowApprovalUsecase;
pub(crate) use preflight::WorkflowRuntimeCommandPreflight;
pub use resume_execution::ResumeExecutionCommand;
pub(crate) use resume_execution::WorkflowResumeExecutionUsecase;
pub(crate) use start_execution::WorkflowStartExecutionUsecase;
pub use start_execution::{ResolvedStartExecutionCommand, StartExecutionCommand};
pub use stop_execution::StopExecutionCommand;
pub(crate) use stop_execution::WorkflowStopExecutionUsecase;
pub use submit_output::SubmitOutputCommand;
pub(crate) use submit_output::WorkflowSubmitOutputUsecase;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition, WorkflowError};
    use crate::usecase::workflow::ports::{
        WorkflowAbortExecutionGateway, WorkflowApprovalGateway, WorkflowResumeExecutionGateway,
        WorkflowStartExecutionGateway, WorkflowStopExecutionGateway, WorkflowSubmitOutputGateway,
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
            _workflow_name: &str,
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
    impl WorkflowStopExecutionGateway for FakeRuntimeGateway {
        async fn stop_execution(
            &self,
            _command: StopExecutionCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("stop");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowResumeExecutionGateway for FakeRuntimeGateway {
        async fn resume_execution(
            &self,
            _command: ResumeExecutionCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("resume");
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

    fn valid_execution_id() -> String {
        "00000000-0000-0000-0000-000000000001".to_string()
    }

    #[tokio::test]
    async fn command_usecases_delegate_valid_commands() {
        let gateway = Arc::new(FakeRuntimeGateway::default());

        WorkflowStartExecutionUsecase::new(gateway.clone())
            .execute(StartExecutionCommand {
                workflow_name: "wf".to_string(),
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
        WorkflowStopExecutionUsecase::new(gateway.clone())
            .execute(StopExecutionCommand {
                execution_id: valid_execution_id(),
            })
            .await
            .unwrap();
        WorkflowResumeExecutionUsecase::new(gateway.clone())
            .execute(ResumeExecutionCommand {
                execution_id: valid_execution_id(),
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
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            [
                "resolve_worktree",
                "resolve_workflow",
                "start",
                "abort",
                "stop",
                "resume",
                "approval",
                "submit"
            ]
        );
    }

    #[tokio::test]
    async fn command_usecases_reject_invalid_commands_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());

        assert!(WorkflowStartExecutionUsecase::new(gateway.clone())
            .execute(StartExecutionCommand {
                workflow_name: "bad name!".to_string(),
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
        assert!(WorkflowStopExecutionUsecase::new(gateway.clone())
            .execute(StopExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
            })
            .await
            .is_err());
        assert!(WorkflowResumeExecutionUsecase::new(gateway.clone())
            .execute(ResumeExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
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
        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
