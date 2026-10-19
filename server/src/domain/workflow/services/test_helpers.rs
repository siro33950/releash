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

pub fn composite_dynamic_fanout_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let mut records = scoped_dynamic_fanout_facts();
    let NodeFact::Started(started) = &mut records[0].fact else {
        unreachable!()
    };
    let definition = started.root.as_mut().unwrap().definition.as_mut().unwrap();
    definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "producer")
        .unwrap()
        .kind = NodeKind::Sequence(SequenceSpec {
        entry: None,
        children: vec![ChildEntry::reference("maker")],
    });
    definition.nodes.push(NodeDefinition {
        name: "maker".into(),
        ..Default::default()
    });
    let NodeKind::Fanout(fanout) = &mut definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "inner")
        .unwrap()
        .kind
    else {
        unreachable!()
    };
    fanout.items = Some(ItemsSource::ArtifactField {
        node: "producer".into(),
        field_path: FieldPath::from_dotted("maker.items").unwrap(),
    });
    let mut expanded = Vec::new();
    for mut record in records {
        if record.meta.node_name == "producer" {
            let parent = ExecutionParentRef::sequence_child(&record.meta.node_execution_id);
            let mut maker = record.meta.clone();
            maker.node_name = "maker".into();
            maker.node_execution_id = format!("{}-maker", maker.node_execution_id);
            maker.parent_id = Some(parent.parent_id.clone());
            if matches!(record.fact, NodeFact::Started(_)) {
                record.meta.kind = NodeKindName::Sequence;
                expanded.push(record.clone());
                record.meta = maker;
                record.fact = NodeFact::Started(StartedFact {
                    root: None,
                    parent: Some(parent),
                    worktree: None,
                });
            } else {
                record.meta = maker;
            }
        }
        expanded.push(record);
    }
    for (index, record) in expanded.iter_mut().enumerate() {
        record.seq = index as i64 + 1;
        record.timestamp_ms = record.seq * 1000;
    }
    expanded
}

pub fn composite_routing_facts(
    fanout: bool,
    switch: bool,
) -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let mut records = composite_dynamic_fanout_facts();
    let NodeFact::Started(started) = &mut records[0].fact else {
        unreachable!()
    };
    let definition = started.root.as_mut().unwrap().definition.as_mut().unwrap();
    let NodeKind::Sequence(group) = &mut definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "group")
        .unwrap()
        .kind
    else {
        unreachable!()
    };
    group.children[0].rules = Some(vec![if switch {
        Rule::Switch {
            on: "maker.verdict".into(),
            cases: [
                ("READY".into(), "inner".into()),
                ("HOLD".into(), "finished".into()),
            ]
            .into(),
            next: None,
        }
    } else {
        Rule::When {
            on: Predicate::Ref("maker.passed".into()),
            then: "inner".into(),
            next: "finished".into(),
        }
    }]);
    if fanout {
        definition
            .nodes
            .iter_mut()
            .find(|node| node.name == "producer")
            .unwrap()
            .kind = NodeKind::Fanout(FanoutSpec {
            items: None,
            children: vec![ChildEntry::reference("maker")],
        });
    }
    for record in &mut records {
        if record.meta.node_name == "maker" {
            if fanout {
                if let NodeFact::Started(started) = &mut record.fact {
                    started.parent = Some(ExecutionParentRef::fanout_child(
                        record.meta.parent_id.as_ref().unwrap(),
                        None,
                        0,
                    ));
                }
            }
            if let NodeFact::ArtifactProduced(artifact) = &mut record.fact {
                artifact.value["verdict"] = serde_json::json!("READY");
                artifact.value["passed"] = serde_json::json!(true);
            }
        } else if fanout && record.meta.node_name == "producer" {
            record.meta.kind = NodeKindName::Fanout;
        }
    }
    records
}

