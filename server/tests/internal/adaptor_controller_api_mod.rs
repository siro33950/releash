pub(crate) mod test_support {

    use std::path::Path;
    use std::sync::Arc;
    use std::sync::Mutex;

    use releashd::test_support::integration::workflow::AbortExecutionCommand;
    use releashd::test_support::integration::workflow::ApprovalCommand;
    use releashd::test_support::integration::workflow::ResolvedStartExecutionCommand;
    use releashd::test_support::integration::workflow::RetryNodeCommand;
    use releashd::test_support::integration::workflow::SubmitOutputArtifact;
    use releashd::test_support::integration::workflow::SubmitOutputCommand;
    use releashd::test_support::integration::workflow::WorkflowAbortExecutionGateway;
    use releashd::test_support::integration::workflow::WorkflowControlPlaneCommit;
    use releashd::test_support::integration::workflow::WorkflowControlPlaneGateway;
    use releashd::test_support::integration::workflow::WorkflowDefinition;
    use releashd::test_support::integration::workflow::WorkflowError;
    use releashd::test_support::integration::workflow::WorkflowRuntimeShutdownGateway;
    use releashd::test_support::integration::workflow::WorkflowRuntimeStateGateway;
    use releashd::test_support::integration::workflow::WorkflowStartExecutionGateway;

    use axum::Router;
    use releashd::test_support::integration::transport::build_router;
    use releashd::test_support::integration::transport::ClientApiDeps;
    use releashd::test_support::integration::workflow::WorkflowRuntimeUsecase;

    #[derive(Default)]
    pub(crate) struct RecordedRuntimeCommands {
        pub(crate) starts: Vec<ResolvedStartExecutionCommand>,
        pub(crate) approvals: Vec<ApprovalCommand>,
        pub(crate) aborts: Vec<AbortExecutionCommand>,
        pub(crate) retries: Vec<RetryNodeCommand>,
        pub(crate) outputs: Vec<SubmitOutputCommand>,
    }

    #[derive(Default)]
    pub(crate) struct RecordedRuntimeErrors {
        pub(crate) start: Option<WorkflowError>,
        pub(crate) abort: Option<WorkflowError>,
        approval: Option<WorkflowError>,
        output: Option<WorkflowError>,
    }

    #[derive(Default)]
    pub(crate) struct RecordingRuntimeGateway {
        pub(crate) commands: Mutex<RecordedRuntimeCommands>,
        pub(crate) errors: Mutex<RecordedRuntimeErrors>,
    }

    #[async_trait::async_trait]
    impl WorkflowStartExecutionGateway for RecordingRuntimeGateway {
        async fn resolve_start_execution_worktree(
            &self,
            worktree_path: String,
        ) -> Result<String, WorkflowError> {
            Ok(worktree_path)
        }

        async fn resolve_start_execution_workflow(
            &self,
            _workflow_name: &str,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            Ok(WorkflowDefinition::default())
        }

        async fn start_resolved_execution(
            &self,
            command: ResolvedStartExecutionCommand,
        ) -> Result<String, WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().start.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().starts.push(command);
            Ok("00000000-0000-4000-8000-000000000001".to_string())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowAbortExecutionGateway for RecordingRuntimeGateway {
        async fn abort_execution(
            &self,
            command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().abort.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().aborts.push(command);
            Ok(())
        }
    }

    fn control_plane_execution_fixture(
        execution_id: &str,
    ) -> releashd::test_support::integration::workflow::workflow_execution_ExecutionTree {
        use std::collections::BTreeMap;
        use std::collections::BTreeSet;

        use releashd::test_support::integration::workflow::workflow_execution_ExecutionTree as ExecutionTree;
        use releashd::test_support::integration::workflow::ExecutionTreeRestore;
        use releashd::test_support::integration::workflow::FanoutSpec;
        use releashd::test_support::integration::workflow::NodeCompletion;
        use releashd::test_support::integration::workflow::NodeCompletionSignal;
        use releashd::test_support::integration::workflow::NodeDefinition as DomainNode;
        use releashd::test_support::integration::workflow::NodeKind as DomainNodeKind;
        use releashd::test_support::integration::workflow::SchemaDef;
        use releashd::test_support::integration::workflow::SessionSpec;
        use releashd::test_support::integration::workflow::{ExecutionParentRef, NodeKindName};

        let workflow = WorkflowDefinition {
            name: "review-workflow".to_string(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::from([(
                "review-result".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "status".to_string(),
                        SchemaDef::String { r#enum: None },
                    )]),
                    required: BTreeSet::from(["status".to_string()]),
                },
            )]),
            nodes: vec![
                DomainNode {
                    name: "command".into(),
                    kind: DomainNodeKind::Command(
                        releashd::test_support::integration::workflow::CommandSpec {
                            command: "true".into(),
                            env: Default::default(),
                        },
                    ),
                    ..Default::default()
                },
                DomainNode {
                    name: "fanout".to_string(),
                    kind: DomainNodeKind::Fanout(FanoutSpec {
                        children: vec![
                            releashd::test_support::integration::workflow::ChildEntry::reference(
                                "review",
                            ),
                            releashd::test_support::integration::workflow::ChildEntry::reference(
                                "command",
                            ),
                        ],
                        items: None,
                    }),
                    ..Default::default()
                },
                DomainNode {
                    name: "review".to_string(),
                    kind: DomainNodeKind::Session(SessionSpec::default()),
                    artifact: Some("review-result".to_string()),
                    completion: NodeCompletion::require_approval(),
                    ..Default::default()
                },
            ],
            entry: "fanout".to_string(),
        };
        let mut execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: execution_id.to_string(),
            workflow,
            worktree_path: "/repo".to_string(),
            started_at: 100.0,
            updated_at: 100.0,
            ..ExecutionTreeRestore::default()
        });
        execution
            .replay_node_started(
                "fanout-parent",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                100.0,
            )
            .unwrap();
        for (attempt, node_execution_id) in [
            (1, "ne-review-1"),
            (2, "ne-review-2"),
            (3, "00000000-0000-4000-8000-000000000456"),
        ] {
            execution
                .replay_node_started(
                    node_execution_id,
                    "review",
                    NodeKindName::Session,
                    attempt,
                    Some(ExecutionParentRef::fanout_child(
                        "fanout-parent",
                        Some(attempt as usize - 1),
                        0,
                    )),
                    100.0 + f64::from(attempt),
                )
                .unwrap();
            execution.record_node_completion_signal(
                node_execution_id,
                NodeCompletionSignal::Stop,
                103.0 + f64::from(attempt),
            );
            if node_execution_id == "ne-review-1" {
                execution.record_node_completion_signal(
                    node_execution_id,
                    NodeCompletionSignal::Submit,
                    106.0,
                );
                execution.mark_node_waiting_approval(node_execution_id, 107.0);
            }
        }
        execution
            .replay_node_started(
                "ne-command-1",
                "command",
                NodeKindName::Command,
                1,
                Some(ExecutionParentRef::fanout_child("fanout-parent", None, 1)),
                100.0,
            )
            .unwrap();
        execution
    }

    #[async_trait::async_trait]
    impl WorkflowControlPlaneGateway for RecordingRuntimeGateway {
        fn node_process_presence(
            &self,
            _execution: &releashd::test_support::integration::workflow::workflow_execution_ExecutionTree,
            _id: &str,
        ) -> Result<
            releashd::test_support::integration::workflow::NodeProcessPresence,
            releashd::test_support::integration::workflow::WorkflowError,
        > {
            Ok(releashd::test_support::integration::workflow::NodeProcessPresence::ConfirmedAbsent)
        }
        fn worktree_exists(
            &self,
            _path: &str,
        ) -> Result<bool, releashd::test_support::integration::workflow::WorkflowError> {
            Ok(true)
        }
        async fn session_conversation_exists(
            &self,
            _session_id: &str,
        ) -> Result<bool, releashd::test_support::integration::workflow::WorkflowError> {
            Ok(true)
        }
        async fn resume_session_process(
            &self,
            _execution_id: &str,
            _node_id: &str,
            _session_id: &str,
        ) -> Result<(), releashd::test_support::integration::workflow::WorkflowError> {
            Ok(())
        }

        fn current_timestamp(&self) -> f64 {
            110.0
        }

        fn new_node_execution_id(&self) -> String {
            "node-execution-next".to_string()
        }

        async fn resolve_workflow_execution_id(
            &self,
            _node_execution_id: &str,
        ) -> Result<Option<String>, WorkflowError> {
            Ok(None)
        }

        async fn load_active_execution(
            &self,
            execution_id: &str,
        ) -> Result<
            Option<releashd::test_support::integration::workflow::workflow_execution_ExecutionTree>,
            WorkflowError,
        > {
            if let Some(error) = self.errors.lock().unwrap().approval.clone() {
                return Err(error);
            }
            if let Some(error) = self.errors.lock().unwrap().output.clone() {
                return Err(error);
            }
            Ok(Some(control_plane_execution_fixture(execution_id)))
        }

        async fn register_started_execution_tree(
            &self,
            _tree_id: &str,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        async fn approval_persisted(
            &self,
            _execution_id: &str,
            _node_name: &str,
            _node_execution_id: Option<&str>,
        ) -> Result<bool, WorkflowError> {
            Ok(false)
        }

        fn configured_secret_values(&self) -> Vec<String> {
            Vec::new()
        }

        async fn commit_control_plane(
            &self,
            commit: WorkflowControlPlaneCommit,
        ) -> Result<
            releashd::test_support::integration::workflow::RuntimeCommitSnapshot,
            WorkflowError,
        > {
            if let Some(command) = commit.workflow_events.iter().find_map(|event| {
                match event {
                releashd::test_support::integration::workflow::WorkflowEvent::ApprovalResolved {
                    execution_id,
                    node_execution_id,
                    node_name,
                    comment,
                    ..
                } => Some(ApprovalCommand {
                    execution_id: execution_id.clone(),
                    node_name: node_name.clone(),
                    node_execution_id: Some(node_execution_id.clone()),
                    comment: comment.clone(),
                }),
                _ => None,
            }
            }) {
                self.commands.lock().unwrap().approvals.push(command);
            }
            let retry = commit.workflow_events.iter().find_map(|event| {
                match event {
                releashd::test_support::integration::workflow::WorkflowEvent::NodeRetryRequested {
                    execution_id,
                    node_execution_id,
                    ..
                } => Some(RetryNodeCommand {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                }),
                _ => None,
            }
            });
            if let Some(command) = retry {
                self.commands.lock().unwrap().retries.push(command);
            }
            let submit = commit.workflow_events.iter().find_map(|event| match event {
                releashd::test_support::integration::workflow::WorkflowEvent::NodeSubmitReceived {
                    execution_id,
                    node_execution_id,
                    ..
                } => {
                    let node_name = commit
                        .after
                        .node_executions
                        .iter()
                        .find(|node| node.id == *node_execution_id)
                        .map(|node| node.node_name.clone())?;
                    let artifact = commit.workflow_events.iter().find_map(|event| match event {
                        releashd::test_support::integration::workflow::WorkflowEvent::ArtifactProduced {
                            node_execution_id: artifact_node_execution_id,
                            contract: Some(contract),
                            value,
                            ..
                        } if artifact_node_execution_id == node_execution_id => {
                            Some(SubmitOutputArtifact {
                                contract: contract.clone(),
                                value: value.clone(),
                            })
                        }
                        _ => None,
                    });
                    Some((
                        SubmitOutputCommand {
                            node_execution_id: node_execution_id.clone(),
                            artifact,
                        },
                        execution_id.clone(),
                        node_name,
                    ))
                }
                _ => None,
            });
            if let Some((command, _, _)) = submit {
                self.commands.lock().unwrap().outputs.push(command);
            }
            releashd::test_support::integration::workflow::RuntimeCommitSnapshot::from_execution(
                &commit.after,
            )
            .map_err(|error| WorkflowError::external(error.to_string()))
        }

        async fn finish_control_plane_commit(
            &self,
            _worktree_path: &str,
            _snapshot: &releashd::test_support::integration::workflow::RuntimeCommitSnapshot,
            _outcome: Option<releashd::test_support::integration::workflow::NodeOutcome>,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl releashd::test_support::integration::workflow::ExecutionTreeProcessGateway
        for RecordingRuntimeGateway
    {
        async fn stop_execution_tree_processes(&self, _: &str) -> Result<(), WorkflowError> {
            unreachable!("process cleanup is not used by this fixture")
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRuntimeStateGateway for RecordingRuntimeGateway {}

    #[async_trait::async_trait]
    impl WorkflowRuntimeShutdownGateway for RecordingRuntimeGateway {
        async fn shutdown_active_commands(&self) {}
    }

    pub(crate) fn test_router_with_optional_deps(
        _data_dir: &Path,
        terminal_token: &str,
        client: Option<ClientApiDeps>,
    ) -> (
        Router,
        Arc<WorkflowRuntimeUsecase>,
        Arc<RecordingRuntimeGateway>,
    ) {
        let gateway = Arc::new(RecordingRuntimeGateway::default());
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(releashd::test_support::integration::workflow::NoopArchiveRepository),
        ));
        let router = build_router(
            releashd::test_support::integration::transport::ClientTokens {
                operator: Arc::<str>::from(terminal_token).into(),
                hook: Arc::<str>::from("hook").into(),
            },
            client,
            releashd::test_support::integration::daemon::default_timeout(),
        );
        (router, runtime, gateway)
    }
}
