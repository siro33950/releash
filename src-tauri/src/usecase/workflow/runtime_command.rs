use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;

#[cfg(test)]
use super::command::ResolvedStartExecutionCommand;
#[cfg(test)]
use super::command::WorkflowRuntimeCommandPreflight;
use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeSessionNodeCommand, RetryNodeCommand,
    StartExecutionCommand, SubmitOutputCommand, WorkflowAbortExecutionUsecase,
    WorkflowRetryNodeUsecase, WorkflowStartExecutionUsecase, WorkflowSubmitOutputUsecase,
};
use super::control_plane::{WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase};
use super::ports::WorkflowRuntimeCommandGateway;
#[cfg(test)]
use super::ports::{
    WorkflowAbortExecutionGateway, WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway,
    WorkflowStartExecutionGateway,
};

#[derive(Clone)]
pub struct WorkflowRuntimeUsecase {
    pub(super) worktree_operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    pub(super) execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
    pub(super) runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
    pub(super) archive_locks: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
        >,
    >,
    start_execution: WorkflowStartExecutionUsecase,
    pub(super) abort_execution: WorkflowAbortExecutionUsecase,
    retry_node: WorkflowRetryNodeUsecase,
    submit_output: WorkflowSubmitOutputUsecase,
    control_plane: WorkflowControlPlaneUsecase,
    #[cfg(test)]
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowRuntimeUsecase {
    #[cfg(test)]
    pub fn new(
        runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
        execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
    ) -> Self {
        Self::new_with_worktree_operations(runtime, execution_archives, Default::default())
    }

    pub(crate) fn new_with_worktree_operations(
        runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
        execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
        worktree_operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    ) -> Self {
        let control_plane_runtime: Arc<dyn WorkflowControlPlaneGateway> = runtime.clone();
        Self {
            execution_archives,
            worktree_operations,
            runtime: runtime.clone(),
            archive_locks: Default::default(),
            start_execution: WorkflowStartExecutionUsecase::new(runtime.clone()),
            abort_execution: WorkflowAbortExecutionUsecase::new(runtime.clone()),
            retry_node: WorkflowRetryNodeUsecase::new(control_plane_runtime.clone()),
            submit_output: WorkflowSubmitOutputUsecase::new(control_plane_runtime.clone()),
            control_plane: WorkflowControlPlaneUsecase::new(control_plane_runtime),
            #[cfg(test)]
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) fn with_startup(
        mut self,
        startup: Option<Arc<super::startup::WorkflowStartupUsecase>>,
    ) -> Self {
        self.control_plane = self.control_plane.with_startup(startup);
        self
    }

    pub async fn start_execution(
        &self,
        command: StartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        let _mutation = self.begin_worktree_mutation(&command.worktree_path)?;
        self.start_execution.execute(command).await
    }

    pub async fn recover_startup(&self) -> Result<(), WorkflowError> {
        self.control_plane.recover_startup().await
    }

    pub async fn abort_execution(
        &self,
        command: AbortExecutionCommand,
    ) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(&command.execution_id)?;
        self.abort_execution.execute(command).await
    }

    pub async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(&command.execution_id)?;
        self.retry_node.execute(command).await
    }