pub fn delegated_routing_facts(switch: bool) -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let mut records = composite_routing_facts(false, switch);
    let NodeFact::Started(started) = &mut records[0].fact else {
        unreachable!()
    };
    let definition = started.root.as_mut().unwrap().definition.as_mut().unwrap();
    let group = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "group")
        .unwrap();
    let NodeKind::Sequence(group) = &mut group.kind else {
        unreachable!()
    };
    match &mut group.children[0].rules.as_mut().unwrap()[0] {
        Rule::Switch { on, .. } => *on = "child.verdict".into(),
        Rule::When { on, .. } => *on = Predicate::Ref("child.passed".into()),
        _ => unreachable!(),
    }
    let producer = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "producer")
        .unwrap();
    producer.kind = NodeKind::Session(Default::default());
    producer.completion.delegate = Some(SessionDelegate {
        child: "maker".into(),
        inputs: Vec::new(),
        max_iterations: 2,
        when: Predicate::Ref("child.passed".into()),
    });
    let inner = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "inner")
        .unwrap();
    let NodeKind::Fanout(inner) = &mut inner.kind else {
        unreachable!()
    };
    inner.items = Some(ItemsSource::ArtifactField {
        node: "producer".into(),
        field_path: FieldPath::from_dotted("child.items").unwrap(),
    });
    let mut expanded = Vec::new();
    for mut record in records {
        if record.meta.node_name == "producer" && matches!(record.fact, NodeFact::Started(_)) {
            record.meta.kind = NodeKindName::Session;
            expanded.push(record.clone());
            for fact in [
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                NodeFact::ArtifactProduced(ArtifactProducedFact {
                    value: serde_json::json!({"body":"private-worker"}),
                    contract: None,
                    request_id: None,
                }),
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
            ] {
                let mut submission = record.clone();
                submission.fact = fact;
                expanded.push(submission);
            }
            continue;
        }
        if record.meta.node_name == "maker" {
            if let NodeFact::Started(started) = &mut record.fact {
                started.parent = Some(ExecutionParentRef::delegate_child(
                    record.meta.parent_id.as_ref().unwrap(),
                ));
            }
        }
        expanded.push(record);
    }
    for (index, record) in expanded.iter_mut().enumerate() {
        record.seq = index as i64 + 1;
        record.timestamp_ms = record.seq * 1000;
    }
    expanded
}

pub fn stop_first_delegate_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    let mut records = retried_delegate_parent_facts();
    let end = records.len();
    records[end - 3..].rotate_right(1);
    for (index, record) in records.iter_mut().enumerate() {
        record.seq = index as i64 + 1;
        record.timestamp_ms = record.seq * 1000;
    }
    records
}

