use super::*;
use crate::domain::workflow::{
    ApprovalGrantedFact, ArtifactProducedFact, ExecutionOrigin, ExecutionParentRef,
    ExecutionTreeLaunch, NodeFactMeta, ProcessExitedFact, StartedFact, StopReceivedFact,
    SubmitReceivedFact, WorkflowDefinition,
};

struct Log {
    root: TreeRootFact,
    records: Vec<NodeFactRecord>,
}

impl Log {
    fn new(nodes: &str) -> Self {
        let definition: WorkflowDefinition =
            serde_saphyr::from_str(&format!("name: test\ndescription: test\nnodes:\n{nodes}"))
                .unwrap();
        Self {
            root: TreeRootFact {
                repository_root: Some("/repo".into()),
                workspace_identity: "/repo".into(),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                workflow_name: definition.name.clone(),
                definition: Some(definition),
                launched_as: ExecutionTreeLaunch::Workflow,
            },
            records: Vec::new(),
        }
    }

    fn start(&mut self, id: &str, name: &str, parent: Option<ExecutionParentRef>, attempt: u32) {
        let meta = NodeFactMeta {
            tree_id: "tree".into(),
            node_execution_id: id.into(),
            node_name: name.into(),
            kind: self
                .root
                .definition
                .as_ref()
                .unwrap()
                .node_by_name(name)
                .unwrap()
                .kind_name(),
            parent_id: parent.as_ref().map(|parent| parent.parent_id.clone()),
            attempt,
        };
        let root = parent.is_none().then(|| Box::new(self.root.clone()));
        self.push(
            meta,
            NodeFact::Started(StartedFact {
                worktree: crate::domain::workflow::WorktreeInheritance::new(
                    self.root
                        .definition
                        .as_ref()
                        .unwrap()
                        .node_by_name(name)
                        .unwrap()
                        .worktree,
                )
                .for_attempt(self.root.repository_root.as_deref(), id, attempt)
                .unwrap(),
                parent,
                root,
            }),
        );
    }

    fn push(&mut self, meta: NodeFactMeta, fact: NodeFact) {
        let seq = self.records.len() as i64 + 1;
        self.records.push(NodeFactRecord {
            meta,
            fact,
            seq,
            timestamp_ms: seq * 1000,
        });
    }

    fn fact(&mut self, id: &str, fact: NodeFact) {
        let meta = self
            .records
            .iter()
            .find(|record| record.meta.node_execution_id == id)
            .unwrap()
            .meta
            .clone();
        self.push(meta, fact);
    }

    fn submit(&mut self, id: &str, value: Option<serde_json::Value>) {
        self.fact(
            id,
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
        );
        if let Some(value) = value {
            self.fact(
                id,
                NodeFact::ArtifactProduced(ArtifactProducedFact {
                    contract: None,
                    value,
                    request_id: None,
                }),
            );
        }
    }

    fn stop(&mut self, id: &str) {
        self.fact(
            id,
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        );
    }

    fn output(&self, name: &str) -> Option<Artifact> {
        let result =
            fact_replay::without_tree_fold(|| derive_node_artifact("tree", &self.records, name))
                .unwrap();
        let tree = fact_replay::fold_execution_tree("tree", &self.records)
            .unwrap()
            .unwrap();
        assert_eq!(
            result,
            fact_replay::derive_node_artifact(&tree, &self.records, name)
        );
        result
    }
}

