use super::*;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::entities::workflow_execution::{
    RuntimeNodeExecutionFailureOrigin, RuntimeNodeExecutionStatus,
};
use crate::domain::workflow::{
    AgentActivityObservedFact, AgentSessionActivity, ApprovalGrantedFact, ArtifactProducedFact,
    ChildEntry, CommandSpec, ExecutionOrigin, ExecutionParentRef, ExecutionTreeLaunch, FanoutSpec,
    NodeCompletion, NodeDefinition, NodeFactMeta, NodeKind, OnFailure, RuntimeExecutionState,
    RuntimeFailureObservedFact, SequenceSpec, SessionAttachedFact, SessionExecutionTreeRootFacts,
    StartedFact, StopReceivedFact, SubmitReceivedFact, WorkflowDefinition,
};

const TREE: &str = "root-exec";

struct FactLog {
    seq: i64,
    records: Vec<NodeFactRecord>,
}

impl FactLog {
    fn new() -> Self {
        Self {
            seq: 0,
            records: Vec::new(),
        }
    }

    fn push(&mut self, meta: NodeFactMeta, fact: NodeFact) {
        self.seq += 1;
        self.records.push(NodeFactRecord {
            meta,
            seq: self.seq,
            timestamp_ms: self.seq * 1000,
            fact,
        });
    }
}

fn meta(
    node_execution_id: &str,
    parent_id: Option<&str>,
    node_name: &str,
    kind: NodeKindName,
    attempt: u32,
) -> NodeFactMeta {
    NodeFactMeta {
        tree_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        parent_id: parent_id.map(str::to_string),
        node_name: node_name.to_string(),
        kind,
        attempt,
    }
}

fn session_leaf(name: &str) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        ..NodeDefinition::default()
    }
}

fn command_leaf(name: &str) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Command(CommandSpec {
            command: "true".to_string(),
            env: Default::default(),
        }),
        ..NodeDefinition::default()
    }
}

fn sequence_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Sequence(SequenceSpec {
            entry: None,
            children,
        }),
        ..NodeDefinition::default()
    }
}

fn fanout_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children,
            items: None,
        }),
        ..NodeDefinition::default()
    }
}

fn workflow_definition(nodes: Vec<NodeDefinition>, entry: &str) -> WorkflowDefinition {
    WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes,
        entry: entry.to_string(),
    }
}

fn workflow_root(definition: WorkflowDefinition) -> TreeRootFact {
    TreeRootFact {
        definition_resolution: Default::default(),
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Cli,
        request: "please work".to_string(),
        definition,
        launched_as: ExecutionTreeLaunch::Workflow,
    }
}

fn session_root() -> TreeRootFact {
    let NodeFact::Started(StartedFact {
        root: Some(root), ..
    }) = SessionExecutionTreeRootFacts::new(TREE, "/repo", "/repo", ProviderKind::Codex)
        .unwrap()
        .started
    else {
        unreachable!();
    };
    root
}

fn started_root(root: TreeRootFact) -> NodeFact {
    NodeFact::Started(StartedFact {
        parent: None,
        root: Some(root),
    })
}

fn started_child(parent: ExecutionParentRef) -> NodeFact {
    NodeFact::Started(StartedFact {
        parent: Some(parent),
        root: None,
    })
}

fn attached(session_id: &str) -> NodeFact {
    NodeFact::SessionAttached(SessionAttachedFact {
        session_id: session_id.to_string(),
        provider_session_id: None,
        transcript_ref: None,
        initial_instruction_admitted: false,
    })
}

fn submit() -> NodeFact {
    NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None })
}

fn artifact(contract: &str, value: serde_json::Value) -> NodeFact {
    NodeFact::ArtifactProduced(ArtifactProducedFact {
        contract: Some(contract.to_string()),
        value,
        request_id: None,
    })
}

fn stop() -> NodeFact {
    NodeFact::StopReceived(StopReceivedFact {
        result_summary: None,
        token_usage: None,
    })
}

fn stop_with_summary(summary: &str) -> NodeFact {
    NodeFact::StopReceived(StopReceivedFact {
        result_summary: Some(summary.to_string()),
        token_usage: None,
    })
}

fn exited(code: i32) -> NodeFact {
    NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
        exit_code: Some(code),
        result_summary: None,
        failure_reason: None,
        failure_kind: None,
    })
}

fn process_lost() -> NodeFact {
    NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
        exit_code: None,
        result_summary: None,
        failure_reason: None,
        failure_kind: None,
    })
}

fn node_status(tree: &FoldedTree, node_execution_id: &str) -> RuntimeNodeExecutionStatus {
    tree.aggregate
        .node_execution(node_execution_id)
        .expect("node execution must exist")
        .status
}

mod standalone_session_tests {
    use super::*;

    #[test]
    fn test_単独session_startedの追記だけで1ノードの実行木として導出される() {
        // Given: 単独 session の root started のみ
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta, attached("session-1"));

