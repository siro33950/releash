use std::sync::Arc;

use crate::domain::workflow::{WorkflowError, WorkflowRuntimeSnapshot};

use super::approval_chat::WorkflowApprovalChatUsecase;
#[cfg(test)]
use super::command::ResolvedStartExecutionCommand;
use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeExecutionCommand, StartExecutionCommand,
    StopExecutionCommand, SubmitOutputCommand, WorkflowAbortExecutionUsecase,
    WorkflowApprovalUsecase, WorkflowResumeExecutionUsecase, WorkflowRuntimeCommandPreflight,
    WorkflowStartExecutionUsecase, WorkflowStopExecutionUsecase, WorkflowSubmitOutputUsecase,
};
use super::ports::{
    ApprovalChatTarget, WorkflowRuntimeCommandGateway, WorkflowStallClearedCommand,
    WorkflowStallClearedNotification, WorkflowStallObservedCommand, WorkflowStallObservedGateway,
    WorkflowStallObservedNotification, WorkflowTurnCompleteNotification,
    WorkflowTurnCompleteRecoveryCommand, WorkflowTurnCompleteRecoveryOutcome,
};
#[cfg(test)]
use super::ports::{
    WorkflowAbortExecutionGateway, WorkflowApprovalChatGateway, WorkflowApprovalGateway,
    WorkflowResumeExecutionGateway, WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway,
    WorkflowStartExecutionGateway, WorkflowStartupRecoveryAdmission, WorkflowStopExecutionGateway,
    WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway,
    WorkflowTurnTokenUsage,
};
use super::turn_complete::WorkflowTurnCompleteUsecase;

