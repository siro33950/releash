pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition};
    use std::sync::Mutex;

    mod runtime_command_tests {
        use super::*;
        use crate::domain::failure::{
            StorageFailure, StorageFailureSource, TechnicalFailureNature,
        };
        use crate::usecase::agent_session::{
            ExecutionTreeCache, ExecutionTreeCacheReleaseError, StartedExecutionTreeRegistrar,
            StartedExecutionTreeRegistrationError,
        };
        use crate::usecase::provider_lifecycle::{
            ProviderExecutionTreeStopCommand, ProviderExecutionTreeStopTransaction,
        };

        #[tokio::test]
        async fn test_workflow失敗_stopとcache解放と起動登録で元の分類を保持する() {
            // Given
            for error in [
                WorkflowError::Technical(
                    crate::common::operation_context::OperationStopped::Expired.into(),
                ),
                WorkflowError::Technical(
                    crate::common::operation_context::OperationStopped::Cancelled.into(),
                ),
                WorkflowError::External("internal".into()),
                WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                    "launch".into(),
                )),
                WorkflowError::Store(
                    crate::domain::failure::StorageFailure::from(
                        crate::domain::local_event::CommitBatchError::QueueBusy,
                    )
                    .with_message("busy"),
                ),
                WorkflowError::Store(
                    crate::domain::failure::StorageFailure::from(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "failure".into(),
                        },
                    )
                    .with_message("expired"),
                ),
                WorkflowError::Store(
                    crate::domain::failure::StorageFailure::from(
                        crate::domain::local_event::CommitBatchError::PayloadConflict,
                    )
                    .with_message("repair"),
                ),
            ] {
                let expected = match &error {
                    WorkflowError::Store(failure) => failure.clone(),
                    error => StorageFailure {
                        nature: match error {
                            WorkflowError::Technical(failure) => failure.nature,
                            _ => TechnicalFailureNature::Other,
                        },
                        source: StorageFailureSource::Workflow(Box::new(error.clone())),
                        context: None,
                    },
                };
                let gateway = Arc::new(FakeRuntimeGateway {
                    failure: Some(error),
                    ..Default::default()
                });
                let usecase = WorkflowRuntimeUsecase::new(
                    gateway,
                    Arc::new(crate::usecase::workflow::NoopArchiveRepository),
                );
                // When / Then
                let stop = ProviderExecutionTreeStopTransaction::commit_provider_stop(
                    &usecase,
                    ProviderExecutionTreeStopCommand {
                        tree_id: "tree".into(),
                        node_execution_id: "node".into(),
                        agent_session_id: "session".into(),
                        binding_id: "binding".into(),
                    },
                    Vec::new(),
                );
                if expected.nature == TechnicalFailureNature::Transient {
                    assert!(
                        tokio::time::timeout(std::time::Duration::from_millis(100), stop)
                            .await
                            .is_err()
                    );
                } else {
                    assert_eq!(
                stop.await.unwrap_err(),
                crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                    expected.clone()
                )
            );
                }
                assert_eq!(
                    match ExecutionTreeCache::release_deleted_execution_tree(&usecase, "tree")
                        .await
                        .unwrap_err()
                    {
                        ExecutionTreeCacheReleaseError::Store(failure) => failure,
                        error => panic!("unexpected error: {error:?}"),
                    },
                    expected
                );
                assert_eq!(
                    match StartedExecutionTreeRegistrar::register_started_execution_tree(
                        &usecase, "tree"
                    )
                    .await
                    .unwrap_err()
                    {
                        StartedExecutionTreeRegistrationError::Store(failure) => failure,
                        error => panic!("unexpected error: {error:?}"),
                    },
                    expected
                );
            }
        }
    }

    #[derive(Default)]
    struct FakeRuntimeGateway {
        calls: Mutex<Vec<&'static str>>,
        failure: Option<WorkflowError>,
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
            if let Some(error) = &self.failure {
                return Err(error.clone());
            }
            Ok(None)
        }

        async fn register_started_execution_tree(
            &self,
            _tree_id: &str,
        ) -> Result<(), WorkflowError> {
            if let Some(error) = &self.failure {
                return Err(error.clone());
            }
            Err(WorkflowError::external(
                "control plane is not used by this test",
            ))
        }

        async fn release_deleted_execution_tree(&self, _: &str) -> Result<(), WorkflowError> {
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
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
        #[cfg(test)]
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
