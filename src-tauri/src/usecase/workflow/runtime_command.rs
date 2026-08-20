use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;

#[cfg(test)]
use super::command::ResolvedStartExecutionCommand;
#[cfg(test)]
use super::command::WorkflowRuntimeCommandPreflight;
use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeExecutionCommand, RetryNodeCommand,
    StartExecutionCommand, StopExecutionCommand, SubmitOutputCommand,
    WorkflowAbortExecutionUsecase, WorkflowResumeExecutionUsecase, WorkflowRetryNodeUsecase,
    WorkflowStartExecutionUsecase, WorkflowStopExecutionUsecase, WorkflowSubmitOutputUsecase,
};
use super::control_plane::{WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase};
use super::ports::WorkflowRuntimeCommandGateway;
#[cfg(test)]
use super::ports::{
    WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway, WorkflowRuntimeShutdownGateway,
    WorkflowRuntimeStateGateway, WorkflowStartExecutionGateway, WorkflowStopExecutionGateway,
};

#[derive(Clone)]
pub struct WorkflowRuntimeUsecase {
    runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
    start_execution: WorkflowStartExecutionUsecase,
    abort_execution: WorkflowAbortExecutionUsecase,
    stop_execution: WorkflowStopExecutionUsecase,
    resume_execution: WorkflowResumeExecutionUsecase,
    retry_node: WorkflowRetryNodeUsecase,
    submit_output: WorkflowSubmitOutputUsecase,
    control_plane: WorkflowControlPlaneUsecase,
    #[cfg(test)]
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowRuntimeUsecase {
    pub fn new(runtime: Arc<dyn WorkflowRuntimeCommandGateway>) -> Self {
        let control_plane_runtime: Arc<dyn WorkflowControlPlaneGateway> = runtime.clone();
        Self {
            runtime: runtime.clone(),
            start_execution: WorkflowStartExecutionUsecase::new(runtime.clone()),
            abort_execution: WorkflowAbortExecutionUsecase::new(runtime.clone()),
            stop_execution: WorkflowStopExecutionUsecase::new(runtime.clone()),
            resume_execution: WorkflowResumeExecutionUsecase::new(runtime.clone()),
            retry_node: WorkflowRetryNodeUsecase::new(control_plane_runtime.clone()),
            submit_output: WorkflowSubmitOutputUsecase::new(control_plane_runtime.clone()),
            control_plane: WorkflowControlPlaneUsecase::new(control_plane_runtime),
            #[cfg(test)]
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub async fn start_execution(
        &self,
        command: StartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        self.start_execution.execute(command).await
    }

    pub async fn recover_startup(&self) -> Result<(), WorkflowError> {
        self.runtime.recover_startup().await
    }

    pub async fn abort_execution(
        &self,
        command: AbortExecutionCommand,
    ) -> Result<(), WorkflowError> {
        self.abort_execution.execute(command).await
    }

    pub async fn stop_execution(&self, command: StopExecutionCommand) -> Result<(), WorkflowError> {
        self.stop_execution.execute(command).await
    }

    pub async fn resume_execution(
        &self,
        command: ResumeExecutionCommand,
    ) -> Result<(), WorkflowError> {
        self.resume_execution.execute(command).await
    }

    pub async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        self.retry_node.execute(command).await
    }

    pub async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.control_plane.resolve_approval(command).await
    }

    pub async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.submit_output.execute(command).await
    }

    pub(crate) async fn record_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderWorkflowStopCommand,
        lifecycle_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        self.control_plane
            .record_provider_stop(command, lifecycle_events)
            .await
    }

    #[cfg(test)]
    pub async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        self.preflight.validate_execution_lookup(execution_id)?;
        self.runtime.get_state_by_execution_id(execution_id).await
    }

    #[cfg(test)]
    pub async fn shutdown_active_commands(&self) {
        self.runtime.shutdown_active_commands().await;
    }

    pub async fn shutdown_execution_commands_for_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> crate::usecase::workflow::ports::WorkflowShutdownEffectReadback {
        self.runtime
            .execute_shutdown_effect(operation_id, effect_identity, owner_revision, execution_id)
            .await
    }

    pub async fn read_shutdown_execution_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> crate::usecase::workflow::ports::WorkflowShutdownEffectReadback {
        self.runtime
            .read_shutdown_effect(operation_id, effect_identity, owner_revision, execution_id)
            .await
    }

    pub async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
        self.runtime
            .application_shutdown_target_execution_ids()
            .await
    }
}

