use super::*;

fn round_trip(fact: NodeFact) -> NodeFact {
    let event_type = fact.event_type();
    let detail = fact.encode_detail().unwrap();
    NodeFact::decode(event_type, &detail).unwrap()
}

mod vocabulary_tests {
    use super::*;

    #[test]
    fn test_事実語彙_event_typeが19種の固定文字列である() {
        // Given: 全19 variant
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
            NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                reason: "activation failed".to_string(),
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            }),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::Working,
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
                "runtime_failure_observed",
                "agent_activity_observed",
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
    fn test_活動観測_活動状態だけをdetailへ保存して往復する() {
        for activity in [
            AgentSessionActivity::Working,
            AgentSessionActivity::AwaitingAnswer,
            AgentSessionActivity::AwaitingInstruction,
        ] {
            let fact = NodeFact::AgentActivityObserved(AgentActivityObservedFact { activity });

            let detail = fact.encode_detail().unwrap();
            let stored: serde_json::Value = serde_json::from_str(&detail).unwrap();

            assert_eq!(fact.event_type(), "agent_activity_observed");
            assert_eq!(stored.as_object().unwrap().len(), 1);
            assert!(stored.get("activity").is_some());
            assert_eq!(round_trip(fact.clone()), fact);
        }
    }

    #[test]
    fn test_agent_session活動導出_stopとprocess_exitで次の指示待ちへ戻る() {
        let stop = NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        });
        let process_exit = NodeFact::ProcessExited(ProcessExitedFact {
            exit_code: Some(0),
            result_summary: None,
            failure_reason: None,
            failure_kind: None,
        });

        for fact in [stop, process_exit] {
            assert_eq!(
                AgentSessionActivity::Working.after_fact(&fact),
                AgentSessionActivity::AwaitingInstruction
            );
        }
    }

    #[test]
    fn test_process_exited_終了結果の全項目から異常終了を判定する() {
        let process_exited = |exit_code, failure_reason, failure_kind| ProcessExitedFact {
            exit_code,
            result_summary: None,
            failure_reason,
            failure_kind,
        };

        assert!(!process_exited(Some(0), None, None).is_abnormal());
        assert!(process_exited(Some(1), None, None).is_abnormal());
        assert!(process_exited(None, None, None).is_abnormal());
        assert!(process_exited(Some(0), Some("failed".to_string()), None).is_abnormal());
        assert!(process_exited(
            Some(0),
            None,
            Some(NodeExecutionFailureKind::InfrastructureCrash),
        )
        .is_abnormal());
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
    use crate::domain::workflow::{
        ExecutionTreeLaunch, NodeDefinition, NodeKind, WorkflowDefinition,
    };

    #[test]
    fn test_rootのstarted_workflow構成が定義snapshotごと往復する() {
        // Given: workflow 木の root started
        let fact = NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact {
                workspace_identity: "/repo".to_string(),
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
                launched_as: ExecutionTreeLaunch::Workflow,
            }),
        });

        // When / Then: 往復で同値
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_rootのstarted_単独session構成が往復する() {
        let fact =
            SessionExecutionTreeRootFacts::new("session-1", "/repo", "/repo", ProviderKind::Codex)
                .unwrap()
                .started;
        let detail = fact.encode_detail().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap()["root"]["definition"]
                ["entry"],
            "session"
        );
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_rootのstarted_provider固有permission値はdetail不一致として拒否する() {
        let mut fact =
            SessionExecutionTreeRootFacts::new("session-1", "/repo", "/repo", ProviderKind::Codex)
                .unwrap()
                .started;
        let NodeFact::Started(StartedFact {
            root: Some(root), ..
        }) = &mut fact
        else {
            unreachable!();
        };
        let NodeKind::Session(spec) = &mut root.definition.nodes[0].kind else {
            unreachable!();
        };
        spec.permission = Some(crate::domain::workflow::SessionPermission::Auto);
        let legacy_detail = fact
            .encode_detail()
            .unwrap()
            .replace(r#""permission":"auto""#, r#""permission":"acceptEdits""#);

        let error = NodeFact::decode("started", &legacy_detail).unwrap_err();

        assert!(matches!(
            error,
            NodeFactDecodeError::DetailMismatch { event_type, reason }
                if event_type == "started"
                    && reason.contains("invalid session permission 'acceptEdits'")
        ));
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

mod session_execution_tree_root_facts_tests {
    use super::*;

    #[test]
    fn test_session実行木root構築_session_node一個のcanonical事実を返す() {
        let session_id = "session-1";

        let facts = SessionExecutionTreeRootFacts::new(
            session_id,
            "workspace-1",
            "/repo",
            ProviderKind::Claude,
        )
        .unwrap();

        assert_eq!(facts.meta.tree_id, session_id);
        assert_eq!(facts.meta.node_execution_id, session_id);
        assert_eq!(facts.meta.parent_id, None);
        assert_eq!(facts.meta.node_name, "session");
        assert_eq!(facts.meta.kind, NodeKindName::Session);
        assert_eq!(facts.meta.attempt, 1);
        let NodeFact::Started(StartedFact {
            parent: None,
            root: Some(root),
        }) = &facts.started
        else {
            panic!("started root fact expected");
        };
        assert_eq!(root.launched_as, ExecutionTreeLaunch::Session);
        assert_eq!(root.definition.nodes.len(), 1);
        assert_eq!(root.definition.entry, "session");
        assert_eq!(root.definition.nodes[0].name, "session");
        assert_eq!(root.definition.nodes[0].completion, NodeCompletion::Auto);
        assert!(matches!(
            root.definition.nodes[0].kind,
            NodeKind::Session(SessionSpec {
                provider: ProviderKind::Claude,
                ..
            })
        ));
        assert_eq!(
            facts.attached,
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: session_id.to_string(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            })
        );
    }

    #[test]
    fn test_session実行木root構築_空入力と空白だけの入力を各項目で拒否する() {
        // When / Then
        assert_eq!(
            SessionExecutionTreeRootFacts::new("", "workspace-1", "/repo", ProviderKind::Claude)
                .unwrap_err(),
            SessionExecutionTreeRootFactsError::SessionId
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(" \t", "workspace-1", "/repo", ProviderKind::Claude)
                .unwrap_err(),
            SessionExecutionTreeRootFactsError::SessionId
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new("session-1", "", "/repo", ProviderKind::Claude)
                .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorkspaceIdentity
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new("session-1", " \t", "/repo", ProviderKind::Claude)
                .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorkspaceIdentity
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                "workspace-1",
                "",
                ProviderKind::Claude,
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorktreePath
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                "workspace-1",
                " \t",
                ProviderKind::Claude,
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorktreePath
        );
    }
}