#[test]
fn test_終端の成果_定義なしで子孫を復元し終端後の事実を無視する() {
    for terminal in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(Default::default()),
    ] {
        for keep_definition in [false, true] {
            // Given
            let mut log = Log::new("  main: {sequence: {children: [group, other]}}\n  group: {worktree: isolated, fanout: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}\n  other: {command: true}");
            log.start("main-id", "main", None, 1);
            log.start(
                "group-id",
                "group",
                Some(ExecutionParentRef::sequence_child("main-id")),
                1,
            );
            log.start(
                "work-id",
                "work",
                Some(ExecutionParentRef::fanout_child("group-id", None, 0)),
                1,
            );
            log.submit("work-id", Some(serde_json::json!({"answer": "kept"})));
            log.stop("work-id");
            log.start(
                "other-id",
                "other",
                Some(ExecutionParentRef::sequence_child("main-id")),
                1,
            );
            log.fact("main-id", terminal.clone());
            if !keep_definition {
                let NodeFact::Started(started) = &mut log.records[0].fact else {
                    panic!()
                };
                started.root.as_mut().unwrap().definition = None;
            }
            // When / Then
            let group = log.output("group");
            if keep_definition || matches!(terminal, NodeFact::ExecutionCompleted) {
                let group = group.unwrap();
                assert_eq!(group.value["work"]["answer"], "kept");
                assert_eq!(group.produced_at, if keep_definition { 6.0 } else { 8.0 });
            } else {
                assert!(group.is_none());
            }
            let output = log.output("work").unwrap();
            assert_eq!(output.value["answer"], "kept");
            assert_eq!(
                output.value["worktree"]["branch"],
                "releash/isolated/work-id-a1"
            );
            log.submit("work-id", Some(serde_json::json!({"answer": "late"})));
            assert_eq!(log.output("work").unwrap(), output);
            assert!(log.output("missing").is_none());
        }
    }
}

#[test]
fn test_終端の成果_保存されたcontractを定義なしで保持する() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.submit("main-id", Some(serde_json::json!({"answer": "kept"})));
    let NodeFact::ArtifactProduced(fact) = &mut log.records[2].fact else {
        panic!()
    };
    fact.contract = Some("stored-result".into());
    log.stop("main-id");
    log.fact("main-id", NodeFact::ExecutionCompleted);
    let NodeFact::Started(started) = &mut log.records[0].fact else {
        panic!()
    };
    started.root.as_mut().unwrap().definition = None;
    // When
    let output =
        fact_replay::without_tree_fold(|| derive_node_artifact("tree", &log.records, "main"))
            .unwrap()
            .unwrap();
    // Then
    assert_eq!(output.contract.as_deref(), Some("stored-result"));
    assert_eq!(output.value["answer"], "kept");
}

#[test]
fn test_隔離合成子の成果_sequenceとfanoutが未完了時は未提出で完了後は子とworktreeを返す() {
    for kind in ["sequence", "fanout"] {
        // Given
        let mut log = Log::new(&format!("  main: {{worktree: isolated, {kind}: {{children: [work]}}}}\n  work: {{worktree: isolated, session: {{provider: codex}}}}"));
        log.start("main-id", "main", None, 1);
        let parent = if kind == "sequence" {
            ExecutionParentRef::sequence_child("main-id")
        } else {
            ExecutionParentRef::fanout_child("main-id", None, 0)
        };
        log.start("work-id", "work", Some(parent), 2);
        // When / Then
        assert!(log.output("main").is_none());
        log.submit("work-id", None);
        assert!(log.output("main").is_none());
        log.stop("work-id");
        let output = log.output("main").unwrap();
        assert_eq!(
            output.value["worktree"]["branch"],
            "releash/isolated/main-id-a1"
        );
        assert_eq!(
            output.value["work"]["worktree"]["branch"],
            "releash/isolated/work-id-a2"
        );
        assert_eq!(output.produced_at, 4.0);
    }
}

#[test]
fn test_隔離合成子の成果_入れ子と同名slotを混ぜず最後に完了した成果を選ぶ() {
    // Given
    let mut log = Log::new("  main: {fanout: {items: [x, y], children: [part]}}\n  part: {worktree: isolated, sequence: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    for (index, id) in [(0, "first"), (1, "second")] {
        log.start(
            id,
            "part",
            Some(ExecutionParentRef::fanout_child("main-id", Some(index), 0)),
            1,
        );
        log.start(
            &format!("{id}-work"),
            "work",
            Some(ExecutionParentRef::sequence_child(id)),
            1,
        );
    }
    // When / Then
    for id in ["second", "first"] {
        log.submit(&format!("{id}-work"), None);
        log.stop(&format!("{id}-work"));
        let output = log.output("part").unwrap();
        assert_eq!(
            output.value["worktree"]["branch"],
            format!("releash/isolated/{id}-a1")
        );
        assert_eq!(
            output.value["work"]["worktree"]["branch"],
            format!("releash/isolated/{id}-work-a1")
        );
    }
    assert!(log.output("main").is_some());
}