pub fn session_timestamp_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let command = || {
        NodeKind::Command(CommandSpec {
            command: "true".into(),
            env: Default::default(),
        })
    };
    let definition = WorkflowDefinition {
        entry: "outer".into(),
        nodes: vec![
            NodeDefinition {
                name: "outer".into(),
                kind: NodeKind::Fanout(FanoutSpec {
                    items: None,
                    children: vec![ChildEntry::reference("main"), ChildEntry::reference("side")],
                }),
                ..Default::default()
            },
            NodeDefinition {
                name: "main".into(),
                completion: NodeCompletion {
                    delegate: Some(SessionDelegate {
                        child: "verify".into(),
                        inputs: vec![],
                        max_iterations: 2,
                        when: Predicate::Ref("child.done".into()),
                    }),
                    ..NodeCompletion::require_approval()
                },
                ..Default::default()
            },
            NodeDefinition {
                name: "verify".into(),
                kind: command(),
                ..Default::default()
            },
            NodeDefinition {
                name: "side".into(),
                kind: command(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let tree = "00000000-0000-4000-8000-000000206500";
    let id = |name: &str| format!("node-{name}");
    let outer = ExecutionParentRef::fanout_child(id("outer"), None, 0);
    let side = ExecutionParentRef::fanout_child(id("outer"), None, 1);
    let delegate = ExecutionParentRef::delegate_child(id("main"));
    let root = TreeRootFact {
        repository_root: None,
        workspace_identity: "/repo".into(),
        worktree_path: "/repo".into(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        workflow_name: "timestamp".into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        definition: Some(definition.clone()),
    };
    let start = |parent| {
        NodeFact::Started(StartedFact {
            parent,
            root: None,
            worktree: None,
        })
    };
    let artifact = |value| {
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            value,
            contract: None,
            request_id: None,
        })
    };
    let exit = |code| {
        NodeFact::ProcessExited(ProcessExitedFact {
            exit_code: Some(code),
            result_summary: None,
            failure_reason: None,
            failure_kind: None,
        })
    };
    let approval = || NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None });
    let facts = vec![
        (
            "outer",
            None,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(root)),
                parent: None,
                worktree: None,
            }),
        ),
        ("main", Some(outer.clone()), start(Some(outer.clone()))),
        ("main", Some(outer.clone()), session_attached("main-agent")),
        (
            "main",
            Some(outer.clone()),
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
        ),
        (
            "main",
            Some(outer.clone()),
            artifact(serde_json::json!({"body":"private-main"})),
        ),
        (
            "main",
            Some(outer.clone()),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ),
        (
            "verify",
            Some(delegate.clone()),
            start(Some(delegate.clone())),
        ),
        ("side", Some(side.clone()), start(Some(side.clone()))),
        (
            "side",
            Some(side.clone()),
            artifact(serde_json::from_str(r#"{"body":"private-first","a":1,"b":2}"#).unwrap()),
        ),
        (
            "side",
            Some(side.clone()),
            artifact(serde_json::from_str(r#"{"b":2,"a":1,"body":"private-first"}"#).unwrap()),
        ),
        (
            "side",
            Some(side.clone()),
            artifact(serde_json::json!({"body":"private-next"})),
        ),
        (
            "side",
            Some(side.clone()),
            artifact(serde_json::json!(true)),
        ),
        ("side", Some(side.clone()), artifact(serde_json::json!(1))),
        ("side", Some(side.clone()), artifact(serde_json::json!(1.0))),
        ("verify", Some(delegate.clone()), exit(1)),
        ("main", Some(outer.clone()), approval()),
        ("side", Some(side.clone()), exit(1)),
        ("main", Some(outer.clone()), exit(0)),
        (
            "verify",
            Some(delegate.clone()),
            artifact(serde_json::json!({"done":true,"body":"private-child"})),
        ),
        ("verify", Some(delegate.clone()), exit(0)),
        ("main", Some(outer.clone()), approval()),
        ("main", Some(outer), approval()),
    ];
    facts
        .into_iter()
        .enumerate()
        .map(|(index, (name, parent, fact))| {
            let seq = index as i64 + 1;
            NodeFactRecord {
                seq,
                timestamp_ms: seq * 1000,
                fact,
                meta: NodeFactMeta {
                    tree_id: tree.into(),
                    node_execution_id: id(name),
                    node_name: name.into(),
                    parent_id: parent.map(|parent| parent.parent_id),
                    kind: definition.node_by_name(name).unwrap().kind_name(),
                    attempt: 1,
                },
            }
        })
        .collect()
}

pub fn sibling_sequence_timestamp_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let mut records = session_timestamp_facts();
    records.truncate(14);
    let NodeFact::Started(started) = &mut records[0].fact else {
        unreachable!()
    };
    let definition = started.root.as_mut().unwrap().definition.as_mut().unwrap();
    let mut leaf = definition
        .nodes
        .iter()
        .find(|node| node.name == "side")
        .unwrap()
        .clone();
    leaf.name = "side-leaf".into();
    definition.nodes.push(leaf);
    let side = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "side")
        .unwrap();
    side.kind = NodeKind::Sequence(SequenceSpec {
        entry: None,
        children: vec![ChildEntry::reference("side-leaf")],
    });
    side.completion = NodeCompletion::require_approval();
    let mut expanded = Vec::new();
    let mut side_meta = None;
    let parent = ExecutionParentRef::sequence_child("node-side");
    for mut record in records {
        if record.meta.node_name == "side" {
            if matches!(record.fact, NodeFact::Started(_)) {
                record.meta.kind = NodeKindName::Sequence;
                side_meta = Some(record.meta.clone());
                expanded.push(record.clone());
                record.fact = NodeFact::Started(StartedFact {
                    root: None,
                    parent: Some(parent.clone()),
                    worktree: None,
                });
            }
            record.meta.kind = NodeKindName::Command;
            record.meta.node_name = "side-leaf".into();
            record.meta.node_execution_id = "node-side-leaf".into();
            record.meta.parent_id = Some("node-side".into());
        }
        expanded.push(record);
    }
    let side = side_meta.unwrap();
    let mut leaf = side.clone();
    leaf.node_execution_id = "node-side-leaf".into();
    leaf.node_name = "side-leaf".into();
    leaf.parent_id = Some("node-side".into());
    leaf.kind = NodeKindName::Command;
    expanded.extend([
        NodeFactRecord {
            meta: leaf,
            seq: 0,
            timestamp_ms: 0,
            fact: NodeFact::ProcessExited(ProcessExitedFact {
                exit_code: Some(0),
                result_summary: None,
                failure_reason: None,
                failure_kind: None,
            }),
        },
        NodeFactRecord {
            meta: side,
            seq: 0,
            timestamp_ms: 0,
            fact: NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
        },
    ]);
    for (index, record) in expanded.iter_mut().enumerate() {
        record.seq = index as i64 + 1;
        record.timestamp_ms = record.seq * 1000;
    }
    expanded
}