#[async_trait::async_trait]
impl super::WorkspaceNodeWorkflowCommandExecutor for WorkflowRuntimeUsecase {
    async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.resolve_approval(command).await
    }

    async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        self.retry_node(command).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderWorkflowStopTransaction
    for WorkflowRuntimeUsecase
{
    async fn commit_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderWorkflowStopCommand,
        lifecycle_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Result<(), crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError> {
        self.record_provider_stop(command, lifecycle_events)
            .await
            .map_err(|error| match error {
                WorkflowError::Validation(_)
                | WorkflowError::InvalidState(_)
                | WorkflowError::NotFound(_)
                | WorkflowError::UnauthorizedApprovalTarget(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::InvalidInput
                }
                WorkflowError::Conflict(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Conflict
                }
                WorkflowError::StorageUnavailable { .. } | WorkflowError::External(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::StorageUnavailable
                }
                WorkflowError::CorruptStoredState(_)
                | WorkflowError::IncompatibleStoredEvent(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Corrupt
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition};
    use std::sync::Mutex;

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

        fn ensure_node_recovery_available(
            &self,
            _execution_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowError> {
            Ok(())
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
            _commit: crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit,
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

    #[async_trait::async_trait]
    impl WorkflowRuntimeStateGateway for FakeRuntimeGateway {
        async fn recover_startup(&self) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("recover_startup");
            Ok(())
        }

        async fn get_state_by_execution_id(
            &self,
            _execution_id: &str,
        ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
            self.calls.lock().unwrap().push("state_by_execution");
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRuntimeShutdownGateway for FakeRuntimeGateway {
        async fn shutdown_active_commands(&self) {
            self.calls.lock().unwrap().push("shutdown_active_commands");
        }

        async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn runtime_usecase_delegates_runtime_commands() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let _ = usecase
            .start_execution(StartExecutionCommand {
                workflow_name: "wf".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                request: None,
                created_from: ExecutionOrigin::DesktopUi,
            })
            .await
            .unwrap();
        usecase
            .abort_execution(AbortExecutionCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
                expected_node_name: None,
            })
            .await
            .unwrap();
        usecase
            .stop_execution(StopExecutionCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
            })
            .await
            .unwrap();
        usecase
            .resume_execution(ResumeExecutionCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
            })
            .await
            .unwrap();
        let _ = usecase
            .get_state_by_execution_id("00000000-0000-0000-0000-000000000001")
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
                "state_by_execution"
            ]
        );
    }

    #[tokio::test]
    async fn runtime_usecase_delegates_active_command_shutdown() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        usecase.shutdown_active_commands().await;

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["shutdown_active_commands"]
        );
    }

    #[tokio::test]
    async fn start_execution_rejects_invalid_workflow_name_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let err = usecase
            .start_execution(StartExecutionCommand {
                workflow_name: "bad name!".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                request: None,
                created_from: ExecutionOrigin::DesktopUi,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_preflight_rejects_invalid_mutations_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let abort_err = usecase
            .abort_execution(AbortExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
                expected_node_name: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(abort_err, WorkflowError::Validation(_)));

        let stop_err = usecase
            .stop_execution(StopExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
            })
            .await
            .unwrap_err();
        assert!(matches!(stop_err, WorkflowError::Validation(_)));

        let resume_err = usecase
            .resume_execution(ResumeExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
            })
            .await
            .unwrap_err();
        assert!(matches!(resume_err, WorkflowError::Validation(_)));

        let approval_err = usecase
            .resolve_approval(ApprovalCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: " ".to_string(),
                node_execution_id: None,
                comment: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(approval_err, WorkflowError::Validation(_)));

        let submit_err = usecase
            .submit_output(SubmitOutputCommand {
                node_execution_id: "node-execution-1".to_string(),
                artifact: Some(crate::usecase::workflow::command::SubmitOutputArtifact {
                    contract: " ".to_string(),
                    value: serde_json::json!({}),
                }),
            })
            .await
            .unwrap_err();
        assert!(matches!(submit_err, WorkflowError::Validation(_)));

        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_preflight_rejects_invalid_queries_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let execution_err = usecase
            .get_state_by_execution_id("not-a-uuid")
            .await
            .unwrap_err();
        assert!(matches!(execution_err, WorkflowError::Validation(_)));

        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