#[test]
fn test_隔離合成子の成果_承認までは未提出で中止後も確定済みの成果を保持する() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, completion: {require: approval}, sequence: {children: [work]}}\n  work: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("work-id", None);
    log.stop("work-id");
    // When / Then
    assert!(log.output("main").is_none());
    log.fact(
        "main-id",
        NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
    );
    let output = log.output("main").unwrap();
    assert_eq!(output.produced_at, 5.0);
    log.fact("main-id", NodeFact::AbortRequested(Default::default()));
    assert_eq!(log.output("main"), Some(output));
}

#[test]
fn test_隔離合成子の成果_失敗したslotの再試行では旧attemptの成果を混ぜない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, fanout: {children: [work]}}\n  work: {worktree: isolated, command: 'true'}");
    log.start("main-id", "main", None, 1);
    let parent = ExecutionParentRef::fanout_child("main-id", None, 0);
    log.start("old", "work", Some(parent.clone()), 1);
    log.fact(
        "old",
        NodeFact::ProcessExited(ProcessExitedFact {
            failure_kind: None,
            exit_code: Some(1),
            result_summary: None,
            failure_reason: None,
        }),
    );
    assert!(log.output("main").is_none());
    log.fact("old", NodeFact::RetryRequested);
    log.start("new", "work", Some(parent), 2);
    // When / Then
    assert!(log.output("main").is_none());
    log.fact(
        "new",
        NodeFact::ProcessExited(ProcessExitedFact {
            failure_kind: None,
            exit_code: Some(0),
            result_summary: None,
            failure_reason: None,
        }),
    );
    let output = log.output("main").unwrap();
    assert_eq!(
        output.value["work"]["worktree"]["branch"],
        "releash/isolated/new-a2"
    );
}

#[test]
fn test_隔離合成子の成果_途中の中止と欠損したrepository_rootから成果を捏造しない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("work-id", None);
    log.fact("main-id", NodeFact::AbortRequested(Default::default()));
    // When / Then
    assert!(log.output("main").is_none());
    log.records.pop();
    log.stop("work-id");
    let NodeFact::Started(started) = &mut log.records[0].fact else {
        unreachable!()
    };
    started.root.as_mut().unwrap().repository_root = None;
    assert!(derive_node_artifact("tree", &log.records, "main").is_err());
}

#[test]
fn test_隔離合成子の成果_fanoutのprocess終了だけでは成果を生成しない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, fanout: {children: [failed, done]}}\n  failed: {command: 'true'}\n  done: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "failed-id",
        "failed",
        Some(ExecutionParentRef::fanout_child("main-id", None, 0)),
        1,
    );
    log.start(
        "done-id",
        "done",
        Some(ExecutionParentRef::fanout_child("main-id", None, 1)),
        1,
    );
    log.fact(
        "failed-id",
        NodeFact::ProcessExited(ProcessExitedFact {
            failure_kind: None,
            exit_code: Some(1),
            result_summary: None,
            failure_reason: None,
        }),
    );
    log.submit("done-id", None);
    log.stop("done-id");
    // When / Then
    assert!(log.output("main").is_none());
}

