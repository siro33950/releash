use super::*;

fn round_trip(fact: NodeFact) -> NodeFact {
    let event_type = fact.event_type();
    let detail = fact.encode_detail().unwrap();
    NodeFact::decode(event_type, &detail).unwrap()
}

mod vocabulary_tests {
    use super::*;

    #[test]
    fn test_事実語彙_event_typeが17種の固定文字列である() {
        // Given: 全17 variant
        let facts: Vec<NodeFact> = vec![
            NodeFact::Started(StartedFact {
                parent: None,
                root: None,
            }),
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "s-1".to_string(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            }),
            NodeFact::CommandSpawned(CommandSpawnedFact {
                display_command: "true".to_string(),
            }),
            NodeFact::ProcessExited(ProcessExitedFact {
                exit_code: Some(0),
                result_summary: None,
                failure_reason: None,
                failure_kind: None,
            }),
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::SubmitRejected(SubmitRejectedFact {
                violations: vec![],
                repair_attempt: 1,
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: None,
                value: serde_json::json!(null),
                request_id: None,
            }),
            NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
            NodeFact::RetryRequested,
            NodeFact::ResumeRequested,
            NodeFact::AbortRequested,
            NodeFact::ArchiveRequested,
            NodeFact::RestoreRequested,
            NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/repo".to_string(),
                worktree_path: "/repo-worktrees/.releash-isolated/node-a1".to_string(),
                branch: "releash/isolated/node-a1".to_string(),
            }),
            NodeFact::IsolatedWorktreeReleased,
            NodeFact::IsolatedWorktreeLost,
        ];

        // When / Then: event_type が確定済み語彙と一致する
        assert_eq!(
            facts.iter().map(NodeFact::event_type).collect::<Vec<_>>(),
            vec![
                "started",
                "session_attached",
                "command_spawned",
                "process_exited",
                "submit_received",
                "submit_rejected",
                "stop_received",
                "artifact_produced",
                "approval_granted",
                "retry_requested",
                "resume_requested",
                "abort_requested",
                "archive_requested",
                "restore_requested",
                "isolated_worktree_created",
                "isolated_worktree_released",
                "isolated_worktree_lost",
            ]
        );

        // Then: 全 variant が encode → decode で往復する
        for fact in facts {
            assert_eq!(round_trip(fact.clone()), fact);
        }
    }

    #[test]
    fn test_事実語彙_未知のevent_typeは拒否する() {
        // 遷移イベントは語彙に存在しない
        assert_eq!(
            NodeFact::decode("node_completed", "{}"),
            Err(NodeFactDecodeError::UnknownEventType(
                "node_completed".to_string()
            ))
        );
    }

    #[test]
    fn test_session_attachedの必須session_id欠落はdetail不一致になる() {
        let error = NodeFact::decode("session_attached", "{}").expect_err("session_id is required");
        assert!(matches!(
            error,
            NodeFactDecodeError::DetailMismatch { event_type, .. }
                if event_type == "session_attached"
        ));
    }

    #[test]
    fn test_隔離worktree生成のdetail_field名を固定する() {
        let detail = NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
            repository_root: "/repo".to_string(),
            worktree_path: "/repo-worktrees/.releash-isolated/node-a1".to_string(),
            branch: "releash/isolated/node-a1".to_string(),
        })
        .encode_detail()
        .unwrap();

        let stored: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(stored["repositoryRoot"], "/repo");
        assert_eq!(
            stored["worktreePath"],
            "/repo-worktrees/.releash-isolated/node-a1"
        );
        assert_eq!(stored["branch"], "releash/isolated/node-a1");

        let error = NodeFact::decode("isolated_worktree_created", r#"{"repositoryRoot":"/repo"}"#)
            .expect_err("worktreePath and branch are required");
        assert!(matches!(
            error,
            NodeFactDecodeError::DetailMismatch { event_type, .. }
                if event_type == "isolated_worktree_created"
        ));
    }

    #[test]
    fn test_payloadなし事実のdetailはjson_object以外を拒否する() {
        for event_type in [
            "retry_requested",
            "resume_requested",
            "abort_requested",
            "archive_requested",
            "restore_requested",
            "isolated_worktree_released",
            "isolated_worktree_lost",
        ] {
            assert!(NodeFact::decode(event_type, "{}").is_ok());
            let error = NodeFact::decode(event_type, "not-json")
                .expect_err("a corrupted detail must not decode");
            assert!(matches!(
                error,
                NodeFactDecodeError::DetailMismatch { event_type: actual, .. }
                    if actual == event_type
            ));
        }
    }

    #[test]
    fn test_既知event_typeの追加fieldは前方互換のため無視する() {
        let decoded = NodeFact::decode(
            "command_spawned",
            r#"{"displayCommand":"true","futureField":1}"#,
        )
        .unwrap();
        assert_eq!(
            decoded,
            NodeFact::CommandSpawned(CommandSpawnedFact {
                display_command: "true".to_string(),
            })
        );
    }
}

mod detail_round_trip_tests {
    use super::*;
    use crate::domain::workflow::{NodeDefinition, WorkflowDefinition};

    #[test]
    fn test_rootのstarted_workflow構成が定義snapshotごと往復する() {
        // Given: workflow 木の root started
        let fact = NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact::Workflow(WorkflowRootFact {
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "please review".to_string(),
                definition: WorkflowDefinition {
                    name: "review".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![NodeDefinition {
                        name: "main".to_string(),
                        ..NodeDefinition::default()
                    }],
                    entry: "main".to_string(),
                },
            })),
        });

        // When / Then: 往復で同値
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_rootのstarted_単独session構成が往復する() {
        let fact = NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact::Session(SessionRootFact {
                workspace_identity: "/repo".to_string(),
                worktree_path: "/repo".to_string(),
                session: crate::domain::workflow::SessionSpec::default(),
                created_from: ExecutionOrigin::DesktopUi,
            })),
        });
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_子のstarted_fanout座標つき親参照が往復する() {
        let fact = NodeFact::Started(StartedFact {
            parent: Some(ExecutionParentRef::fanout_child("parent-exec", Some(2), 1)),
            root: None,
        });
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_process_exited_喪失と失敗分類が往復する() {
        let fact = NodeFact::ProcessExited(ProcessExitedFact {
            exit_code: None,
            result_summary: None,
            failure_reason: Some("provider process lost".to_string()),
            failure_kind: Some(NodeExecutionFailureKind::InfrastructureCrash),
        });
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_stop_received_結果summaryとtoken_usageが往復する() {
        let fact = NodeFact::StopReceived(StopReceivedFact {
            result_summary: Some("done".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 20,
            }),
        });
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_submit_rejected_違反内容が往復する() {
        let fact = NodeFact::SubmitRejected(SubmitRejectedFact {
            violations: vec![ContractViolationRecord {
                path: "$.result".to_string(),
                reason: "missing".to_string(),
            }],
            repair_attempt: 2,
            request_id: Some("req-1".to_string()),
        });
        assert_eq!(round_trip(fact.clone()), fact);
    }
}
