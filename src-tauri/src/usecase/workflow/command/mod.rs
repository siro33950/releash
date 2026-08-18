mod abort_execution;
mod approval;
mod preflight;
mod resume_execution;
mod retry_node;
mod start_execution;
mod stop_execution;
mod submit_output;

pub use abort_execution::AbortExecutionCommand;
pub(crate) use abort_execution::WorkflowAbortExecutionUsecase;
pub use approval::ApprovalCommand;
pub(crate) use preflight::WorkflowRuntimeCommandPreflight;
pub use resume_execution::ResumeExecutionCommand;
pub(crate) use resume_execution::WorkflowResumeExecutionUsecase;
pub use retry_node::RetryNodeCommand;
pub(crate) use retry_node::WorkflowRetryNodeUsecase;
pub(crate) use start_execution::WorkflowStartExecutionUsecase;
pub use start_execution::{ResolvedStartExecutionCommand, StartExecutionCommand};
pub use stop_execution::StopExecutionCommand;
pub(crate) use stop_execution::WorkflowStopExecutionUsecase;
pub(crate) use submit_output::WorkflowSubmitOutputUsecase;
pub use submit_output::{SubmitOutputArtifact, SubmitOutputCommand};

pub(crate) const CONTROL_PLANE_MAX_ATTEMPTS: usize = 4;

pub(crate) async fn retry_control_plane_conflicts<T, F, Fut>(
    mut operation: F,
) -> Result<T, crate::domain::workflow::WorkflowError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::domain::workflow::WorkflowError>>,
{
    for attempt in 1..=CONTROL_PLANE_MAX_ATTEMPTS {
        match operation().await {
            Err(crate::domain::workflow::WorkflowError::Conflict(_))
                if attempt < CONTROL_PLANE_MAX_ATTEMPTS => {}
            result => return result,
        }
    }
    unreachable!("bounded control-plane retry always returns from the loop")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition, WorkflowError};
    use crate::usecase::workflow::control_plane::{
        WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
    };
    use crate::usecase::workflow::ports::{
        WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway,
        WorkflowStartExecutionGateway, WorkflowStopExecutionGateway,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
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
    impl WorkflowControlPlaneGateway for FakeRuntimeGateway {
        fn current_timestamp(&self) -> f64 {
            100.0
        }

        fn new_node_execution_id(&self) -> String {
            "node-execution-test".to_string()
        }

        async fn resolve_workflow_execution_id(
            &self,
            _node_execution_id: &str,
        ) -> Result<Option<String>, WorkflowError> {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn load_active_execution(
            &self,
            _execution_id: &str,
        ) -> Result<
            Option<crate::domain::workflow::entities::workflow_execution::WorkflowExecution>,
            WorkflowError,
        > {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn recover_active_executions(&self) -> Result<(), WorkflowError> {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn load_persisted_events(
            &self,
            _execution_id: &str,
        ) -> Result<Vec<crate::domain::workflow::WorkflowEvent>, WorkflowError> {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        fn configured_secret_values(&self) -> Vec<String> {
            Vec::new()
        }

        async fn commit_control_plane(
            &self,
            _commit: WorkflowControlPlaneCommit,
        ) -> Result<crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot, WorkflowError>
        {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn finish_control_plane_commit(
            &self,
            _worktree_path: &str,
            _snapshot: &crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot,
            _outcome: Option<crate::usecase::workflow::runtime_driver::NodeOutcome>,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    fn valid_execution_id() -> String {
        "00000000-0000-0000-0000-000000000001".to_string()
    }

    #[tokio::test]
    async fn control_plane_conflict_is_retried_until_the_operation_converges() {
        let attempts = AtomicUsize::new(0);

        let result = super::retry_control_plane_conflicts(|| async {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(WorkflowError::Conflict("stale workflow head".to_string()))
            } else {
                Ok("committed")
            }
        })
        .await;

        assert_eq!(result.unwrap(), "committed");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn control_plane_conflict_retry_is_bounded() {
        let attempts = AtomicUsize::new(0);

        let result = super::retry_control_plane_conflicts(|| async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(WorkflowError::Conflict("stale workflow head".to_string()))
        })
        .await;

        assert!(matches!(result, Err(WorkflowError::Conflict(_))));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            super::CONTROL_PLANE_MAX_ATTEMPTS
        );
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
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            [
                "resolve_worktree",
                "resolve_workflow",
                "start",
                "abort",
                "stop",
                "resume"
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
        assert!(WorkflowSubmitOutputUsecase::new(gateway.clone())
            .execute(SubmitOutputCommand {
                node_execution_id: "node-execution-1".to_string(),
                artifact: Some(SubmitOutputArtifact {
                    contract: " ".to_string(),
                    value: serde_json::json!({}),
                }),
            })
            .await
            .is_err());
        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