#[test]
fn test_隔離合成子の成果_動的itemsは開始時の兄弟成果から件数を得る() {
    // Given
    let mut log = Log::new("  main: {sequence: {children: [produce, part]}}\n  produce: {session: {provider: codex}}\n  part: {worktree: isolated, fanout: {items: produce.items, children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "produce-id",
        "produce",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("produce-id", Some(serde_json::json!({"items": ["x", "y"]})));
    log.stop("produce-id");
    log.start(
        "part-id",
        "part",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    for index in 0..2 {
        let id = format!("work-{index}");
        log.start(
            &id,
            "work",
            Some(ExecutionParentRef::fanout_child("part-id", Some(index), 0)),
            1,
        );
        log.submit(&id, None);
        log.stop(&id);
        if index == 0 {
            assert!(log.output("part").is_none());
        }
    }
    // When / Then
    let output = log.output("part").unwrap();
    for index in 0..2 {
        assert_eq!(
            output.value[index.to_string()]["worktree"]["branch"],
            format!("releash/isolated/work-{index}-a1")
        );
    }
}

#[test]
fn test_隔離合成子の成果_sequenceは未実行のchildを含めず明示終端で完了する() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, sequence: {children: [{work: {rules: []}}, skipped]}}\n  work: {worktree: isolated, session: {provider: codex}}\n  skipped: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("work-id", None);
    log.stop("work-id");
    // When / Then
    let output = log.output("main").unwrap();
    assert!(output.value.get("skipped").is_none());
    assert!(output.value.get("work").is_some());
}

#[test]
fn test_隔離合成子の成果_破損した実行木を成果に置き換えない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {session: {provider: codex}}");
    assert!(derive_node_artifact("tree", &[], "main").unwrap().is_none());
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("work-id", None);
    log.stop("work-id");
    // When / Then
    assert!(derive_node_artifact("other-tree", &log.records, "main").is_err());
    assert!(derive_node_artifact("tree", &log.records[1..], "main").is_err());
    assert!(derive_node_artifact("tree", &log.records[2..], "main").is_err());
    assert!(log.output("missing").is_none());
}

#[test]
fn test_隔離合成子の成果_同じtimestampの遅延事実で完了順を変えない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, fanout: {children: [failed, done]}}\n  failed: {command: 'true'}\n  done: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "failed-id",
        "failed",
        Some(ExecutionParentRef::fanout_child("main-id", None, 0)),
        1,
    );
    log.start(
        "done-id",
        "done",
        Some(ExecutionParentRef::fanout_child("main-id", None, 1)),
        1,
    );
    log.fact(
        "failed-id",
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"ok": false, "exit_code": 1}),
            request_id: None,
        }),
    );
    log.fact(
        "failed-id",
        NodeFact::ProcessExited(ProcessExitedFact {
            failure_kind: None,
            exit_code: Some(0),
            result_summary: None,
            failure_reason: None,
        }),
    );
    log.submit("done-id", None);
    log.stop("done-id");
    log.fact(
        "failed-id",
        NodeFact::ProcessExited(ProcessExitedFact {
            failure_kind: None,
            exit_code: Some(0),
            result_summary: None,
            failure_reason: None,
        }),
    );
    for record in &mut log.records {
        record.timestamp_ms = 1000;
    }
    // When / Then
    assert_eq!(log.output("main").unwrap().produced_at, 1.0);
}

#[test]
fn test_隔離合成子の成果_sequenceのループでは最後のattemptだけを統合する() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, sequence: {children: [work, {again: {rules: [{loop_guard: {max_iterations: 2, on_exhausted: done}}, {next: work}]}}, done]}}\n  work: {worktree: isolated, session: {provider: codex}}\n  again: {session: {provider: codex}}\n  done: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    for attempt in 1..=2 {
        for name in ["work", "again"] {
            let id = format!("{name}-{attempt}");
            log.start(
                &id,
                name,
                Some(ExecutionParentRef::sequence_child("main-id")),
                attempt,
            );
            log.submit(&id, None);
            log.stop(&id);
            assert!(log.output("main").is_none());
        }
    }
    log.start(
        "done-id",
        "done",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("done-id", None);
    log.stop("done-id");
    // When / Then
    let output = log.output("main").unwrap();
    assert_eq!(
        output.value["work"]["worktree"]["branch"],
        "releash/isolated/work-2-a2"
    );
}