        // When: fold する
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 1 ノードの実行木が Running で導出される
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
        assert_eq!(tree.aggregate.node_executions().len(), 1);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Running
        );
        assert_eq!(tree.root.launched_as, ExecutionTreeLaunch::Session);
    }

    #[test]
    fn test_単独session_submitとstopの二信号で完了が導出される() {
        // Given: 完了二信号まで揃った事実列（遷移イベントは存在しない）
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), submit());
        log.push(root_meta, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 完了は事実からの導出のみで決まる
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn test_単独session_活動事実はsubmitとstopの完了条件を変えない() {
        // Given: 活動事実を完了信号の前後へ挟んだ事実列
        let cases = [
            (
                vec![
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::Working,
                    }),
                    submit(),
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::AwaitingAnswer,
                    }),
                ],
                crate::domain::workflow::NodeCompletionSignalState::SubmitReceived,
                RuntimeNodeExecutionStatus::Running,
                RuntimeExecutionState::Running,
                AgentSessionActivity::AwaitingAnswer,
            ),
            (
                vec![
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::Working,
                    }),
                    stop(),
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::AwaitingAnswer,
                    }),
                ],
                crate::domain::workflow::NodeCompletionSignalState::StopReceived,
                RuntimeNodeExecutionStatus::Running,
                RuntimeExecutionState::Running,
                AgentSessionActivity::AwaitingAnswer,
            ),
            (
                vec![
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::Working,
                    }),
                    submit(),
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::AwaitingAnswer,
                    }),
                    stop(),
                    NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                        activity: AgentSessionActivity::Working,
                    }),
                ],
                crate::domain::workflow::NodeCompletionSignalState::Ready,
                RuntimeNodeExecutionStatus::Succeeded,
                RuntimeExecutionState::Completed,
                AgentSessionActivity::Working,
            ),
        ];

        for (facts, expected_signals, expected_status, expected_state, expected_activity) in cases {
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            for fact in facts {
                log.push(root_meta.clone(), fact);
            }

            // When: 事実列を実行木へ fold する
            let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
            let node = tree.aggregate.node_execution("root-exec").unwrap();

            // Then: 完了は二信号だけで決まり、活動状態は最後の観測を保つ
            assert_eq!(node.completion_signals, expected_signals);
            assert_eq!(node.status, expected_status);
            assert_eq!(*tree.aggregate.state(), expected_state);
            assert_eq!(
                tree.session_activities.get("root-exec"),
                Some(&expected_activity)
            );
        }
    }

    #[test]
    fn test_単独session_stop後のprocess_exitは未決着nodeに正常と異常を適用する() {
        for (fact, expected) in [
            (exited(0), RuntimeNodeExecutionStatus::Paused),
            (process_lost(), RuntimeNodeExecutionStatus::Failed),
        ] {
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            log.push(root_meta.clone(), stop());
            log.push(root_meta, fact);

            let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
            let node = tree.aggregate.node_execution("root-exec").unwrap();

            assert_eq!(node.status, expected);
            assert_eq!(
                node.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
        }
    }

    #[test]
    fn test_単独session_異常process_exit後のstopはfailedへ決着済みのため無視する() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), process_lost());
        log.push(root_meta, stop());

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let node = tree.aggregate.node_execution("root-exec").unwrap();

        assert_eq!(node.status, RuntimeNodeExecutionStatus::Failed);
        assert_eq!(
            node.completion_signals,
            crate::domain::workflow::NodeCompletionSignalState::Pending
        );
    }

    #[test]
    fn test_単独session_完了後に遅着した異常process_exitを無視する() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), submit());
        log.push(root_meta.clone(), stop());
        log.push(root_meta, process_lost());

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let node = tree.aggregate.node_execution("root-exec").unwrap();

        assert_eq!(node.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(
            node.completion_signals,
            crate::domain::workflow::NodeCompletionSignalState::Ready
        );
    }

    #[test]
    fn test_単独session_stopが運ぶ結果summaryがread_modelへ導出される() {
        // Given: 親スコープを持たない root leaf の stop が result summary を運ぶ
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), submit());
        log.push(
            root_meta,
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: Some("summarized".to_string()),
                token_usage: None,
            }),
        );

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let model = derive_read_model(&tree);

        // Then: root leaf でも result_summary が失われない
        assert_eq!(
            model.node_executions[0].result_summary.as_deref(),
            Some("summarized")
        );
        assert_eq!(
            model.status,
            crate::domain::workflow::ExecutionStatus::Completed
        );
    }

    #[test]
    fn test_単独session_archiveとrestoreが最終状態として導出される() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), NodeFact::ArchiveRequested);
        assert!(super::derive_session_facts(&log.records, "root-exec", "root-exec").archived);

        log.push(root_meta, NodeFact::RestoreRequested);
        assert!(!super::derive_session_facts(&log.records, "root-exec", "root-exec").archived);
    }

    #[test]
    fn test_単独session_process_exitの状態と異常終了を導出する() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), exited(0));
        let normal = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(normal.exited);
        assert!(!normal.last_exit_abnormal);

        log.push(root_meta.clone(), attached("session-1"));
        let reattached = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(!reattached.exited);

        log.push(root_meta.clone(), exited(1));
        let non_zero = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(non_zero.exited);
        assert!(non_zero.last_exit_abnormal);

        log.push(
            root_meta,
            NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
                exit_code: Some(0),
                result_summary: None,
                failure_reason: Some("provider failure".to_string()),
                failure_kind: None,
            }),
        );
        let failed = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(failed.last_exit_abnormal);
    }

    #[test]
    fn test_単独session_活動状態を初期値と最後の観測から導出しprocess_exitで戻す() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));

        let initial = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            initial.session_activities.get("root-exec"),
            Some(&AgentSessionActivity::AwaitingInstruction)
        );

        log.push(
            root_meta.clone(),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::Working,
            }),
        );
        log.push(
            root_meta.clone(),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::AwaitingAnswer,
            }),
        );
        let waiting = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            waiting.session_activities.get("root-exec"),
            Some(&AgentSessionActivity::AwaitingAnswer)
        );
        assert_eq!(
            derive_session_facts(&log.records, "root-exec", "session-1").activity,
            AgentSessionActivity::AwaitingAnswer
        );

        log.push(root_meta, exited(0));
        let exited = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            exited.session_activities.get("root-exec"),
            Some(&AgentSessionActivity::AwaitingInstruction)
        );
        assert_eq!(
            derive_session_facts(&log.records, "root-exec", "session-1").activity,
            AgentSessionActivity::AwaitingInstruction
        );
    }

    #[test]
    fn test_単独session_process_exitの正常終了はpausedで異常終了はfailedになる() {
        for (fact, expected) in [
            (exited(0), RuntimeNodeExecutionStatus::Paused),
            (exited(1), RuntimeNodeExecutionStatus::Failed),
            (process_lost(), RuntimeNodeExecutionStatus::Failed),
            (
                NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
                    exit_code: Some(0),
                    result_summary: None,
                    failure_reason: Some("provider failure".to_string()),
                    failure_kind: None,
                }),
                RuntimeNodeExecutionStatus::Failed,
            ),
        ] {
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            log.push(root_meta, fact);

            let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

            assert_eq!(node_status(&tree, "root-exec"), expected);
        }
    }

    #[test]
    fn test_provider参照なしのattachは確定済みprovider参照を上書きしない() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(
            root_meta.clone(),
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "session-1".to_string(),
                provider_session_id: Some("provider-session-1".to_string()),
                transcript_ref: Some("provider://transcript/1".to_string()),
                initial_instruction_admitted: false,
            }),
        );
        log.push(root_meta, attached("session-1"));

        let view = super::derive_session_facts(&log.records, "root-exec", "session-1");

        assert_eq!(
            view.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
        assert_eq!(
            view.transcript_ref.as_deref(),
            Some("provider://transcript/1")
        );
    }
}