    pub async fn resume_session_node_by_id(
        &self,
        node_execution_id: String,
    ) -> Result<(), WorkflowError> {
        let execution_id = self
            .runtime
            .resolve_workflow_execution_id(&node_execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Node execution not found: {node_execution_id}"))
            })?;
        self.resume_session_node(ResumeSessionNodeCommand {
            execution_id,
            node_execution_id,
        })
        .await
    }

    pub async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(&command.execution_id)?;
        self.control_plane.resume_session_node(command).await
    }

    pub async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        let _mutation = self.begin_execution_tree_mutation(&command.execution_id)?;
        self.control_plane.resolve_approval(command).await
    }

    pub async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        super::command::WorkflowRuntimeCommandPreflight.validate_submit_output(&command)?;
        let id = self
            .runtime
            .resolve_workflow_execution_id(&command.node_execution_id)
            .await?
            .ok_or_else(|| WorkflowError::NotFound(command.node_execution_id.clone()))?;
        let _mutation = self.begin_execution_tree_mutation(&id)?;
        self.submit_output.execute(command).await
    }

    pub(crate) async fn record_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
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

    pub async fn shutdown_active_commands(&self) {
        self.runtime.shutdown_active_commands().await;
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
    async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        self.resume_session_node(command).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderExecutionTreeStopTransaction
    for WorkflowRuntimeUsecase
{
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    > {
        self.begin_worktree_mutation(path).map_err(|_| {
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Conflict
        })
    }

    async fn commit_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
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

impl crate::usecase::agent_session::WorktreeMutationAdmission for WorkflowRuntimeUsecase {
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<crate::usecase::worktree_operation::WorktreeMutationGuard, WorkflowError> {
        self.begin_worktree_mutation(path)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::ExecutionTreeCache for WorkflowRuntimeUsecase {
    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::ExecutionTreeCacheReleaseError> {
        self.runtime
            .release_deleted_execution_tree(tree_id)
            .await
            .map_err(|error| match error {
                WorkflowError::StorageUnavailable { .. } | WorkflowError::External(_) => {
                    crate::usecase::agent_session::ExecutionTreeCacheReleaseError::Unavailable
                }
                WorkflowError::Validation(_)
                | WorkflowError::InvalidState(_)
                | WorkflowError::NotFound(_)
                | WorkflowError::UnauthorizedApprovalTarget(_)
                | WorkflowError::Conflict(_)
                | WorkflowError::CorruptStoredState(_)
                | WorkflowError::IncompatibleStoredEvent(_) => {
                    crate::usecase::agent_session::ExecutionTreeCacheReleaseError::Corrupt
                }
            })
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::StartedExecutionTreeRegistrar for WorkflowRuntimeUsecase {
    async fn reserve_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::StartedExecutionTreeRegistrationError> {
        self.runtime
            .reserve_started_execution_tree(tree_id)
            .await
            .map_err(map_started_execution_tree_error)
    }

    async fn register_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::StartedExecutionTreeRegistrationError> {
        self.runtime
            .register_started_execution_tree(tree_id)
            .await
            .map_err(map_started_execution_tree_error)
    }

    async fn release_started_execution_tree_reservation(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::StartedExecutionTreeRegistrationError> {
        self.runtime
            .release_started_execution_tree_reservation(tree_id)
            .await
            .map_err(map_started_execution_tree_error)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::AgentSessionExecutionTreeLifecycle for WorkflowRuntimeUsecase {
    async fn lock_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, WorkflowError> {
        self.lock_execution_tree(tree_id).await
    }

    async fn archive_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.archive_execution_tree(tree_id, "manual").await
    }

    async fn restore_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.restore_execution_tree_locked(tree_id).await
    }
}

fn map_started_execution_tree_error(
    error: WorkflowError,
) -> crate::usecase::agent_session::StartedExecutionTreeRegistrationError {
    match error {
        WorkflowError::StorageUnavailable { .. } | WorkflowError::External(_) => {
            crate::usecase::agent_session::StartedExecutionTreeRegistrationError::Unavailable
        }
        WorkflowError::Validation(_)
        | WorkflowError::InvalidState(_)
        | WorkflowError::NotFound(_)
        | WorkflowError::UnauthorizedApprovalTarget(_)
        | WorkflowError::Conflict(_)
        | WorkflowError::CorruptStoredState(_)
        | WorkflowError::IncompatibleStoredEvent(_) => {
            crate::usecase::agent_session::StartedExecutionTreeRegistrationError::Corrupt
        }
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
        fail_startup: bool,
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
            self.calls.lock().unwrap().push("load_active");
            Ok(None)
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
    impl crate::usecase::workflow::ports::ExecutionTreeProcessGateway for FakeRuntimeGateway {
        async fn stop_execution_tree_processes(&self, _: &str) -> Result<(), WorkflowError> {
            unreachable!("process cleanup is not used by this fixture")
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRuntimeStateGateway for FakeRuntimeGateway {
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
    }

    impl crate::domain::workflow::repository::WorkflowStartupRepository for FakeRuntimeGateway {
        fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
            self.calls.lock().unwrap().push("startup_list");
            if self.fail_startup {
                Err(WorkflowError::external("startup read failed"))
            } else {
                Ok(Vec::new())
            }
        }

        fn load(
            &self,
            _tree_id: &str,
        ) -> Result<Option<crate::domain::workflow::repository::WorkflowStartupRecord>, WorkflowError>
        {
            unreachable!("empty startup inventory")
        }

        fn append(
            &self,
            _root: &crate::domain::workflow::NodeFactMeta,
            _fact: &crate::domain::workflow::NodeFact,
            _timestamp: f64,
        ) -> Result<(), WorkflowError> {
            unreachable!("empty startup inventory")
        }
    }

    #[async_trait::async_trait]
    impl super::super::startup::WorkflowStartupGateway for FakeRuntimeGateway {
        fn current_timestamp(&self) -> f64 {
            100.0
        }
        async fn is_registered_or_reserved(&self, _tree_id: &str) -> bool {
            unreachable!("empty startup inventory")
        }
        async fn reconcile_tree(
            &self,
            _tree_id: &str,
            _timestamp: f64,
        ) -> Result<(), WorkflowError> {
            unreachable!("empty startup inventory")
        }
    }

    fn runtime_with_startup(gateway: Arc<FakeRuntimeGateway>) -> WorkflowRuntimeUsecase {
        WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        )
        .with_startup(Some(Arc::new(
            super::super::startup::WorkflowStartupUsecase::new(gateway.clone(), gateway),
        )))
    }

    #[tokio::test]
    async fn test_起動時復旧_構築時には実行せずusecaseの入口から直接呼び出す() {
        // Given
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = runtime_with_startup(gateway.clone());
        assert!(gateway.calls.lock().unwrap().is_empty());
        // When
        usecase.recover_startup().await.unwrap();
        // Then
        assert_eq!(*gateway.calls.lock().unwrap(), ["startup_list"]);
    }

    #[tokio::test]
    async fn test_起動時復旧_通常起動とstop受理で同じusecaseのエラーを伝播する() {
        for fail_startup in [false, true] {
            // Given
            let gateway = Arc::new(FakeRuntimeGateway {
                fail_startup,
                ..Default::default()
            });
            let usecase = runtime_with_startup(gateway.clone());
            // When
            let startup = usecase.recover_startup().await;
            let stop = usecase
                .record_provider_stop(
                    crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand {
                        tree_id: "tree".into(),
                        node_execution_id: "node".into(),
                        agent_session_id: "session".into(),
                        binding_id: "binding".into(),
                    },
                    Vec::new(),
                )
                .await;
            // Then
            if fail_startup {
                assert!(startup
                    .unwrap_err()
                    .to_string()
                    .contains("startup read failed"));
                assert!(stop
                    .unwrap_err()
                    .to_string()
                    .contains("startup read failed"));
                assert_eq!(
                    *gateway.calls.lock().unwrap(),
                    ["startup_list", "load_active", "startup_list"]
                );
            } else {
                startup.unwrap();
                stop.unwrap();
                assert_eq!(
                    *gateway.calls.lock().unwrap(),
                    ["startup_list", "load_active", "startup_list", "load_active"]
                );
            }
        }
    }

    #[tokio::test]
    async fn test_起動時復旧_永続ストアのない構成は処理を呼ばない() {
        // Given
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );
        // When
        usecase.recover_startup().await.unwrap();
        // Then
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_usecase_delegates_runtime_commands() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );

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
                "state_by_execution"
            ]
        );
    }

    #[tokio::test]
    async fn runtime_usecase_delegates_active_command_shutdown() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );

        usecase.shutdown_active_commands().await;

        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["shutdown_active_commands"]
        );
    }

    #[tokio::test]
    async fn start_execution_rejects_invalid_workflow_name_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );

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
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );

        let abort_err = usecase
            .abort_execution(AbortExecutionCommand {
                execution_id: "not-a-uuid".to_string(),
                expected_node_name: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(abort_err, WorkflowError::Validation(_)));

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
        let usecase = WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );

        let execution_err = usecase
            .get_state_by_execution_id("not-a-uuid")
            .await
            .unwrap_err();
        assert!(matches!(execution_err, WorkflowError::Validation(_)));

        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