#[test]
fn test_隔離合成子の成果_別nodeの事実を挟んだ提出と成果を一対にしない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, fanout: {children: [work, observer]}}\n  work: {worktree: isolated, session: {provider: codex}}\n  observer: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::fanout_child("main-id", None, 0)),
        1,
    );
    log.start(
        "observer-id",
        "observer",
        Some(ExecutionParentRef::fanout_child("main-id", None, 1)),
        1,
    );
    log.stop("work-id");
    log.submit("work-id", None);
    log.submit("observer-id", None);
    log.fact(
        "work-id",
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"late": true}),
            request_id: None,
        }),
    );
    log.stop("observer-id");
    // When / Then
    let output = log.output("main").unwrap();
    assert_eq!(output.value["work"]["late"], true);
    assert_eq!(log.output("work").unwrap().produced_at, 5.0);
}

#[test]
fn test_隔離合成子の成果_合成子完了後の葉の成果更新は確定したmapへ混ぜない() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, sequence: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "work-id",
        "work",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("work-id", None);
    log.stop("work-id");
    let original = log.output("main").unwrap();
    log.fact(
        "work-id",
        NodeFact::ArtifactProduced(ArtifactProducedFact {
            contract: None,
            value: serde_json::json!({"late": true}),
            request_id: None,
        }),
    );
    // When / Then
    assert_eq!(log.output("main"), Some(original));
    assert_eq!(log.output("work").unwrap().value["late"], true);
}

#[test]
fn test_空の隔離fanout_保存事実だけからworktree成果と完了を復元する() {
    // Given
    let mut log = Log::new("  main: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    // When
    let output = log.output("main").unwrap();
    let folded = fact_replay::fold_execution_tree("tree", &log.records)
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        output.value,
        serde_json::json!({"worktree": {"branch": "releash/isolated/main-id-a1", "path": "/repo-worktrees/.releash-isolated/main-id-a1"}})
    );
    assert_eq!(folded.aggregate.node_execution("main-id").unwrap().status, crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Succeeded);
    assert_eq!(
        fact_replay::derive_read_model(&folded).status,
        crate::domain::workflow::ExecutionStatus::Completed
    );
}

#[test]
fn test_空の隔離fanout_承認待ちと生成失敗と中止を完了へ置き換えない() {
    // Given
    for outcome in ["approval", "failure", "abort"] {
        let completion = if outcome != "failure" {
            ", completion: {require: approval}"
        } else {
            ""
        };
        let mut log = Log::new(&format!("  main: {{worktree: isolated, fanout: {{items: [], children: [work]}}{completion}}}\n  work: {{session: {{provider: codex}}}}"));
        log.start("main-id", "main", None, 1);
        // When
        match outcome {
            "failure" => log.fact(
                "main-id",
                NodeFact::RuntimeFailureObserved(
                    crate::domain::workflow::RuntimeFailureObservedFact {
                        reason: "creation failed".into(),
                        failure_kind:
                            crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                    },
                ),
            ),
            "abort" => log.fact("main-id", NodeFact::AbortRequested(Default::default())),
            _ => {}
        }
        // Then
        assert!(log.output("main").is_none());
        if outcome == "approval" {
            log.fact(
                "main-id",
                NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
            );
            assert!(log.output("main").is_some());
        }
    }
}