mod isolated_worktree_ledger_tests {
    use super::*;
    use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;
    use crate::domain::workflow::IsolatedWorktreeLifecycle;

    #[test]
    fn test_隔離worktree出自は事実の再decode後も同じ台帳へ導出される() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(
            root_meta,
            NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/projects/repo".to_string(),
                worktree_path: "/projects/repo-worktrees/.releash-isolated/root-exec-a1"
                    .to_string(),
                branch: "releash/isolated/root-exec-a1".to_string(),
            }),
        );

        let persisted = log
            .records
            .iter()
            .cloned()
            .map(|mut record| {
                let event_type = record.fact.event_type();
                let detail = record.fact.encode_detail().unwrap();
                record.fact = NodeFact::decode(event_type, &detail).unwrap();
                record
            })
            .collect::<Vec<_>>();
        let restarted = fold_execution_tree(TREE, &persisted).unwrap().unwrap();
        let entry = restarted.isolated_worktrees.entries().next().unwrap();

        assert_eq!(entry.lifecycle, IsolatedWorktreeLifecycle::Created);
        assert_eq!(entry.owner.node_execution_id, "root-exec");
        assert_eq!(
            entry.worktree_path,
            "/projects/repo-worktrees/.releash-isolated/root-exec-a1"
        );
    }
}

mod session_display_name_fact_tests {
    use super::*;
    use crate::domain::workflow::{ProviderSessionTitleObservedFact, SessionNodeRenamedFact};