#[derive(Clone)]
pub struct WorkflowRuntimeUsecase {
    runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
    stall_observed: Arc<dyn WorkflowStallObservedGateway>,
    start_execution: WorkflowStartExecutionUsecase,
    abort_execution: WorkflowAbortExecutionUsecase,
    stop_execution: WorkflowStopExecutionUsecase,
    resume_execution: WorkflowResumeExecutionUsecase,
    approval: WorkflowApprovalUsecase,
    submit_output: WorkflowSubmitOutputUsecase,
    approval_chat: WorkflowApprovalChatUsecase,
    turn_complete: WorkflowTurnCompleteUsecase,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowRuntimeUsecase {
    pub fn new(runtime: Arc<dyn WorkflowRuntimeCommandGateway>) -> Self {
        Self {
            runtime: runtime.clone(),
            stall_observed: runtime.clone(),
            start_execution: WorkflowStartExecutionUsecase::new(runtime.clone()),
            abort_execution: WorkflowAbortExecutionUsecase::new(runtime.clone()),
            stop_execution: WorkflowStopExecutionUsecase::new(runtime.clone()),
            resume_execution: WorkflowResumeExecutionUsecase::new(runtime.clone()),
            approval: WorkflowApprovalUsecase::new(runtime.clone()),
            submit_output: WorkflowSubmitOutputUsecase::new(runtime.clone()),
            approval_chat: WorkflowApprovalChatUsecase::new(runtime.clone()),
            turn_complete: WorkflowTurnCompleteUsecase::new(runtime),
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

    /// Wait for the verified local-store authority and invoke workflow
    /// recovery exactly once. A blocked migration leaves the runtime inert.
    #[cfg(test)]
    pub async fn recover_startup_after_admission(
        &self,
        admission: &dyn WorkflowStartupRecoveryAdmission,
    ) -> Result<bool, WorkflowError> {
        if !self.wait_for_startup_recovery_admission(admission).await {
            return Ok(false);
        }
        self.recover_startup().await?;
        Ok(true)
    }

    /// Waits for verified local-store cutover without starting recovery. The
    /// composition root uses this boundary to replay durable workflow turn
    /// handoffs before orphan interruption can inspect the same executions.
    #[cfg(test)]
    pub async fn wait_for_startup_recovery_admission(
        &self,
        admission: &dyn WorkflowStartupRecoveryAdmission,
    ) -> bool {
        loop {
            if admission.normal_mutation_admitted() {
                return true;
            }
            if admission.migration_blocked() {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
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

    pub async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.approval.execute(command).await
    }

    pub async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.submit_output.execute(command).await
    }

    pub async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteNotification,
    ) -> Result<(), WorkflowError> {
        self.turn_complete.complete_turn(command).await
    }

    pub async fn recover_turn_complete(
        &self,
        command: WorkflowTurnCompleteRecoveryCommand,
    ) -> Result<WorkflowTurnCompleteRecoveryOutcome, WorkflowError> {
        self.turn_complete.recover_turn_complete(command).await
    }

    pub async fn observe_stall(
        &self,
        command: WorkflowStallObservedNotification,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_stall_observed(&command)?;
        self.stall_observed
            .observe_stall(WorkflowStallObservedCommand {
                chat_session_id: command.chat_session_id,
                turn_phase: command.turn_phase,
                idle_secs: command.idle_secs,
                signal_count: command.signal_count,
                cap_reached: command.cap_reached,
            })
            .await
    }

    pub async fn clear_stall(
        &self,
        command: WorkflowStallClearedNotification,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_stall_cleared(&command)?;
        self.stall_observed
            .clear_stall(WorkflowStallClearedCommand {
                chat_session_id: command.chat_session_id,
            })
            .await
    }

    #[allow(dead_code)] // issues-1301 B-3/G-1: retained for workflow node guards around agent turn completion.
    pub async fn is_session_running(&self, chat_session_id: &str) -> bool {
        self.turn_complete.is_session_running(chat_session_id).await
    }

    #[cfg(test)]
    pub async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        self.preflight.validate_execution_lookup(execution_id)?;
        self.runtime.get_state_by_execution_id(execution_id).await
    }

    pub async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        self.preflight.validate_worktree_lookup(worktree_path)?;
        self.runtime.get_state_by_worktree(worktree_path).await
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

    pub async fn prepare_approval_chat(
        &self,
        execution_id: &str,
        content: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError> {
        self.approval_chat
            .prepare_approval_chat(execution_id, content)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        session_running: bool,
    }

    #[derive(Default)]
    struct FakeStartupRecoveryAdmission {
        normal_mutation_admitted: AtomicBool,
        migration_blocked: AtomicBool,
    }

    impl WorkflowStartupRecoveryAdmission for FakeStartupRecoveryAdmission {
        fn normal_mutation_admitted(&self) -> bool {
            self.normal_mutation_admitted.load(Ordering::SeqCst)
        }

        fn migration_blocked(&self) -> bool {
            self.migration_blocked.load(Ordering::SeqCst)
        }
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
            self.calls.lock().unwrap().push("submit_output");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteGateway for FakeRuntimeGateway {
        async fn is_session_running(&self, _chat_session_id: &str) -> bool {
            self.calls.lock().unwrap().push("is_running");
            self.session_running
        }

        async fn complete_turn(
            &self,
            _command: WorkflowTurnCompleteCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("complete_turn");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStallObservedGateway for FakeRuntimeGateway {
        async fn observe_stall(
            &self,
            _command: WorkflowStallObservedCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("observe_stall");
            Ok(())
        }

        async fn clear_stall(
            &self,
            _command: WorkflowStallClearedCommand,
        ) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().push("clear_stall");
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

        async fn get_state_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
            self.calls.lock().unwrap().push("state_by_worktree");
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

    #[async_trait::async_trait]
    impl WorkflowApprovalChatGateway for FakeRuntimeGateway {
        async fn resolve_approval_chat_target(
            &self,
            _execution_id: &str,
        ) -> Result<ApprovalChatTarget, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_approval_chat");
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
            self.calls.lock().unwrap().push("validate_approval_chat");
            Ok(())
        }
    }

    #[tokio::test]
    async fn startup_recovery_waits_for_normal_admission_and_runs_once() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());
        let admission = Arc::new(FakeStartupRecoveryAdmission::default());

        assert!(
            gateway.calls.lock().unwrap().is_empty(),
            "runtime construction must not start recovery"
        );

        let pending_recovery = {
            let usecase = usecase.clone();
            let admission = admission.clone();
            tokio::spawn(async move {
                usecase
                    .recover_startup_after_admission(admission.as_ref())
                    .await
            })
        };
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert!(!pending_recovery.is_finished());
        assert!(gateway.calls.lock().unwrap().is_empty());

        admission
            .normal_mutation_admitted
            .store(true, Ordering::SeqCst);
        let recovered = tokio::time::timeout(std::time::Duration::from_secs(1), pending_recovery)
            .await
            .expect("recovery should observe normal admission")
            .expect("recovery task should not panic")
            .expect("recovery should succeed");

        assert!(recovered);
        assert_eq!(
            gateway.calls.lock().unwrap().as_slice(),
            ["recover_startup"]
        );
    }

    #[tokio::test]
    async fn startup_recovery_stays_inert_when_migration_is_blocked() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());
        let admission = FakeStartupRecoveryAdmission::default();
        admission.migration_blocked.store(true, Ordering::SeqCst);

        assert!(!usecase
            .recover_startup_after_admission(&admission)
            .await
            .unwrap());
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_usecase_delegates_runtime_commands() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let _ = usecase
            .start_execution(StartExecutionCommand {
                workflow_name: "wf".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                request: None,
                created_from: ExecutionOrigin::DesktopUi,
                permission_mode: "ask".to_string(),
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
        usecase
            .resolve_approval(ApprovalCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-1".to_string()),
                comment: None,
            })
            .await
            .unwrap();
        usecase
            .submit_output(SubmitOutputCommand {
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-1".to_string()),
                contract: "review-fix-tasks".to_string(),
                artifact: serde_json::json!({}),
            })
            .await
            .unwrap();
        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: vec!["ok".to_string()],
                failure_signal: None,
                token_usage: Some(WorkflowTurnTokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                }),
                interrupted: false,
            })
            .await
            .unwrap();
        let _ = usecase
            .get_state_by_execution_id("00000000-0000-0000-0000-000000000001")
            .await
            .unwrap();
        let _ = usecase.get_state_by_worktree("/tmp/wt").await.unwrap();
        let _ = usecase
            .prepare_approval_chat("00000000-0000-0000-0000-000000000001", "ok")
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
                "submit_output",
                "is_running",
                "complete_turn",
                "state_by_execution",
                "state_by_worktree",
                "resolve_approval_chat",
                "validate_approval_chat"
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
    async fn complete_turn_returns_when_session_is_not_running() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

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
    async fn observe_stall_validates_and_delegates_to_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        usecase
            .observe_stall(WorkflowStallObservedNotification {
                chat_session_id: "chat".to_string(),
                turn_phase: "streaming".to_string(),
                idle_secs: 44,
                signal_count: 1,
                cap_reached: false,
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["observe_stall"]);
    }

    #[tokio::test]
    async fn observe_stall_rejects_empty_session_id() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let err = usecase
            .observe_stall(WorkflowStallObservedNotification {
                chat_session_id: " ".to_string(),
                turn_phase: "streaming".to_string(),
                idle_secs: 44,
                signal_count: 1,
                cap_reached: false,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_stall_validates_and_delegates_to_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        usecase
            .clear_stall(WorkflowStallClearedNotification {
                chat_session_id: "chat".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["clear_stall"]);
    }

    #[tokio::test]
    async fn clear_stall_rejects_empty_session_id() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let err = usecase
            .clear_stall(WorkflowStallClearedNotification {
                chat_session_id: " ".to_string(),
            })
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
        assert!(gateway.calls.lock().unwrap().is_empty());
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
                permission_mode: "ask".to_string(),
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
                execution_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: "review".to_string(),
                node_execution_id: None,
                contract: " ".to_string(),
                artifact: serde_json::json!({}),
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

        let worktree_err = usecase.get_state_by_worktree(" ").await.unwrap_err();
        assert!(matches!(worktree_err, WorkflowError::Validation(_)));

        let chat_err = usecase
            .prepare_approval_chat("00000000-0000-0000-0000-000000000001", " ")
            .await
            .unwrap_err();
        assert!(matches!(chat_err, WorkflowError::Validation(_)));

        assert!(gateway.calls.lock().unwrap().is_empty());
    }
}