#[test]
fn test_空の隔離fanout_動的itemsと後続nodeの事実をliveと同じ順で決着する() {
    // Given
    let mut log = Log::new("  main: {sequence: {children: [produce, part, next]}}\n  produce: {session: {provider: codex}}\n  part: {worktree: isolated, fanout: {items: produce.items, children: [work]}}\n  work: {session: {provider: codex}}\n  next: {session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "produce-id",
        "produce",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("produce-id", Some(serde_json::json!({"items": []})));
    log.stop("produce-id");
    log.start(
        "part-id",
        "part",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    // When
    let empty = log.output("part").unwrap();
    log.start(
        "next-id",
        "next",
        Some(ExecutionParentRef::sequence_child("main-id")),
        1,
    );
    log.submit("next-id", None);
    log.stop("next-id");
    // Then
    assert_eq!(log.output("main").unwrap().value["part"], empty.value);
    assert_eq!(log.output("part"), Some(empty));
}

#[test]
fn test_空の隔離fanout_承認なしでも保存済みabortと後続failureを保持する() {
    // Given
    for parent_abort in [false, true] {
        for failure_after_abort in [false, true] {
            let prefix = if parent_abort {
                "  root: {sequence: {children: [main]}}\n"
            } else {
                ""
            };
            let mut log = Log::new(&format!("{prefix}  main: {{worktree: isolated, fanout: {{items: [], children: [work]}}}}\n  work: {{session: {{provider: codex}}}}"));
            let parent = parent_abort.then(|| {
                log.start("root-id", "root", None, 1);
                ExecutionParentRef::sequence_child("root-id")
            });
            log.start("main-id", "main", parent, 1);
            log.fact(
                if parent_abort { "root-id" } else { "main-id" },
                NodeFact::AbortRequested(Default::default()),
            );
            if failure_after_abort {
                log.fact("main-id", NodeFact::RuntimeFailureObserved(crate::domain::workflow::RuntimeFailureObservedFact {
                    reason: "creation failed".into(),
                    failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                }));
            }
            // When
            let output = log.output("main");
            let folded = fact_replay::fold_execution_tree("tree", &log.records)
                .unwrap()
                .unwrap();
            // Then
            assert!(output.is_none());
            assert_eq!(
                fact_replay::derive_read_model(&folded).status,
                crate::domain::workflow::ExecutionStatus::Aborted
            );
            let node = folded.aggregate.node_execution("main-id").unwrap();
            assert!(node.artifact.is_none());
            assert_eq!(
                node.worktree.as_ref().unwrap().branch,
                "releash/isolated/main-id-a1"
            );
        }
    }
}

#[test]
fn test_隔離成果選択_提出順と完了順が逆転しても最後の提出を返す() {
    // Given
    let mut log = Log::new("  main: {fanout: {items: [x, y], children: [work]}}\n  work: {worktree: isolated, session: {provider: codex}}");
    log.start("main-id", "main", None, 1);
    log.start(
        "first",
        "work",
        Some(ExecutionParentRef::fanout_child("main-id", Some(0), 0)),
        2,
    );
    log.start(
        "second",
        "work",
        Some(ExecutionParentRef::fanout_child("main-id", Some(1), 0)),
        1,
    );
    log.submit("first", None);
    log.submit("second", None);
    log.stop("second");
    log.stop("first");
    // When
    let output = log.output("work").unwrap();
    // Then
    assert_eq!(
        output.value["worktree"]["branch"],
        "releash/isolated/second-a1"
    );
    let composite = log.output("main").unwrap();
    assert_eq!(
        composite.value["0"]["worktree"]["branch"],
        "releash/isolated/first-a2"
    );
    assert_eq!(
        composite.value["1"]["worktree"]["branch"],
        "releash/isolated/second-a1"
    );
}

#[test]
fn test_delegate_保存事実の再生は注入前後と同一内容の再提出と上限を保持する() {
    // Given
    let mut log = Log::new("  main: {artifact: result, session: {provider: codex}, completion: {delegate: {child: check, when: child.passed, max_iterations: 2}}}\n  check: {artifact: result, session: {provider: codex}}");
    log.start("parent", "main", None, 1);
    for round in 1..=2 {
        log.submit("parent", Some(serde_json::json!({"done": false})));
        log.stop("parent");
        let id = format!("check-{round}");
        log.start(
            &id,
            "check",
            Some(ExecutionParentRef::delegate_child("parent")),
            round,
        );
        log.submit(
            &id,
            Some(serde_json::json!({"passed": false, "round": round})),
        );
        log.stop(&id);
        // When / Then
        assert_eq!(log.output("main").unwrap().value["child"]["round"], round);
        let tree = fact_replay::fold_execution_tree("tree", &log.records)
            .unwrap()
            .unwrap();
        assert_eq!(
            tree.aggregate
                .pending_delegate_injection("parent")
                .unwrap()
                .child_execution_id,
            id
        );
        assert!(tree.aggregate.derive_pending_advances().is_empty());
        log.fact("parent", NodeFact::DelegateResultInjected(id));
        let tree = fact_replay::fold_execution_tree("tree", &log.records)
            .unwrap()
            .unwrap();
        assert!(tree.aggregate.pending_delegate_injections().is_empty());
        assert_eq!(log.output("main").unwrap().value["child"]["round"], round);
    }
    log.submit("parent", Some(serde_json::json!({"done": false})));
    log.stop("parent");
    let tree = fact_replay::fold_execution_tree("tree", &log.records)
        .unwrap()
        .unwrap();
    assert_eq!(tree.aggregate.node_execution("parent").unwrap().status, crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Succeeded);
    assert_eq!(
        log.output("main").unwrap().value,
        serde_json::json!({"done": false, "child": {"passed": false, "round": 2}})
    );
}

#[test]
fn test_delegate_合成子の配下でも子のmapと後続参照用の成果を局所的に再生する() {
    for kind in ["sequence", "fanout"] {
        // Given
        let mut log = Log::new(&format!("  root: {{sequence: {{children: [main]}}}}\n  main: {{artifact: result, session: {{provider: codex}}, completion: {{delegate: {{child: checks, when: child.check.passed, max_iterations: 2}}}}}}\n  checks: {{{kind}: {{children: [check]}}}}\n  check: {{artifact: result, session: {{provider: codex}}}}"));
        log.start("root", "root", None, 1);
        log.start(
            "parent",
            "main",
            Some(ExecutionParentRef::sequence_child("root")),
            1,
        );
        log.submit("parent", Some(serde_json::json!({"done": false})));
        log.stop("parent");
        log.start(
            "checks",
            "checks",
            Some(ExecutionParentRef::delegate_child("parent")),
            1,
        );
        let parent = if kind == "sequence" {
            ExecutionParentRef::sequence_child("checks")
        } else {
            ExecutionParentRef::fanout_child("checks", None, 0)
        };
        log.start("check", "check", Some(parent), 1);
        // When
        log.submit("check", Some(serde_json::json!({"passed": true})));
        log.stop("check");
        // Then
        let expected = serde_json::json!({"done": false, "child": {"check": {"passed": true}}});
        assert_eq!(log.output("main").unwrap().value, expected);
        assert_eq!(log.output("root").unwrap().value["main"], expected);
    }
}

#[test]
fn test_delegate_childの失敗と手動retryの成果を部分木から再生する() {
    // Given
    let mut log = Log::new("  main: {artifact: result, session: {provider: codex}, completion: {delegate: {child: check, when: child.passed, max_iterations: 2}}}\n  check: {artifact: result, session: {provider: codex}}");
    log.start("parent", "main", None, 1);
    log.submit("parent", Some(serde_json::json!({"done": false})));
    log.stop("parent");
    log.start(
        "first",
        "check",
        Some(ExecutionParentRef::delegate_child("parent")),
        1,
    );
    log.fact(
        "first",
        NodeFact::RuntimeFailureObserved(crate::domain::workflow::RuntimeFailureObservedFact {
            reason: "child failed".into(),
            failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
        }),
    );
    log.fact("first", NodeFact::RetryRequested);
    log.start(
        "retry",
        "check",
        Some(ExecutionParentRef::delegate_child("parent")),
        2,
    );
    // When
    log.submit("retry", Some(serde_json::json!({"passed": false})));
    log.stop("retry");
    // Then
    assert_eq!(
        log.output("main").unwrap().value,
        serde_json::json!({"done": false, "child": {"passed": false}})
    );
    log.fact("parent", NodeFact::DelegateResultInjected("retry".into()));
    assert_eq!(
        log.output("main").unwrap().value["child"],
        serde_json::json!({"passed": false})
    );
}