    fn provider_title(title: &str) -> NodeFact {
        NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
            title: title.to_string(),
        })
    }

    fn assert_provider_title(log: &FactLog, expected: Option<&str>) {
        assert_eq!(
            derive_session_facts(&log.records, "root-exec", "session-1")
                .provider_session_title
                .as_deref(),
            expected
        );
        assert_eq!(
            fold_execution_tree(TREE, &log.records)
                .unwrap()
                .unwrap()
                .session_display_names
                .get("root-exec")
                .and_then(|inputs| inputs.provider_session_title.as_deref()),
            expected
        );
    }

    #[test]
    fn test_session表示名事実は実行状態と完了signalを変えない() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        let before = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        log.push(
            root_meta.clone(),
            NodeFact::SessionNodeRenamed(SessionNodeRenamedFact {
                name: "release review".to_string(),
            }),
        );
        log.push(
            root_meta,
            NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
                title: "Fix the flaky test".to_string(),
            }),
        );
        let after = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        assert_eq!(after.aggregate.state(), before.aggregate.state());
        assert_eq!(
            after.aggregate.node_executions()[0].status,
            before.aggregate.node_executions()[0].status
        );
        assert_eq!(
            after.aggregate.node_executions()[0].completion_signals,
            before.aggregate.node_executions()[0].completion_signals
        );
    }

    #[test]
    fn test_session表示名事実はrenameとproviderタイトルを独立に最後の値から導出する() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        for fact in [
            NodeFact::SessionNodeRenamed(SessionNodeRenamedFact {
                name: "first name".to_string(),
            }),
            NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
                title: "first title".to_string(),
            }),
            NodeFact::SessionNodeRenamed(SessionNodeRenamedFact {
                name: "latest name".to_string(),
            }),
            NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
                title: "latest title".to_string(),
            }),
        ] {
            log.push(root_meta.clone(), fact);
        }

        let view = derive_session_facts(&log.records, "root-exec", "session-1");

        assert_eq!(view.manual_name.as_deref(), Some("latest name"));
        assert_eq!(view.provider_session_title.as_deref(), Some("latest title"));
        assert_eq!(
            fold_execution_tree(TREE, &log.records)
                .unwrap()
                .unwrap()
                .session_display_names
                .get("root-exec"),
            Some(&SessionDisplayNameInputs {
                manual_name: Some("latest name".to_string()),
                provider_session_title: Some("latest title".to_string()),
            })
        );
    }

    #[test]
    fn test_providerタイトル_停止前の最終値を停止後の観測で上書きしない() {
        for stop in [exited(0), NodeFact::ArchiveRequested] {
            // Given
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            log.push(root_meta.clone(), provider_title("active title"));
            log.push(root_meta.clone(), stop);

            // When
            log.push(root_meta, provider_title("title observed after stop"));

            // Then
            assert_provider_title(&log, Some("active title"));
        }
    }

    #[test]
    fn test_providerタイトル_未観測のまま停止した後の観測を採用しない() {
        for stop in [exited(0), NodeFact::ArchiveRequested] {
            // Given
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            log.push(root_meta.clone(), stop);

            // When
            log.push(root_meta, provider_title("title observed after stop"));

            // Then
            assert_provider_title(&log, None);
        }
    }

    #[test]
    fn test_providerタイトル_再活動後の観測を採用する() {
        for (stop, reactivate) in [
            (exited(0), NodeFact::ResumeRequested),
            (exited(0), attached("session-1")),
            (NodeFact::ArchiveRequested, NodeFact::RestoreRequested),
        ] {
            // Given
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
            log.push(root_meta.clone(), started_root(session_root()));
            log.push(root_meta.clone(), attached("session-1"));
            log.push(root_meta.clone(), provider_title("active title"));
            log.push(root_meta.clone(), stop);
            log.push(
                root_meta.clone(),
                provider_title("title observed after stop"),
            );
            assert_provider_title(&log, Some("active title"));

            // When
            log.push(root_meta.clone(), reactivate);
            log.push(root_meta, provider_title("reactivated title"));

            // Then
            assert_provider_title(&log, Some("reactivated title"));
        }
    }

    #[test]
    fn test_providerタイトル_別sessionのattachでは停止を解除しない() {
        // Given
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), exited(0));

        // When
        log.push(root_meta.clone(), attached("session-2"));
        log.push(root_meta, provider_title("other session title"));

        // Then
        assert_provider_title(&log, None);
    }
}

mod sequence_tests {
    use super::*;

