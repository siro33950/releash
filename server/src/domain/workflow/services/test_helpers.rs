use crate::domain::workflow::{NodeFact, SessionAttachedFact};

pub fn session_attached(session_id: &str) -> NodeFact {
    NodeFact::SessionAttached(SessionAttachedFact {
        session_id: session_id.to_string(),
        provider_session_id: None,
        transcript_ref: None,
        initial_instruction_admitted: false,
    })
}

pub fn retried_delegate_parent_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let root = TreeRootFact {
        repository_root: None,
        workspace_identity: "/repo".into(),
        worktree_path: "/repo".into(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        workflow_name: "retry-parent".into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        definition: Some(WorkflowDefinition {
            entry: "main".into(),
            nodes: vec![
                NodeDefinition {
                    name: "main".into(),
                    completion: NodeCompletion {
                        delegate: Some(SessionDelegate {
                            child: "judge".into(),
                            inputs: Vec::new(),
                            when: Predicate::Ref("child.passed".into()),
                            max_iterations: 2,
                        }),
                        ..NodeCompletion::require_approval()
                    },
                    ..Default::default()
                },
                NodeDefinition {
                    name: "judge".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    };
    let tree = "00000000-0000-4000-8000-000000206300";
    let parent = "00000000-0000-4000-8000-000000206301";
    let next = "00000000-0000-4000-8000-000000206302";
    let child = "00000000-0000-4000-8000-000000206303";
    let mut records = Vec::new();
    let mut push = |id: &str, name: &str, parent: Option<&str>, attempt, fact| {
        let seq = records.len() as i64 + 1;
        records.push(NodeFactRecord {
            meta: NodeFactMeta {
                tree_id: tree.into(),
                node_execution_id: id.into(),
                node_name: name.into(),
                parent_id: parent.map(str::to_string),
                kind: NodeKindName::Session,
                attempt,
            },
            seq,
            timestamp_ms: seq * 1000,
            fact,
        });
    };
    push(
        parent,
        "main",
        None,
        1,
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root)),
            parent: None,
            worktree: None,
        }),
    );
    push(parent, "main", None, 1, session_attached("parent-agent"));
    push(
        parent,
        "main",
        None,
        1,
        NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
    );
    push(
        parent,
        "main",
        None,
        1,
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"round":1,"body":"private"}),
            request_id: None,
        }),
    );
    push(
        parent,
        "main",
        None,
        1,
        NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    );
    push(
        child,
        "judge",
        Some(parent),
        1,
        NodeFact::Started(StartedFact {
            root: None,
            parent: Some(ExecutionParentRef::delegate_child(parent)),
            worktree: None,
        }),
    );
    push(
        child,
        "judge",
        Some(parent),
        1,
        session_attached("judge-agent"),
    );
    push(
        child,
        "judge",
        Some(parent),
        1,
        NodeFact::AgentActivityObserved(AgentActivityObservedFact {
            activity: AgentSessionActivity::Working,
        }),
    );
    push(parent, "main", None, 1, NodeFact::RetryRequested);
    push(
        next,
        "main",
        None,
        2,
        NodeFact::Started(StartedFact {
            root: None,
            parent: None,
            worktree: None,
        }),
    );
    push(next, "main", None, 2, session_attached("next-parent-agent"));
    push(
        child,
        "judge",
        Some(parent),
        1,
        NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
    );
    push(
        child,
        "judge",
        Some(parent),
        1,
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"passed":true,"body":"private"}),
            request_id: None,
        }),
    );
    push(
        child,
        "judge",
        Some(parent),
        1,
        NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    );
    records
}

