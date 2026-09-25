mod abort_execution;
mod approval;
mod preflight;
mod retry_node;
mod start_execution;
mod submit_output;

pub use abort_execution::AbortExecutionCommand;
pub(crate) use abort_execution::WorkflowAbortExecutionUsecase;
pub use approval::ApprovalCommand;
pub(crate) use preflight::WorkflowRuntimeCommandPreflight;
pub(crate) use retry_node::WorkflowRetryNodeUsecase;
pub use retry_node::{ResumeSessionNodeCommand, RetryNodeCommand};
pub(crate) use start_execution::WorkflowStartExecutionUsecase;
pub use start_execution::{ResolvedStartExecutionCommand, StartExecutionCommand};
pub(crate) use submit_output::WorkflowSubmitOutputUsecase;
pub use submit_output::{SubmitOutputArtifact, SubmitOutputCommand};

pub(crate) async fn retry_control_plane_conflicts<T, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    target: &str,
    operation: F,
) -> Result<T, crate::domain::workflow::WorkflowError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::domain::workflow::WorkflowError>>,
{
    retry_control_plane_operation(queue, target, operation).await
}

pub(crate) async fn retry_control_plane_operation<T, E, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    target: &str,
    operation: F,
) -> Result<T, E>
where
    E: crate::domain::failure::ClassifiedFailure + std::fmt::Debug,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    crate::usecase::work_queue::retry(
        queue,
        crate::usecase::work_queue::WorkKey::new("workflow_control_plane", target),
        crate::common::retry::RetryBackoff::CONFLICT,
        operation,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition, WorkflowError};
    use crate::usecase::workflow::control_plane::{
        WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
    };
    use crate::usecase::workflow::ports::{
        WorkflowAbortExecutionGateway, WorkflowStartExecutionGateway,
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
    impl WorkflowControlPlaneGateway for FakeRuntimeGateway {
        fn node_process_presence(
            &self,
            _execution: &crate::domain::workflow::entities::workflow_execution::ExecutionTree,
            _id: &str,
        ) -> Result<
            crate::domain::workflow::NodeProcessPresence,
            crate::domain::workflow::WorkflowError,
        > {
            Ok(crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent)
        }
        fn worktree_exists(
            &self,
            _path: &str,
        ) -> Result<bool, crate::domain::workflow::WorkflowError> {
            Ok(true)
        }
        async fn session_conversation_exists(
            &self,
            _session_id: &str,
        ) -> Result<bool, crate::domain::workflow::WorkflowError> {
            Ok(true)
        }
        async fn resume_session_process(
            &self,
            _execution_id: &str,
            _node_id: &str,
            _session_id: &str,
        ) -> Result<(), crate::domain::workflow::WorkflowError> {
            Ok(())
        }

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
            Option<crate::domain::workflow::entities::workflow_execution::ExecutionTree>,
            WorkflowError,
        > {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn register_started_execution_tree(
            &self,
            _tree_id: &str,
        ) -> Result<(), WorkflowError> {
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn approval_persisted(
            &self,
            _execution_id: &str,
            _node_name: &str,
            _node_execution_id: Option<&str>,
        ) -> Result<bool, WorkflowError> {
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

        let result = super::retry_control_plane_conflicts(
            crate::usecase::work_queue::shared(),
            "test",
            || async {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(WorkflowError::Conflict("stale workflow head".to_string()))
                } else {
                    Ok("committed")
                }
            },
        )
        .await;

        assert_eq!(result.unwrap(), "committed");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_競合再試行_4回を超えて収束する() {
        let attempts = AtomicUsize::new(0);

        let result = super::retry_control_plane_conflicts(
            crate::usecase::work_queue::shared(),
            "test",
            || async {
                if attempts.fetch_add(1, Ordering::SeqCst) < 5 {
                    Err(WorkflowError::Conflict("stale workflow head".to_string()))
                } else {
                    Ok(())
                }
            },
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 6);
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
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["resolve_worktree", "resolve_workflow", "start", "abort",]
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
        assert!(WorkflowSubmitOutputUsecase::new(
            crate::usecase::work_queue::shared().clone(),
            gateway.clone()
        )
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