    fn two_step_definition() -> WorkflowDefinition {
        workflow_definition(
            vec![
                session_leaf("a"),
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
                ),
            ],
            "main",
        )
    }

    #[test]
    fn test_sequence_子の完了導出と前進の事実で終端到達が完了になる() {
        // Given: a → b と進んだ workflow の事実列
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(two_step_definition())),
        );
        let a = meta("a-exec", Some("main-exec"), "a", NodeKindName::Session, 1);
        log.push(
            a.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(a.clone(), attached("session-a"));
        log.push(a.clone(), submit());
        log.push(a, stop());
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), attached("session-b"));
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: a / b とも完了、sequence は終端到達で完了、木全体も完了
        assert_eq!(
            node_status(&tree, "a-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            node_status(&tree, "b-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            node_status(&tree, "main-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_sequence_前半のみの事実列では実行中のままになる() {
        // Given: a 完了までの事実列（b は未開始）
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(two_step_definition())),
        );
        let a = meta("a-exec", Some("main-exec"), "a", NodeKindName::Session, 1);
        log.push(
            a.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(a.clone(), submit());
        log.push(a, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: a は完了・木は実行中（前進は事実が無い限り起きない）
        assert_eq!(
            node_status(&tree, "a-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
    }

    #[test]
    fn test_sequence_stop後のartifact付きsubmitを統合mapとして完了導出する() {
        // Given: 子で Stop が先着し、Submit の直後に Artifact が記録された事実列
        let mut output = session_leaf("make_plan");
        output.artifact = Some("plan".to_string());
        let main = sequence_node("main", vec![ChildEntry::reference("make_plan")]);

        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(workflow_definition(
                vec![output, main],
                "main",
            ))),
        );
        let make_plan = meta(
            "make-plan-exec",
            Some("main-exec"),
            "make_plan",
            NodeKindName::Session,
            1,
        );
        log.push(
            make_plan.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(make_plan.clone(), stop());
        log.push(make_plan.clone(), submit());
        let artifact = serde_json::json!({"plan": "ready"});
        log.push(
            make_plan,
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("plan".to_string()),
                value: artifact.clone(),
                request_id: None,
            }),
        );

        // When: 永続化順に事実列を fold する
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let make_plan = tree.aggregate.node_execution("make-plan-exec").unwrap();
        let main = tree.aggregate.node_execution("main-exec").unwrap();

        // Then: 子の Artifact を Sequence の統合 map に含めて成功する
        assert_eq!(make_plan.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(make_plan.artifact.as_ref(), Some(&artifact));
        assert_eq!(main.failure, None);
        assert_eq!(main.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(
            main.artifact,
            Some(serde_json::json!({"make_plan": artifact}))
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_sequence_artifact付きsubmit後のstopと同一artifact再適用で成果が変わらない() {
        let mut output = session_leaf("make_plan");
        output.artifact = Some("plan".to_string());
        let main = sequence_node("main", vec![ChildEntry::reference("make_plan")]);

        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(workflow_definition(
                vec![output, main],
                "main",
            ))),
        );
        let make_plan = meta(
            "make-plan-exec",
            Some("main-exec"),
            "make_plan",
            NodeKindName::Session,
            1,
        );
        log.push(
            make_plan.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let produced = serde_json::json!({"plan": "ready"});
        log.push(make_plan.clone(), submit());
        log.push(make_plan.clone(), artifact("plan", produced.clone()));
        log.push(make_plan.clone(), stop());

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let output = tree.aggregate.node_execution("make-plan-exec").unwrap();
        let sequence = tree.aggregate.node_execution("main-exec").unwrap();
        assert_eq!(output.artifact.as_ref(), Some(&produced));
        assert_eq!(
            sequence.artifact,
            Some(serde_json::json!({"make_plan": produced}))
        );
        assert_eq!(sequence.status, RuntimeNodeExecutionStatus::Succeeded);

        log.push(make_plan, artifact("plan", produced.clone()));
        let replayed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let replayed_output = replayed.aggregate.node_execution("make-plan-exec").unwrap();
        let replayed_sequence = replayed.aggregate.node_execution("main-exec").unwrap();
        assert_eq!(replayed_output.artifact.as_ref(), Some(&produced));
        assert_eq!(
            replayed_sequence.artifact,
            Some(serde_json::json!({"make_plan": produced}))
        );
        assert_eq!(
            replayed_sequence.status,
            RuntimeNodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn test_sequence_stop先着の子のartifactを下流入力と統合mapに使う() {
        let mut output = session_leaf("make_plan");
        output.artifact = Some("plan".to_string());
        let mut downstream = session_leaf("judge");
        downstream.input.push(crate::domain::workflow::InputParam {
            name: "plan".to_string(),
            contract: Some("plan".to_string()),
        });
        let main = sequence_node(
            "main",
            vec![
                ChildEntry::reference("make_plan"),
                ChildEntry {
                    name: "judge".to_string(),
                    inputs: vec![(
                        "plan".to_string(),
                        crate::domain::workflow::value_objects::InputSourceRef::new("make_plan"),
                    )],
                    rules: None,
                    on_failure: None,
                },
            ],
        );

        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(workflow_definition(
                vec![output, downstream, main],
                "main",
            ))),
        );
        let make_plan = meta(
            "make-plan-exec",
            Some("main-exec"),
            "make_plan",
            NodeKindName::Session,
            1,
        );
        log.push(
            make_plan.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let produced = serde_json::json!({"plan": "ready"});
        log.push(make_plan.clone(), stop());
        log.push(make_plan.clone(), submit());
        log.push(make_plan, artifact("plan", produced.clone()));

        let judge = meta(
            "judge-exec",
            Some("main-exec"),
            "judge",
            NodeKindName::Session,
            1,
        );
        log.push(
            judge.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let running = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            running
                .aggregate
                .leaf_start_for("judge-exec")
                .unwrap()
                .bindings,
            vec![("plan".to_string(), produced.clone())]
        );

        log.push(judge.clone(), submit());
        log.push(judge, stop());
        let completed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let output = completed
            .aggregate
            .node_execution("make-plan-exec")
            .unwrap();
        let sequence = completed.aggregate.node_execution("main-exec").unwrap();
        assert_eq!(output.artifact.as_ref(), Some(&produced));
        assert_eq!(
            sequence.artifact,
            Some(serde_json::json!({"make_plan": produced}))
        );
        assert_eq!(sequence.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(sequence.failure, None);
        assert!(!sequence.can_retry());
    }

    #[test]
    fn test_sequence_子がartifactなしで終端到達すると空mapを導出する() {
        let mut output = session_leaf("make_plan");
        output.artifact = Some("plan".to_string());
        let main = sequence_node("main", vec![ChildEntry::reference("make_plan")]);

        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(workflow_definition(
                vec![output, main],
                "main",
            ))),
        );
        let make_plan = meta(
            "make-plan-exec",
            Some("main-exec"),
            "make_plan",
            NodeKindName::Session,
            1,
        );
        log.push(
            make_plan.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(make_plan.clone(), stop());
        log.push(make_plan, submit());

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let sequence = tree.aggregate.node_execution("main-exec").unwrap();
        assert_eq!(sequence.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(sequence.failure, None);
        assert_eq!(sequence.artifact, Some(serde_json::json!({})));
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }
}

mod fanout_tests {
    use super::*;

    #[test]
    fn test_fanout_全子完了で合成子と木全体の完了が導出される() {
        // Given: fanout の 2 子が完了した事実列
        let definition = workflow_definition(
            vec![
                session_leaf("x"),
                session_leaf("y"),
                fanout_node(
                    "fan",
                    vec![ChildEntry::reference("x"), ChildEntry::reference("y")],
                ),
                sequence_node("main", vec![ChildEntry::reference("fan")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        log.push(
            meta(
                "fan-exec",
                Some("main-exec"),
                "fan",
                NodeKindName::Fanout,
                1,
            ),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let x = meta("x-exec", Some("fan-exec"), "x", NodeKindName::Session, 1);
        log.push(
            x.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 0)),
        );
        let y = meta("y-exec", Some("fan-exec"), "y", NodeKindName::Session, 1);
        log.push(
            y.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 1)),
        );
        log.push(x.clone(), submit());
        log.push(x, stop());

        // 1 子完了時点では fanout は未完了
        let halfway = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&halfway, "fan-exec"),
            RuntimeNodeExecutionStatus::Running
        );

        log.push(y.clone(), submit());
        log.push(y, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 全子完了 → fanout 完了 → sequence 終端 → 木完了
        assert_eq!(
            node_status(&tree, "fan-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_fanout_同名同attemptの並走laneで結果summaryを取り違えない() {
        let definition = workflow_definition(
            vec![
                session_leaf("worker"),
                fanout_node(
                    "fan",
                    vec![
                        ChildEntry::reference("worker"),
                        ChildEntry::reference("worker"),
                    ],
                ),
                sequence_node("main", vec![ChildEntry::reference("fan")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        log.push(
            meta(
                "fan-exec",
                Some("main-exec"),
                "fan",
                NodeKindName::Fanout,
                1,
            ),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        for (index, id) in [(0, "worker-a"), (1, "worker-b")] {
            let worker = meta(id, Some("fan-exec"), "worker", NodeKindName::Session, 1);
            log.push(
                worker.clone(),
                started_child(ExecutionParentRef::fanout_child("fan-exec", None, index)),
            );
        }
        for (id, summary) in [("worker-a", None), ("worker-b", Some("result-b"))] {
            let worker = meta(id, Some("fan-exec"), "worker", NodeKindName::Session, 1);
            log.push(worker.clone(), submit());
            log.push(worker, summary.map_or_else(stop, stop_with_summary));
        }

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let model = derive_read_model(&tree);

        for (id, expected) in [("worker-a", None), ("worker-b", Some("result-b"))] {
            assert_eq!(
                model
                    .node_executions
                    .iter()
                    .find(|node| node.id == id)
                    .and_then(|node| node.result_summary.as_deref()),
                expected
            );
        }
    }

    #[test]
    fn test_fanout_stop先着の最終子artifactをnullにせず集約する() {
        let mut x = session_leaf("x");
        x.artifact = Some("result".to_string());
        let mut y = session_leaf("y");
        y.artifact = Some("result".to_string());
        let definition = workflow_definition(
            vec![
                x,
                y,
                fanout_node(
                    "fan",
                    vec![ChildEntry::reference("x"), ChildEntry::reference("y")],
                ),
                sequence_node("main", vec![ChildEntry::reference("fan")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        log.push(
            meta(
                "fan-exec",
                Some("main-exec"),
                "fan",
                NodeKindName::Fanout,
                1,
            ),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let x = meta("x-exec", Some("fan-exec"), "x", NodeKindName::Session, 1);
        log.push(
            x.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 0)),
        );
        let y = meta("y-exec", Some("fan-exec"), "y", NodeKindName::Session, 1);
        log.push(
            y.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 1)),
        );
        let x_artifact = serde_json::json!({"result": "x"});
        log.push(x.clone(), submit());
        log.push(x.clone(), artifact("result", x_artifact.clone()));
        log.push(x, stop());

        let y_artifact = serde_json::json!({"result": "y"});
        log.push(y.clone(), stop());
        log.push(y.clone(), submit());
        log.push(y, artifact("result", y_artifact.clone()));

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let fanout = tree.aggregate.node_execution("fan-exec").unwrap();
        assert_eq!(
            fanout.artifact.as_ref(),
            Some(&serde_json::json!([x_artifact, y_artifact]))
        );
        assert_eq!(fanout.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }
}

mod approval_tests {
    use super::*;

    fn approval_definition() -> WorkflowDefinition {
        let mut reviewed = session_leaf("reviewed");
        reviewed.completion = NodeCompletion::Approval;
        workflow_definition(
            vec![
                reviewed,
                sequence_node("main", vec![ChildEntry::reference("reviewed")]),
            ],
            "main",
        )
    }

    #[test]
    fn test_approval_二信号が揃っても承認事実まで完了しない() {
        // Given: completion: approval の node が二信号まで揃った事実列
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(approval_definition())),
        );
        let node = meta(
            "r-exec",
            Some("main-exec"),
            "reviewed",
            NodeKindName::Session,
            1,
        );
        log.push(
            node.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(node.clone(), submit());
        log.push(node.clone(), stop());

        // When / Then: 承認待ちの導出
        let waiting = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&waiting, "r-exec"),
            RuntimeNodeExecutionStatus::WaitingApproval
        );
        assert_eq!(*waiting.aggregate.state(), RuntimeExecutionState::Running);

        // When: approval_granted の追記
        log.push(
            node,
            NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
        );
        let approved = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: human の承認事実で完了が導出される
        assert_eq!(
            node_status(&approved, "r-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            *approved.aggregate.state(),
            RuntimeExecutionState::Completed
        );
    }

    #[test]
    fn test_approval_正常なprocess_exit後も承認待ちを維持し活動だけを待機へ戻す() {
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(approval_definition())),
        );
        let node = meta(
            "r-exec",
            Some("main-exec"),
            "reviewed",
            NodeKindName::Session,
            1,
        );
        log.push(
            node.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(node.clone(), attached("session-1"));
        log.push(
            node.clone(),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::Working,
            }),
        );
        log.push(node.clone(), submit());
        log.push(node.clone(), stop());
        log.push(node, exited(0));

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        assert_eq!(
            node_status(&tree, "r-exec"),
            RuntimeNodeExecutionStatus::WaitingApproval
        );
        assert_eq!(
            derive_session_facts(&log.records, "r-exec", "session-1").activity,
            AgentSessionActivity::AwaitingInstruction
        );
    }

    #[test]
    fn test_approval_承認待ちの異常process_exitはfailedになる() {
        for process_exit in [exited(1), process_lost()] {
            // Given: completion: approval の Session Node が承認待ちに到達した事実列
            let mut log = FactLog::new();
            log.push(
                meta("main-exec", None, "main", NodeKindName::Sequence, 1),
                started_root(workflow_root(approval_definition())),
            );
            let node = meta(
                "r-exec",
                Some("main-exec"),
                "reviewed",
                NodeKindName::Session,
                1,
            );
            log.push(
                node.clone(),
                started_child(ExecutionParentRef::sequence_child("main-exec")),
            );
            log.push(node.clone(), attached("session-1"));
            log.push(node.clone(), submit());
            log.push(node.clone(), stop());
            assert_eq!(
                node_status(
                    &fold_execution_tree(TREE, &log.records).unwrap().unwrap(),
                    "r-exec",
                ),
                RuntimeNodeExecutionStatus::WaitingApproval
            );

            // When: exit code 非0または process lost を適用する
            log.push(node, process_exit);
            let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

            // Then: 承認待ちも異常終了では Failed になる
            assert_eq!(
                node_status(&tree, "r-exec"),
                RuntimeNodeExecutionStatus::Failed
            );
        }
    }
}

mod failure_tests {
    use super::*;

    #[test]
    fn test_on_failure_retry_失敗とretry事実の再実行で完了に到達する() {
        // Given: command が失敗 → retry → 成功した事実列
        let document = "document body";
        let mut retried_command = command_leaf("c");
        retried_command
            .input
            .push(crate::domain::workflow::InputParam {
                name: "document".to_string(),
                contract: None,
            });
        let NodeKind::Command(command) = &mut retried_command.kind else {
            unreachable!();
        };
        command.env = [(
            crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap(),
            crate::domain::workflow::InputParameterRef::new("document").unwrap(),
        )]
        .into_iter()
        .collect();
        let expected_env = command.env.clone();
        let definition = workflow_definition(
            vec![
                retried_command,
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![
                        ChildEntry {
                            name: "c".to_string(),
                            inputs: vec![(
                                "document".to_string(),
                                crate::domain::workflow::value_objects::InputSourceRef::new(
                                    "request",
                                ),
                            )],
                            rules: None,
                            on_failure: Some(OnFailure::Retry(1)),
                        },
                        ChildEntry::reference("b"),
                    ],
                ),
            ],
            "main",
        );
        let mut log = FactLog::new();
        let mut root = workflow_root(definition);
        root.request = document.to_string();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(root),
        );
        let first = meta("c-exec-1", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            first.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(first.clone(), exited(1));
        log.push(first, NodeFact::RetryRequested);
        let second = meta("c-exec-2", Some("main-exec"), "c", NodeKindName::Command, 2);
        log.push(
            second.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );

        // When: retry attempt の Started までを fold する
        let retried_tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let retried_leaf = retried_tree.aggregate.leaf_start_for("c-exec-2").unwrap();

        // Then: 保存済み定義と再構築 binding から元の env 値を再解決できる
        assert_eq!(
            retried_leaf.bindings,
            vec![(
                "document".to_string(),
                serde_json::Value::String(document.to_string()),
            )]
        );
        let root = &retried_tree.root;
        let command = root
            .definition
            .node_by_name("c")
            .and_then(NodeDefinition::command_spec)
            .unwrap();
        assert_eq!(&command.env, &expected_env);
        assert_eq!(
            crate::domain::workflow::services::reference::resolve_command_environment(
                &command.env,
                &retried_leaf.bindings,
            )
            .unwrap(),
            vec![("DOC".to_string(), document.to_string())]
        );

        // Given: retry attempt が成功し、後続 session も完了した事実列
        log.push(second, exited(0));
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: attempt 1 は失敗のまま行として残り、attempt 2 の完了で前進した
        assert_eq!(
            node_status(&tree, "c-exec-1"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(
            node_status(&tree, "c-exec-2"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_on_failure_ignore_失敗のまま親の前進が導出される() {
        // Given: on_failure: ignore の command が失敗し、次の子が完了した事実列
        let definition = workflow_definition(
            vec![
                command_leaf("c"),
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![
                        ChildEntry {
                            name: "c".to_string(),
                            inputs: Vec::new(),
                            rules: None,
                            on_failure: Some(OnFailure::Ignore),
                        },
                        ChildEntry::reference("b"),
                    ],
                ),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        let c = meta("c-exec", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            c.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(c, exited(1));
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: c は失敗のまま、b の完了で木全体は完了
        assert_eq!(
            node_status(&tree, "c-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_失敗既定_on_failure宣言なしは失敗で停止したままになる() {
        let definition = workflow_definition(
            vec![
                command_leaf("c"),
                sequence_node("main", vec![ChildEntry::reference("c")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        let c = meta("c-exec", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            c.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(c, exited(1));

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&tree, "c-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
    }
}

mod paused_tests {
    use super::*;

    #[test]
    fn test_paused_正常終了は導出でありpause事実は存在しない() {
        // Given: 二信号未揃いのままプロセスが正常終了した session
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), exited(0));

        // When / Then: Paused は process_exited と二信号未揃いからの純導出
        let paused = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&paused, "root-exec"),
            RuntimeNodeExecutionStatus::Paused
        );
        assert_eq!(*paused.aggregate.state(), RuntimeExecutionState::Running);

        // When: resume の指示と再 attach
        log.push(root_meta.clone(), NodeFact::ResumeRequested);
        log.push(root_meta, attached("session-2"));
        let resumed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: Running へ戻る
        assert_eq!(
            node_status(&resumed, "root-exec"),
            RuntimeNodeExecutionStatus::Running
        );
    }

    #[test]
    fn test_failed_session_resume_requestedで同じattemptをrunningへ戻す() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), process_lost());

        let failed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&failed, "root-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
        let failed_node = failed.aggregate.node_execution("root-exec").unwrap();
        assert!(failed_node.can_resume());
        assert_eq!(
            failed_node.failure.as_ref().map(|failure| failure.origin),
            Some(RuntimeNodeExecutionFailureOrigin::ProviderProcessExit)
        );

        log.push(root_meta, NodeFact::ResumeRequested);
        let resumed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let node = resumed.aggregate.node_execution("root-exec").unwrap();

        assert_eq!(node.status, RuntimeNodeExecutionStatus::Running);
        assert_eq!(node.attempt, 1);
        assert_eq!(node.failure, None);
    }

    #[test]
    fn test_runtime失敗のsessionはfailedでもresume対象にならずsession参照を要求しない() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(
            root_meta.clone(),
            NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                reason: "activation failed".to_string(),
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            }),
        );

        let failed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let node = failed.aggregate.node_execution("root-exec").unwrap();
        assert_eq!(node.status, RuntimeNodeExecutionStatus::Failed);
        assert_eq!(node.session_id, None);
        assert_eq!(
            node.failure.as_ref().map(|failure| failure.origin),
            Some(RuntimeNodeExecutionFailureOrigin::Runtime)
        );
        assert!(!node.can_resume());

        log.push(root_meta, NodeFact::ResumeRequested);
        let resumed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&resumed, "root-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
    }
}

mod command_process_exit_tests {
    use super::*;

    #[test]
    fn test_command_process_exit_exit_codeで完了失敗中断を導出する() {
        for (fact, expected) in [
            (exited(0), RuntimeNodeExecutionStatus::Succeeded),
            (exited(1), RuntimeNodeExecutionStatus::Failed),
            (process_lost(), RuntimeNodeExecutionStatus::Paused),
        ] {
            let mut log = FactLog::new();
            let root_meta = meta("root-exec", None, "command", NodeKindName::Command, 1);
            log.push(
                root_meta.clone(),
                started_root(workflow_root(workflow_definition(
                    vec![command_leaf("command")],
                    "command",
                ))),
            );
            log.push(root_meta, fact);

            let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

            assert_eq!(node_status(&tree, "root-exec"), expected);
        }
    }

    #[test]
    fn test_failed_command_resume_requestedではfailedのままにする() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "command", NodeKindName::Command, 1);
        log.push(
            root_meta.clone(),
            started_root(workflow_root(workflow_definition(
                vec![command_leaf("command")],
                "command",
            ))),
        );
        log.push(root_meta.clone(), exited(1));
        log.push(root_meta, NodeFact::ResumeRequested);

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
    }
}

mod abort_tests {
    use super::*;

    #[test]
    fn test_abort_指示の事実だけで中止が導出される() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta, NodeFact::AbortRequested);

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Aborted);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Aborted
        );
    }
}

mod retroactive_interpretation_tests {
    use super::*;

    /// 許容済みトレードオフの固定: 「当時完了と判定した」という記録は持たず、
    /// 完了は fold 時点の規則で毎回導出される。規則が変われば同じログの解釈も
    /// 変わる。このテストは、完了・遷移を表す事実が入力に一切存在しないことと、
    /// それでも完了が導出されることを両方主張する。
    #[test]
    fn test_遡及_完了は記録ではなくfold時点の規則による導出である() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "session", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), submit());
        log.push(root_meta, stop());

        // 入力の事実語彙に遷移（completed 等）は存在しない
        for record in &log.records {
            assert!(matches!(
                record.fact.event_type(),
                "started" | "submit_received" | "stop_received"
            ));
        }

        // それでも完了は導出される（規則: Submit + Stop 揃いで完了）
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }
}

mod input_validation_tests {
    use super::*;

    #[test]
    fn test_空の事実列は木が存在しない() {
        assert!(fold_execution_tree(TREE, &[]).unwrap().is_none());
    }

    #[test]
    fn test_別のtreeの行が混ざった入力は拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "session", NodeKindName::Session, 1),
            started_root(session_root()),
        );
        assert!(fold_execution_tree("other-tree", &log.records).is_err());
    }

    #[test]
    fn test_先頭がstartedではない事実列を拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "session", NodeKindName::Session, 1),
            submit(),
        );

        assert!(fold_execution_tree(TREE, &log.records).is_err());
    }

    #[test]
    fn test_先頭startedがrootを持たない事実列を拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "session", NodeKindName::Session, 1),
            NodeFact::Started(StartedFact {
                parent: None,
                root: None,
            }),
        );

        assert!(fold_execution_tree(TREE, &log.records).is_err());
    }
}

#[path = "fact_replay_recovery_test.rs"]
mod recovery_tests;
