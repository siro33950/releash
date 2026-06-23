use std::sync::Arc;

use crate::domain::workflow::{WorkflowError, WorkflowStateSnapshot};

use super::approval_chat::WorkflowApprovalChatUsecase;
use super::command::{
    AbortRunCommand, ApprovalCommand, ResolvedStartRunCommand, StartRunCommand,
    SubmitOutputCommand, WorkflowAbortRunUsecase, WorkflowApprovalUsecase,
    WorkflowPendingRuntimeCommandUsecase, WorkflowRuntimeCommandPreflight, WorkflowStartRunUsecase,
    WorkflowSubmitOutputUsecase,
};
use super::ports::{
    ApprovalChatTarget, PendingRuntimeCommand, PendingRuntimeCommandOutcome,
    PendingRuntimeCommandPayload, WorkflowAbortRunGateway, WorkflowApprovalChatGateway,
    WorkflowApprovalGateway, WorkflowPendingRuntimeCommandGateway, WorkflowRuntimeCommandGateway,
    WorkflowRuntimeStateGateway, WorkflowStartRunGateway, WorkflowSubmitOutputGateway,
    WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway, WorkflowTurnCompleteNotification,
    WorkflowTurnTokenUsage,
};
use super::turn_complete::WorkflowTurnCompleteUsecase;

#[derive(Clone)]
pub struct WorkflowRuntimeUsecase {
    runtime: Arc<dyn WorkflowRuntimeStateGateway>,
    start_run: WorkflowStartRunUsecase,
    abort_run: WorkflowAbortRunUsecase,
    approval: WorkflowApprovalUsecase,
    submit_output: WorkflowSubmitOutputUsecase,
    pending_command: WorkflowPendingRuntimeCommandUsecase,
    approval_chat: WorkflowApprovalChatUsecase,
    turn_complete: WorkflowTurnCompleteUsecase,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowRuntimeUsecase {
    pub fn new(runtime: Arc<dyn WorkflowRuntimeCommandGateway>) -> Self {
        Self {
            runtime: runtime.clone(),
            start_run: WorkflowStartRunUsecase::new(runtime.clone()),
            abort_run: WorkflowAbortRunUsecase::new(runtime.clone()),
            approval: WorkflowApprovalUsecase::new(runtime.clone()),
            submit_output: WorkflowSubmitOutputUsecase::new(runtime.clone()),
            pending_command: WorkflowPendingRuntimeCommandUsecase::new(runtime.clone()),
            approval_chat: WorkflowApprovalChatUsecase::new(runtime.clone()),
            turn_complete: WorkflowTurnCompleteUsecase::new(runtime),
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub async fn start_run(&self, command: StartRunCommand) -> Result<String, WorkflowError> {
        self.start_run.execute(command).await
    }

    pub async fn abort_run(&self, command: AbortRunCommand) -> Result<(), WorkflowError> {
        self.abort_run.execute(command).await
    }

    pub async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.approval.execute(command).await
    }

    pub async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.submit_output.execute(command).await
    }

    pub async fn dispatch_pending_command(
        &self,
        command: PendingRuntimeCommand,
    ) -> PendingRuntimeCommandOutcome {
        self.pending_command.dispatch(command).await
    }

    pub async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteNotification,
    ) -> Result<(), WorkflowError> {
        self.turn_complete.complete_turn(command).await
    }

    pub async fn is_session_running(&self, chat_session_id: &str) -> bool {
        self.turn_complete.is_session_running(chat_session_id).await
    }

    pub async fn get_state_by_run_id(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        self.preflight.validate_run_lookup(run_id)?;
        self.runtime.get_state_by_run_id(run_id).await
    }

