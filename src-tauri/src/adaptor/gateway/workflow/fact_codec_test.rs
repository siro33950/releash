use super::*;
use crate::adaptor::gateway::workflow::fact_codec;
use crate::domain::provider_lifecycle::ProviderKind;

fn round_trip(fact: NodeFact) -> NodeFact {
    let event_type = fact_codec::event_type(&fact);
    let detail = fact_codec::encode_detail(&fact).unwrap();
    fact_codec::decode(event_type, &detail).unwrap()
}

mod vocabulary_tests {
    use super::*;

    #[test]
    fn test_事実語彙_event_typeが19種の固定文字列である() {
        // Given: 全21 variant
        let facts: Vec<NodeFact> = vec![
            NodeFact::Started(StartedFact {
                worktree: None,
                parent: None,
                root: None,
            }),
            NodeFact::RepositoryRootObserved("/repo".into()),
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
                failure_kind: None,
                exit_code: Some(0),
                result_summary: None,
                failure_reason: None,
            }),
            NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                reason: "activation failed".to_string(),
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            }),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::Working,
            }),
            NodeFact::SessionNodeRenamed(SessionNodeRenamedFact {
                name: "release review".to_string(),
            }),
            NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
                title: "Fix the flaky test".to_string(),
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
            NodeFact::DelegateResultInjected("child-1".into()),
            NodeFact::SessionContinuationAdmitted(SessionContinuationAdmittedFact {
                session_id: "session-1".into(),
                request_id: "workflow-delegate-continuation-child-1".into(),
            }),
            NodeFact::RetryRequested,
            NodeFact::ResumeRequested,
            NodeFact::ExecutionCompleted,
            NodeFact::AbortRequested(Default::default()),
            NodeFact::ArchiveRequested(ArchiveRequestedFact {
                reason: "manual".into(),
                archived_at: 12.345678,
            }),
            NodeFact::RestoreRequested,
        ];

        // When / Then: event_type が確定済み語彙と一致する
        assert_eq!(
            facts.iter().map(fact_codec::event_type).collect::<Vec<_>>(),
            vec![
                "started",
                "repository_root_observed",
                "session_attached",
                "command_spawned",
                "process_exited",
                "runtime_failure_observed",
                "agent_activity_observed",
                "session_node_renamed",
                "provider_session_title_observed",
                "submit_received",
                "submit_rejected",
                "stop_received",
                "artifact_produced",
                "approval_granted",
                "delegate_result_injected",
                "session_continuation_admitted",
                "retry_requested",
                "resume_requested",
                "execution_completed",
                "abort_requested",
                "archive_requested",
                "restore_requested",
            ]
        );

        // Then: 全 variant が encode → decode で往復する
        for fact in facts {
            assert_eq!(round_trip(fact.clone()), fact);
        }
    }

    #[test]
    fn test_session表示名事実_1fieldのdetailを保存して往復する() {
        for (fact, field, value) in [
            (
                NodeFact::SessionNodeRenamed(SessionNodeRenamedFact {
                    name: "release review".to_string(),
                }),
                "name",
                "release review",
            ),
            (
                NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
                    title: "Fix the flaky test".to_string(),
                }),
                "title",
                "Fix the flaky test",
            ),
        ] {
            let detail = fact_codec::encode_detail(&fact).unwrap();
            let stored: serde_json::Value = serde_json::from_str(&detail).unwrap();

            assert_eq!(stored.as_object().unwrap().len(), 1);
            assert_eq!(stored[field], value);
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

            let detail = fact_codec::encode_detail(&fact).unwrap();
            let stored: serde_json::Value = serde_json::from_str(&detail).unwrap();

            assert_eq!(fact_codec::event_type(&fact), "agent_activity_observed");
            assert_eq!(stored.as_object().unwrap().len(), 1);
            assert!(stored.get("activity").is_some());
            assert_eq!(round_trip(fact.clone()), fact);
        }
    }

    #[test]
    fn test_事実語彙_未知のevent_typeは拒否する() {
        // 遷移イベントは語彙に存在しない
        assert_eq!(
            fact_codec::decode("node_completed", "{}"),
            Err(NodeFactDecodeError::UnknownEventType(
                "node_completed".to_string()
            ))
        );
    }

    #[test]
    fn test_session_attachedの必須session_id欠落はdetail不一致になる() {
        let error =
            fact_codec::decode("session_attached", "{}").expect_err("session_id is required");
        assert!(matches!(
            error,
            NodeFactDecodeError::DetailMismatch { event_type, .. }
                if event_type == "session_attached"
        ));
    }

    #[test]
    fn test_payloadなし事実のdetailはjson_object以外を拒否する() {
        for event_type in [
            "retry_requested",
            "resume_requested",
            "abort_requested",
            "restore_requested",
        ] {
            assert!(fact_codec::decode(event_type, "{}").is_ok());
            let error = fact_codec::decode(event_type, "not-json")
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
        let decoded = fact_codec::decode(
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
            worktree: None,
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: None,
                workspace_identity: "/repo".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "please review".to_string(),
                workflow_name: "review".to_string(),
                definition: Some(WorkflowDefinition {
                    name: "review".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![NodeDefinition {
                        name: "main".to_string(),
                        ..NodeDefinition::default()
                    }],
                    entry: "main".to_string(),
                }),
                launched_as: ExecutionTreeLaunch::Workflow,
            })),
        });

        // When / Then: 往復で同値
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_rootのstarted_単独session構成が往復する() {
        let fact = SessionExecutionTreeRootFacts::new(
            "session-1",
            "/repo",
            "/repo",
            ProviderKind::Codex,
            None,
        )
        .unwrap()
        .started;
        let detail = fact_codec::encode_detail(&fact).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap()["root"]["definition"]
                ["entry"],
            "session"
        );
        assert_eq!(round_trip(fact.clone()), fact);
    }

    #[test]
    fn test_rootのstarted_workflow名を定義外に記録し旧形式も全体読取できる() {
        // Given
        let fact = SessionExecutionTreeRootFacts::new(
            "session-1",
            "/repo",
            "/repo",
            ProviderKind::Codex,
            None,
        )
        .unwrap()
        .started;
        let detail = fact_codec::encode_detail(&fact).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(value["root"]["workflowName"], "session");
        value["root"]
            .as_object_mut()
            .unwrap()
            .remove("workflowName");
        // When / Then
        assert_eq!(
            fact_codec::decode("started", &value.to_string()).unwrap(),
            fact
        );
        value["root"]["workflowName"] = serde_json::json!("recorded-name");
        let NodeFact::Started(started) = fact_codec::decode("started", &value.to_string()).unwrap()
        else {
            panic!()
        };
        assert_eq!(started.root.unwrap().workflow_name, "recorded-name");
    }

    #[test]
    fn test_rootのstarted_provider固有permission値はdetail不一致として拒否する() {
        let mut fact = SessionExecutionTreeRootFacts::new(
            "session-1",
            "/repo",
            "/repo",
            ProviderKind::Codex,
            None,
        )
        .unwrap()
        .started;
        let NodeFact::Started(StartedFact {
            worktree: None,
            root: Some(root),
            ..
        }) = &mut fact
        else {
            unreachable!();
        };
        let NodeKind::Session(spec) = &mut root.definition.as_mut().unwrap().nodes[0].kind else {
            unreachable!();
        };
        spec.permission = Some(crate::domain::workflow::SessionPermission::Auto);
        let legacy_detail = fact_codec::encode_detail(&fact)
            .unwrap()
            .replace(r#""permission":"auto""#, r#""permission":"acceptEdits""#);

        let error = fact_codec::decode("started", &legacy_detail).unwrap_err();

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
            worktree: None,
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

#[test]
fn test_delegate注入事実_未知fieldの型を問わずchildを復元する() {
    for unknown in [
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!({"nested": [null]}),
    ] {
        // Given
        let detail = serde_json::json!({"childExecutionId": "child-1", "future": unknown});
        // When
        let fact = fact_codec::decode("delegate_result_injected", &detail.to_string()).unwrap();
        // Then
        assert_eq!(fact, NodeFact::DelegateResultInjected("child-1".into()));
    }
}

#[test]
fn test_delegate注入事実_child識別子の欠落と空文字と非文字列を拒否する() {
    for detail in [
        "{}",
        r#"{"childExecutionId":""}"#,
        r#"{"childExecutionId":42}"#,
    ] {
        // Given / When
        let result = fact_codec::decode("delegate_result_injected", detail);
        // Then
        assert!(matches!(
            result,
            Err(NodeFactDecodeError::DetailMismatch { .. })
        ));
    }
}

#[test]
fn test_終端事実_abort理由と旧形式の空detailを往復する() {
    // Given
    let fact = NodeFact::AbortRequested(AbortRequestedFact {
        reason: Some("Workflow definition is unavailable: completion".into()),
    });
    // When / Then
    assert_eq!(
        fact_codec::decode(
            fact_codec::event_type(&fact),
            &fact_codec::encode_detail(&fact).unwrap()
        )
        .unwrap(),
        fact
    );
    assert_eq!(
        fact_codec::decode("abort_requested", "{}").unwrap(),
        NodeFact::AbortRequested(Default::default())
    );
    assert!(fact_codec::decode("abort_requested", "[]").is_err());
    assert!(fact_codec::decode("execution_completed", "[]").is_err());
}

#[test]
fn test_archive事実codec_旧形式を境界で補い理由と精密な時刻を往復する() {
    // Given / When
    let legacy = crate::adaptor::gateway::workflow::fact_log::decode_stored_fact(
        "archive_requested",
        "{}",
        42125,
    )
    .unwrap()
    .unwrap();
    // Then
    assert_eq!(
        legacy,
        NodeFact::ArchiveRequested(ArchiveRequestedFact {
            reason: "manual".into(),
            archived_at: 42.125
        })
    );
    for reason in ["manual", "worktree_removed"] {
        let fact = NodeFact::ArchiveRequested(ArchiveRequestedFact {
            reason: reason.into(),
            archived_at: 12.345678,
        });
        let detail = fact_codec::encode_detail(&fact).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap(),
            serde_json::json!({ "reason": reason, "archivedAt": 12.345678 })
        );
        assert_eq!(
            fact_codec::decode("archive_requested", &detail).unwrap(),
            fact
        );
    }
    for invalid in [
        "null",
        "[]",
        "invalid",
        "{\"reason\":false}",
        "{\"archivedAt\":\"bad\"}",
    ] {
        assert!(fact_codec::decode("archive_requested", invalid).is_err());
    }
}

#[test]
fn test_repository所属の観測事実_往復し欠損と空のpathを拒否する() {
    // Given
    let fact = NodeFact::RepositoryRootObserved("/repo".into());
    // When / Then
    let detail = fact_codec::encode_detail(&fact).unwrap();
    assert_eq!(
        fact_codec::decode(fact_codec::event_type(&fact), &detail).unwrap(),
        fact
    );
    for invalid in [
        "{}",
        "null",
        "[]",
        r#"{"repositoryRoot":null}"#,
        r#"{"repositoryRoot":1}"#,
        r#"{"repositoryRoot":" "}"#,
    ] {
        assert!(fact_codec::decode(fact_codec::event_type(&fact), invalid).is_err());
    }
}