pub fn scoped_dynamic_fanout_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let definition = WorkflowDefinition {
        entry: "main".into(),
        nodes: vec![
            NodeDefinition {
                name: "main".into(),
                completion: NodeCompletion {
                    delegate: Some(SessionDelegate {
                        child: "groups".into(),
                        inputs: Vec::new(),
                        when: Predicate::Ref("done".into()),
                        max_iterations: 2,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            NodeDefinition {
                name: "groups".into(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("group")],
                    items: Some(ItemsSource::Literal(vec![
                        serde_json::json!(0),
                        serde_json::json!(1),
                    ])),
                }),
                ..Default::default()
            },
            NodeDefinition {
                name: "group".into(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    children: vec![
                        ChildEntry::reference("producer"),
                        ChildEntry::reference("inner"),
                    ],
                }),
                ..Default::default()
            },
            NodeDefinition {
                name: "producer".into(),
                ..Default::default()
            },
            NodeDefinition {
                name: "inner".into(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("judge")],
                    items: Some(ItemsSource::ArtifactField {
                        node: "producer".into(),
                        field_path: FieldPath::from_dotted("items").unwrap(),
                    }),
                }),
                completion: NodeCompletion::require_approval(),
                ..Default::default()
            },
            NodeDefinition {
                name: "judge".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let root = TreeRootFact {
        repository_root: None,
        workspace_identity: "/repo".into(),
        worktree_path: "/repo".into(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        workflow_name: "dynamic-scopes".into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        definition: Some(definition.clone()),
    };
    let id = |index| format!("00000000-0000-4000-8000-{index:012}");
    let mut records = Vec::new();
    let mut push = |index, name: &str, parent: Option<ExecutionParentRef>, fact| {
        let seq = records.len() as i64 + 1;
        records.push(NodeFactRecord {
            meta: NodeFactMeta {
                tree_id: id(206400),
                node_execution_id: id(index),
                node_name: name.into(),
                parent_id: parent.as_ref().map(|parent| parent.parent_id.clone()),
                kind: definition.node_by_name(name).unwrap().kind_name(),
                attempt: 1,
            },
            seq,
            timestamp_ms: seq * 1000,
            fact,
        });
    };
    push(
        206401,
        "main",
        None,
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root)),
            parent: None,
            worktree: None,
        }),
    );
    push(
        206401,
        "main",
        None,
        session_attached("dynamic-parent-agent"),
    );
    push(
        206401,
        "main",
        None,
        NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
    );
    push(
        206401,
        "main",
        None,
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"body":"private"}),
            request_id: None,
        }),
    );
    push(
        206401,
        "main",
        None,
        NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    );
    let parent = ExecutionParentRef::delegate_child(id(206401));
    push(
        206402,
        "groups",
        Some(parent.clone()),
        NodeFact::Started(StartedFact {
            root: None,
            parent: Some(parent),
            worktree: None,
        }),
    );
    for (index, count) in [(0, 2), (1, 1)] {
        let group = 206410 + index;
        let parent = ExecutionParentRef::fanout_child(id(206402), Some(index as usize), 0);
        push(
            group,
            "group",
            Some(parent.clone()),
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(parent),
                worktree: None,
            }),
        );
        let parent = ExecutionParentRef::sequence_child(id(group));
        push(
            206420 + index,
            "producer",
            Some(parent.clone()),
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(parent.clone()),
                worktree: None,
            }),
        );
        for fact in [
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: None,
                value: serde_json::json!({"items": (0..count).collect::<Vec<_>>(), "body":"private"}),
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            push(206420 + index, "producer", Some(parent.clone()), fact);
        }
    }
    let parent = ExecutionParentRef::sequence_child(id(206410));
    push(
        206430,
        "inner",
        Some(parent.clone()),
        NodeFact::Started(StartedFact {
            root: None,
            parent: Some(parent),
            worktree: None,
        }),
    );
    for index in 0..2 {
        let parent = ExecutionParentRef::fanout_child(id(206430), Some(index as usize), 0);
        push(
            206440 + index,
            "judge",
            Some(parent.clone()),
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(parent.clone()),
                worktree: None,
            }),
        );
        for fact in [
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: None,
                value: serde_json::json!({"passed":true,"body":"private"}),
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            push(206440 + index, "judge", Some(parent.clone()), fact);
        }
    }
    records
}