    pub async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        self.preflight.validate_worktree_lookup(worktree_path)?;
        self.runtime.get_state_by_worktree(worktree_path).await
    }

    pub async fn prepare_approval_chat(
        &self,
        run_id: &str,
        content: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError> {
        self.approval_chat
            .prepare_approval_chat(run_id, content)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ApprovalDecision, TriggerSource, WorkflowDefinition};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        session_running: bool,
    }

    #[async_trait::async_trait]
    impl WorkflowStartRunGateway for FakeRuntimeGateway {
        async fn resolve_start_run_worktree(
            &self,
            worktree_path: String,
        ) -> Result<String, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_worktree");
            Ok(worktree_path)
        }

        async fn resolve_start_run_workflow(
            &self,
            _workflow_file_stem: &str,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            self.calls.lock().unwrap().push("resolve_workflow");
            Ok(WorkflowDefinition::default())
        }

        async fn start_resolved_run(
            &self,
            _command: ResolvedStartRunCommand,
        ) -> Result<String, WorkflowError> {
            self.calls.lock().unwrap().push("start");
            Ok("00000000-0000-0000-0000-000000000001".to_string())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowAbortRunGateway for FakeRuntimeGateway {
        async fn abort_run(&self, _command: AbortRunCommand) -> Result<(), WorkflowError> {
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
            self.calls.lock().unwrap().push("submit_output");
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

    #[async_trait::async_trait]
    impl WorkflowRuntimeStateGateway for FakeRuntimeGateway {
        async fn get_state_by_run_id(
            &self,
            _run_id: &str,
        ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
            self.calls.lock().unwrap().push("state_by_run");
            Ok(None)
        }

        async fn get_state_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
            self.calls.lock().unwrap().push("state_by_worktree");
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl WorkflowApprovalChatGateway for FakeRuntimeGateway {
        async fn resolve_approval_chat_target(
            &self,
            _run_id: &str,
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
    async fn runtime_usecase_delegates_runtime_commands() {
        let gateway = Arc::new(FakeRuntimeGateway {
            session_running: true,
            ..Default::default()
        });
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let _ = usecase
            .start_run(StartRunCommand {
                workflow_file_stem: "wf".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                task: None,
                trigger_source: TriggerSource::DesktopUi,
                permission_mode: "ask".to_string(),
            })
            .await
            .unwrap();
        usecase
            .abort_run(AbortRunCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                expected_node_name: None,
            })
            .await
            .unwrap();
        usecase
            .resolve_approval(ApprovalCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: Some("review".to_string()),
                decision: ApprovalDecision::Approve { comment: None },
            })
            .await
            .unwrap();
        usecase
            .submit_output(SubmitOutputCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                step_name: "review".to_string(),
                contract: "review-fix-tasks".to_string(),
                structured_output: serde_json::json!({}),
            })
            .await
            .unwrap();
        let pending = usecase
            .dispatch_pending_command(PendingRuntimeCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                request_id: "00000000-0000-0000-0000-000000000002".to_string(),
                requested_at: 1.0,
                payload: PendingRuntimeCommandPayload::Abort { node_name: None },
            })
            .await;
        assert_eq!(pending, PendingRuntimeCommandOutcome::Accepted);
        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: vec!["ok".to_string()],
                token_usage: Some(WorkflowTurnTokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                }),
                interrupted: false,
            })
            .await
            .unwrap();
        let _ = usecase
            .get_state_by_run_id("00000000-0000-0000-0000-000000000001")
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
                "approval",
                "submit_output",
                "pending",
                "is_running",
                "pickup_pending",
                "complete_turn",
                "state_by_run",
                "state_by_worktree",
                "resolve_approval_chat",
                "validate_approval_chat"
            ]
        );
    }

    #[tokio::test]
    async fn complete_turn_returns_without_pickup_when_session_is_not_running() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        usecase
            .complete_turn(WorkflowTurnCompleteNotification {
                chat_session_id: "chat".to_string(),
                exit_code: 0,
                final_text_parts: Vec::new(),
                token_usage: None,
                interrupted: false,
            })
            .await
            .unwrap();

        assert_eq!(gateway.calls.lock().unwrap().as_slice(), ["is_running"]);
    }

    #[tokio::test]
    async fn start_run_rejects_invalid_workflow_name_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let err = usecase
            .start_run(StartRunCommand {
                workflow_file_stem: "bad name!".to_string(),
                worktree_path: "/tmp/wt".to_string(),
                task: None,
                trigger_source: TriggerSource::DesktopUi,
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
            .abort_run(AbortRunCommand {
                run_id: "not-a-uuid".to_string(),
                expected_node_name: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(abort_err, WorkflowError::Validation(_)));

        let approval_err = usecase
            .resolve_approval(ApprovalCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                node_name: Some(" ".to_string()),
                decision: ApprovalDecision::Approve { comment: None },
            })
            .await
            .unwrap_err();
        assert!(matches!(approval_err, WorkflowError::Validation(_)));

        let submit_err = usecase
            .submit_output(SubmitOutputCommand {
                run_id: "00000000-0000-0000-0000-000000000001".to_string(),
                step_name: "review".to_string(),
                contract: " ".to_string(),
                structured_output: serde_json::json!({}),
            })
            .await
            .unwrap_err();
        assert!(matches!(submit_err, WorkflowError::Validation(_)));

        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_preflight_rejects_invalid_pending_command_without_gateway_dispatch() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let outcome = usecase
            .dispatch_pending_command(PendingRuntimeCommand {
                run_id: "not-a-uuid".to_string(),
                request_id: "00000000-0000-0000-0000-000000000002".to_string(),
                requested_at: 1.0,
                payload: PendingRuntimeCommandPayload::Abort { node_name: None },
            })
            .await;

        assert!(matches!(
            outcome,
            PendingRuntimeCommandOutcome::RejectedFinal(reason)
                if reason.contains("invalid run_id")
        ));
        assert!(gateway.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_preflight_rejects_invalid_queries_before_gateway() {
        let gateway = Arc::new(FakeRuntimeGateway::default());
        let usecase = WorkflowRuntimeUsecase::new(gateway.clone());

        let run_err = usecase.get_state_by_run_id("not-a-uuid").await.unwrap_err();
        assert!(matches!(run_err, WorkflowError::Validation(_)));

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
