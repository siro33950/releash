use super::*;

pub(crate) mod tests {
    #[test]
    fn test_session名前変更_所有済みと保存競合を区別して返す() {
        // Given / When / Then
        for (source, expected) in [
            (super::AgentSessionRenameError::Conflict, true),
            (
                super::AgentSessionRenameError::ProviderSessionAlreadyOwned,
                false,
            ),
        ] {
            let error = super::map_session_rename_error(source);
            assert_eq!(
                matches!(error, crate::domain::workflow::WorkflowError::Conflict(_)),
                expected
            );
            assert!(matches!(
                error,
                crate::domain::workflow::WorkflowError::Conflict(_)
                    | crate::domain::workflow::WorkflowError::InvalidState(_)
            ));
            assert!(!error.to_string().contains("Store failure"));
        }
    }

    use std::sync::Mutex;

    use super::super::*;

    enum FakeActionResult<T> {
        Value(T),
        Unavailable,
    }

    struct FakeWorkspaceNodeActionResolver {
        approval: FakeActionResult<WorkspaceNodeApprovalTarget>,
        retry: FakeActionResult<WorkspaceNodeRetryTarget>,
        rename: FakeActionResult<WorkspaceSessionNodeRenameTarget>,
        requests: Mutex<Vec<(String, String, String)>>,
    }

    impl FakeWorkspaceNodeActionResolver {
        fn approval(target: WorkspaceNodeApprovalTarget) -> Self {
            Self {
                approval: FakeActionResult::Value(target),
                retry: FakeActionResult::Unavailable,
                rename: FakeActionResult::Unavailable,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn retry(target: WorkspaceNodeRetryTarget) -> Self {
            Self {
                approval: FakeActionResult::Unavailable,
                retry: FakeActionResult::Value(target),
                rename: FakeActionResult::Unavailable,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn rename(target: WorkspaceSessionNodeRenameTarget) -> Self {
            Self {
                approval: FakeActionResult::Unavailable,
                retry: FakeActionResult::Unavailable,
                rename: FakeActionResult::Value(target),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl WorkspaceNodeActionResolver for FakeWorkspaceNodeActionResolver {
        async fn resolve_session_resume_target(
            &self,
            worktree: &str,
            node: &str,
        ) -> Result<ResumeSessionNodeCommand, WorkflowError> {
            let target = self.resolve_retry_target(worktree, node).await?;
            Ok(ResumeSessionNodeCommand {
                execution_id: target.execution_id,
                node_execution_id: target.node_execution_id,
            })
        }

        async fn resolve_approval_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "approval".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.approval {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("approval unavailable"))
                }
            }
        }

        async fn resolve_retry_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "retry".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.retry {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("retry unavailable"))
                }
            }
        }

        async fn resolve_session_rename_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "rename".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.rename {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("rename unavailable"))
                }
            }
        }
    }

    #[derive(Default)]
    struct FakeWorkspaceNodeWorkflowGateway {
        approvals: Mutex<Vec<ApprovalCommand>>,
        retries: Mutex<Vec<RetryNodeCommand>>,
        resumes: Mutex<Vec<ResumeSessionNodeCommand>>,
    }

    #[async_trait::async_trait]
    impl WorkspaceNodeWorkflowCommandExecutor for FakeWorkspaceNodeWorkflowGateway {
        async fn resume_session_node(
            &self,
            command: ResumeSessionNodeCommand,
        ) -> Result<(), WorkflowError> {
            self.resumes.lock().unwrap().push(command);
            Ok(())
        }

        async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
            self.approvals.lock().unwrap().push(command);
            Ok(())
        }

        async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
            self.retries.lock().unwrap().push(command);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeAgentSessionRenameExecutor {
        requests: Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl AgentSessionRenameExecutor for FakeAgentSessionRenameExecutor {
        async fn rename(
            &self,
            agent_session_id: &str,
            name: &str,
        ) -> Result<
            crate::domain::agent_session::aggregates::AgentSessionMutationOutcome,
            AgentSessionRenameError,
        > {
            self.requests
                .lock()
                .unwrap()
                .push((agent_session_id.to_string(), name.to_string()));
            Ok(crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied)
        }
    }

    #[tokio::test]
    async fn approve_workspace_node_resolves_target_and_executes_one_workflow_command() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::approval(
            WorkspaceNodeApprovalTarget {
                execution_id: "execution-1".to_string(),
                node_name: "review".to_string(),
                node_execution_id: "node-execution-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames);

        usecase
            .approve_workspace_node(ApproveWorkspaceNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "approval".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert_eq!(workflows.approvals.lock().unwrap().len(), 1);
        assert!(workflows.retries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn retry_workspace_node_resolves_target_and_executes_one_workflow_command() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::retry(
            WorkspaceNodeRetryTarget {
                execution_id: "execution-1".to_string(),
                node_execution_id: "node-execution-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames);

        usecase
            .retry_workspace_node(RetryWorkspaceNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "retry".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert!(workflows.approvals.lock().unwrap().is_empty());
        assert_eq!(workflows.retries.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn resume_workspace_node_resolves_target_and_executes_one_workflow_command() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::retry(
            WorkspaceNodeRetryTarget {
                execution_id: "execution-1".to_string(),
                node_execution_id: "node-execution-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames);

        usecase
            .resume_workspace_session_node(ResumeWorkspaceSessionNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "retry".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert!(workflows.approvals.lock().unwrap().is_empty());
        assert!(workflows.retries.lock().unwrap().is_empty());
        assert_eq!(
            *workflows.resumes.lock().unwrap(),
            vec![ResumeSessionNodeCommand {
                execution_id: "execution-1".into(),
                node_execution_id: "node-execution-1".into()
            }]
        );
    }

    #[tokio::test]
    async fn rename_workspace_session_node_resolves_target_and_delegates_name() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::rename(
            WorkspaceSessionNodeRenameTarget {
                agent_session_id: "agent-session-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames.clone());

        usecase
            .rename_workspace_session_node(RenameWorkspaceSessionNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
                name: "release review".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "rename".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert_eq!(
            *renames.requests.lock().unwrap(),
            vec![("agent-session-1".to_string(), "release review".to_string())]
        );
        assert!(workflows.approvals.lock().unwrap().is_empty());
        assert!(workflows.retries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rename_workspace_session_node_rejects_unresolvable_node_without_delegation() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::approval(
            WorkspaceNodeApprovalTarget {
                execution_id: "execution-1".to_string(),
                node_name: "sequence".to_string(),
                node_execution_id: "sequence-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver, workflows, renames.clone());

        let result = usecase
            .rename_workspace_session_node(RenameWorkspaceSessionNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "non-renameable-node".to_string(),
                name: "release review".to_string(),
            })
            .await;

        assert!(matches!(result, Err(WorkflowError::InvalidState(_))));
        assert!(renames.requests.lock().unwrap().is_empty());
    }
}