pub fn isolated_session_timestamp_facts() -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let tree = "00000000-0000-4000-8000-000000206600";
    let node = "00000000-0000-4000-8000-000000206601";
    let worktree = IsolatedWorktree::for_attempt("/repo", node, 1);
    let first = serde_json::json!({"body": "private-first", "verdict": "READY"});
    let changed = serde_json::json!({"body": "private-next", "verdict": "READY"});
    let accepted = worktree.with_artifact(Some(changed.clone()));
    let artifact = |value| {
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            value,
            contract: None,
            request_id: None,
        })
    };
    let root = TreeRootFact {
        repository_root: Some("/repo".into()),
        workspace_identity: "/repo".into(),
        worktree_path: "/repo".into(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        workflow_name: "isolated-time".into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        definition: Some(WorkflowDefinition {
            entry: "session".into(),
            nodes: vec![NodeDefinition {
                name: "session".into(),
                worktree: Some(WorktreeMode::Isolated),
                ..Default::default()
            }],
            ..Default::default()
        }),
    };
    [
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root)),
            parent: None,
            worktree: Some(worktree),
        }),
        session_attached("isolated-agent"),
        NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
        artifact(first.clone()),
        NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
        artifact(first),
        artifact(changed),
        artifact(accepted.clone()),
        artifact(accepted),
        NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, fact)| {
        let seq = index as i64 + 1;
        NodeFactRecord {
            seq,
            timestamp_ms: seq * 1000,
            fact,
            meta: NodeFactMeta {
                tree_id: tree.into(),
                node_execution_id: node.into(),
                node_name: "session".into(),
                kind: NodeKindName::Session,
                parent_id: None,
                attempt: 1,
            },
        }
    })
    .collect()
}

pub fn isolated_delegate_facts(submitted: bool) -> Vec<crate::domain::workflow::NodeFactRecord> {
    use crate::domain::workflow::*;
    let mut records = retried_delegate_parent_facts();
    records.retain(|record| {
        record.meta.attempt == 1
            && !matches!(record.fact, NodeFact::RetryRequested)
            && (submitted
                || record.meta.node_name != "judge"
                || !matches!(record.fact, NodeFact::ArtifactProduced(_)))
    });
    let NodeFact::Started(started) = &mut records[0].fact else {
        unreachable!()
    };
    let root = started.root.as_mut().unwrap();
    root.repository_root = Some("/repo".into());
    let definition = root.definition.as_mut().unwrap();
    let main = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "main")
        .unwrap();
    main.artifact = Some("result".into());
    main.completion = NodeCompletion {
        delegate: Some(SessionDelegate {
            child: "judge".into(),
            inputs: vec![],
            when: if submitted {
                Predicate::And(vec![
                    Predicate::Ref("done".into()),
                    Predicate::Ref("child.passed".into()),
                ])
            } else {
                Predicate::Ref("done".into())
            },
            max_iterations: 1,
        }),
        ..Default::default()
    };
    definition.schemas.insert(
        "result".into(),
        SchemaDef::Object {
            properties: [("done".into(), SchemaDef::Boolean)].into(),
            required: ["done".into()].into(),
        },
    );
    let judge = definition
        .nodes
        .iter_mut()
        .find(|node| node.name == "judge")
        .unwrap();
    judge.worktree = Some(WorktreeMode::Isolated);
    if submitted {
        judge.artifact = Some("judgment".into());
        definition.schemas.insert(
            "judgment".into(),
            SchemaDef::Object {
                properties: [("passed".into(), SchemaDef::Boolean)].into(),
                required: ["passed".into()].into(),
            },
        );
    }
    for (index, record) in records.iter_mut().enumerate() {
        if let NodeFact::ArtifactProduced(artifact) = &mut record.fact {
            if record.meta.node_name == "main" {
                artifact.value = serde_json::json!({"done": false});
            }
        }
        if record.meta.node_name == "judge" {
            if let NodeFact::Started(started) = &mut record.fact {
                started.worktree = Some(IsolatedWorktree::for_attempt(
                    "/repo",
                    &record.meta.node_execution_id,
                    1,
                ));
            }
        }
        record.seq = index as i64 + 1;
        record.timestamp_ms = record.seq * 1000;
    }
    records
}
