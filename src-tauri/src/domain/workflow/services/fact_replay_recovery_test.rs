use super::*;
use crate::domain::workflow::entities::workflow_execution::{
    ExecutionAdvanceDecision, PendingAdvance,
};

fn recovery_root(nodes: Vec<NodeDefinition>, entry: &str, unavailable: &str) -> TreeRootFact {
    let mut root = workflow_root(workflow_definition(nodes, entry));
    root.definition_resolution
        .node_errors
        .insert(unavailable.into(), "unsupported field".into());
    root
}

fn root_log(root: TreeRootFact, kind: NodeKindName) -> FactLog {
    let entry = root.definition.entry.clone();
    let mut log = FactLog::new();
    log.push(meta(TREE, None, &entry, kind, 1), started_root(root));
    log
}

fn start(
    log: &mut FactLog,
    id: &str,
    name: &str,
    kind: NodeKindName,
    parent: ExecutionParentRef,
) -> NodeFactMeta {
    let meta = meta(id, Some(&parent.parent_id), name, kind, 1);
    log.push(meta.clone(), started_child(parent));
    meta
}

fn complete_command(log: &mut FactLog, meta: NodeFactMeta) {
    log.push(
        meta.clone(),
        artifact(
            "result",
            serde_json::json!({"stdout":"kept", "stderr":"", "exit_code":0, "duration":1}),
        ),
    );
    log.push(meta, exited(0));
}

#[test]
fn test_部分復元_未実行の未対応定義は正常な木の完了を妨げない() {
    // Given
    let root = recovery_root(
        vec![
            sequence_node("main", vec![ChildEntry::reference("cmd")]),
            command_leaf("cmd"),
        ],
        "main",
        "unused",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, cmd);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
    assert_eq!(
        node_status(&folded, "cmd-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
}

#[test]
fn test_部分復元_親の定義が未対応でもsession接続とcommand出力を保持する() {
    // Given
    let root = recovery_root(
        vec![session_leaf("session"), command_leaf("cmd")],
        "main",
        "main",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, cmd);
    let session = start(
        &mut log,
        "session-1",
        "session",
        NodeKindName::Session,
        ExecutionParentRef::sequence_child(TREE),
    );
    log.push(session.clone(), attached("session-id"));
    log.push(session.clone(), submit());
    log.push(session, stop());

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(
        node_status(&folded, TREE),
        RuntimeNodeExecutionStatus::Unresolved
    );
    assert_eq!(
        node_status(&folded, "cmd-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        node_status(&folded, "session-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        folded
            .aggregate
            .node_execution("session-1")
            .unwrap()
            .session_id
            .as_deref(),
        Some("session-id")
    );
    assert_eq!(
        folded
            .aggregate
            .node_execution("cmd-1")
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["stdout"],
        "kept"
    );
    assert!(folded.aggregate.derive_pending_advances().is_empty());
}

#[test]
fn test_部分復元_過去の未対応nodeより後の正常なnodeは前進できる() {
    // Given
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("old"),
                    ChildEntry::reference("current"),
                    ChildEntry::reference("next"),
                ],
            ),
            command_leaf("current"),
            command_leaf("next"),
        ],
        "main",
        "old",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    start(
        &mut log,
        "old-1",
        "old",
        NodeKindName::Sequence,
        ExecutionParentRef::sequence_child(TREE),
    );
    let current = start(
        &mut log,
        "current-1",
        "current",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, current);

    // When
    let mut folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
    let pending = folded.aggregate.derive_pending_advances();
    assert_eq!(pending.len(), 1);
    let advance = folded
        .aggregate
        .apply_pending_advance(&pending[0], &mut || "next-1".into(), 10.0)
        .unwrap();

    // Then
    let ExecutionAdvanceDecision::StartLeaves(leaves) = advance.decision else {
        panic!("the healthy successor must start")
    };
    assert_eq!(leaves[0].node_name, "next");
    assert_eq!(
        folded
            .aggregate
            .node_executions
            .iter()
            .filter(|node| node.node_name == "old")
            .count(),
        1
    );
    assert_ne!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
}

#[test]
fn test_部分復元_未対応の次nodeは未起動のまま前進を制限する() {
    // Given
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("cmd"),
                    ChildEntry::reference("unknown"),
                ],
            ),
            command_leaf("cmd"),
        ],
        "main",
        "unknown",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, cmd);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(
        node_status(&folded, "cmd-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        node_status(&folded, TREE),
        RuntimeNodeExecutionStatus::Unresolved
    );
    assert_eq!(folded.aggregate.node_executions.len(), 2);
    assert!(folded.aggregate.derive_pending_advances().is_empty());
}

#[test]
fn test_部分復元_fanoutの未対応nodeが開始済みでも他のslotを展開できる() {
    // Given
    let root = recovery_root(
        vec![
            fanout_node(
                "main",
                vec![
                    ChildEntry::reference("unknown"),
                    ChildEntry::reference("cmd"),
                ],
            ),
            command_leaf("cmd"),
        ],
        "main",
        "unknown",
    );
    let mut log = root_log(root, NodeKindName::Fanout);
    start(
        &mut log,
        "unknown-1",
        "unknown",
        NodeKindName::Command,
        ExecutionParentRef::fanout_child(TREE, None, 0),
    );

    // When
    let mut folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
    let pending = folded.aggregate.derive_pending_advances();
    assert!(matches!(
        pending.as_slice(),
        [PendingAdvance::ExpandFanout { .. }]
    ));
    let advance = folded
        .aggregate
        .apply_pending_advance(&pending[0], &mut || "cmd-1".into(), 10.0)
        .unwrap();

    // Then
    let ExecutionAdvanceDecision::StartLeaves(leaves) = advance.decision else {
        panic!("the healthy slot must start")
    };
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node_name, "cmd");
    assert_eq!(
        folded
            .aggregate
            .node_executions
            .iter()
            .filter(|node| node.node_name == "unknown")
            .count(),
        1
    );
    assert_ne!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
}

#[test]
fn test_部分復元_未対応nodeをignoreで成功扱いにせず保存成果は保持する() {
    // Given
    let mut unknown = ChildEntry::reference("unknown");
    unknown.on_failure = Some(OnFailure::Ignore);
    let root = recovery_root(
        vec![
            fanout_node("main", vec![unknown, ChildEntry::reference("cmd")]),
            command_leaf("cmd"),
        ],
        "main",
        "unknown",
    );
    let mut log = root_log(root, NodeKindName::Fanout);
    let unknown = start(
        &mut log,
        "unknown-1",
        "unknown",
        NodeKindName::Command,
        ExecutionParentRef::fanout_child(TREE, None, 0),
    );
    log.push(
        unknown.clone(),
        artifact("result", serde_json::json!({"raw":"preserved"})),
    );
    log.push(unknown, exited(1));
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::fanout_child(TREE, None, 1),
    );
    complete_command(&mut log, cmd);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_ne!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
    assert!(!folded
        .aggregate
        .node_execution("unknown-1")
        .unwrap()
        .can_retry());
    assert_eq!(
        folded
            .aggregate
            .node_execution("unknown-1")
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["raw"],
        "preserved"
    );
}

#[test]
fn test_部分復元_過去の未対応nodeの成果を必要とする後続だけを制限する() {
    use crate::domain::workflow::value_objects::InputSourceRef;
    use crate::domain::workflow::InputParam;
    // Given
    let mut next = ChildEntry::reference("next");
    next.inputs
        .push(("previous".into(), InputSourceRef::new("old")));
    let mut consumer = command_leaf("next");
    consumer.input.push(InputParam {
        name: "previous".into(),
        contract: None,
    });
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("old"),
                    ChildEntry::reference("current"),
                    next,
                ],
            ),
            command_leaf("current"),
            consumer,
        ],
        "main",
        "old",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    start(
        &mut log,
        "old-1",
        "old",
        NodeKindName::Sequence,
        ExecutionParentRef::sequence_child(TREE),
    );
    let current = start(
        &mut log,
        "current-1",
        "current",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, current);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(
        node_status(&folded, "current-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        node_status(&folded, TREE),
        RuntimeNodeExecutionStatus::Unresolved
    );
    assert!(folded
        .aggregate
        .node_execution(TREE)
        .unwrap()
        .recovery_reason
        .as_ref()
        .unwrap()
        .contains("next.previous"));
    assert!(folded.aggregate.derive_pending_advances().is_empty());
    assert!(!folded
        .aggregate
        .node_executions
        .iter()
        .any(|node| node.node_name == "next"));
}

#[test]
fn test_部分復元_未対応nodeの保存成果を取得できれば後続inputを束縛できる() {
    use crate::domain::workflow::value_objects::InputSourceRef;
    use crate::domain::workflow::InputParam;
    // Given
    let mut next = ChildEntry::reference("next");
    next.inputs
        .push(("previous".into(), InputSourceRef::new("old.value")));
    let mut consumer = command_leaf("next");
    consumer.input.push(InputParam {
        name: "previous".into(),
        contract: None,
    });
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("old"),
                    ChildEntry::reference("current"),
                    next,
                ],
            ),
            command_leaf("current"),
            consumer,
        ],
        "main",
        "old",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    let old = start(
        &mut log,
        "old-1",
        "old",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    log.push(
        old,
        artifact("result", serde_json::json!({"value":"stored"})),
    );
    let current = start(
        &mut log,
        "current-1",
        "current",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, current);

    // When
    let mut folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
    let pending = folded.aggregate.derive_pending_advances();
    assert_eq!(pending.len(), 1);
    let advance = folded
        .aggregate
        .apply_pending_advance(&pending[0], &mut || "next-1".into(), 10.0)
        .unwrap();

    // Then
    let ExecutionAdvanceDecision::StartLeaves(leaves) = advance.decision else {
        panic!("stored input must be usable")
    };
    assert_eq!(
        leaves[0].bindings,
        vec![("previous".into(), serde_json::json!("stored"))]
    );
}

#[test]
fn test_部分復元_fanoutのitemsを復元できなくても開始済みの子は保持する() {
    use crate::domain::workflow::{FieldPath, ItemsSource};
    // Given
    let mut fanout = fanout_node("fan", vec![ChildEntry::reference("cmd")]);
    let NodeKind::Fanout(spec) = &mut fanout.kind else {
        panic!()
    };
    spec.items = Some(ItemsSource::ArtifactField {
        node: "old".into(),
        field_path: FieldPath::from_reference("old.items").unwrap().1,
    });
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![ChildEntry::reference("old"), ChildEntry::reference("fan")],
            ),
            fanout,
            command_leaf("cmd"),
        ],
        "main",
        "old",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    start(
        &mut log,
        "old-1",
        "old",
        NodeKindName::Sequence,
        ExecutionParentRef::sequence_child(TREE),
    );
    start(
        &mut log,
        "fan-1",
        "fan",
        NodeKindName::Fanout,
        ExecutionParentRef::sequence_child(TREE),
    );
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::fanout_child("fan-1", Some(2), 0),
    );
    complete_command(&mut log, cmd);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(
        node_status(&folded, "fan-1"),
        RuntimeNodeExecutionStatus::Unresolved
    );
    assert_eq!(
        node_status(&folded, "cmd-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        folded
            .aggregate
            .node_execution("cmd-1")
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()["stdout"],
        "kept"
    );
    assert!(folded.aggregate.derive_pending_advances().is_empty());
}

#[test]
fn test_部分復元_未対応nodeの成果を展開元にするfanoutは起動前に制限する() {
    use crate::domain::workflow::{FieldPath, ItemsSource};
    // Given
    let mut fanout = fanout_node("fan", vec![ChildEntry::reference("cmd")]);
    let NodeKind::Fanout(spec) = &mut fanout.kind else {
        panic!()
    };
    spec.items = Some(ItemsSource::ArtifactField {
        node: "old".into(),
        field_path: FieldPath::from_reference("old.items").unwrap().1,
    });
    let root = recovery_root(
        vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("old"),
                    ChildEntry::reference("current"),
                    ChildEntry::reference("fan"),
                ],
            ),
            command_leaf("current"),
            fanout,
            command_leaf("cmd"),
        ],
        "main",
        "old",
    );
    let mut log = root_log(root, NodeKindName::Sequence);
    start(
        &mut log,
        "old-1",
        "old",
        NodeKindName::Sequence,
        ExecutionParentRef::sequence_child(TREE),
    );
    let current = start(
        &mut log,
        "current-1",
        "current",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    complete_command(&mut log, current);

    // When
    let folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

    // Then
    assert_eq!(
        node_status(&folded, "current-1"),
        RuntimeNodeExecutionStatus::Succeeded
    );
    assert_eq!(
        node_status(&folded, TREE),
        RuntimeNodeExecutionStatus::Unresolved
    );
    assert!(folded
        .aggregate
        .node_execution(TREE)
        .unwrap()
        .recovery_reason
        .as_ref()
        .unwrap()
        .contains("old.items"));
    assert!(folded.aggregate.derive_pending_advances().is_empty());
    assert!(!folded
        .aggregate
        .node_executions
        .iter()
        .any(|node| node.node_name == "fan"));
}

#[test]
fn test_部分復元_未対応の親からinputを復元できないcommandのretryはattemptを増やさない() {
    use crate::domain::workflow::entities::workflow_execution::NodeRestartMode;
    use crate::domain::workflow::InputParam;
    // Given
    let mut command = command_leaf("cmd");
    command.input.push(InputParam {
        name: "value".into(),
        contract: None,
    });
    let root = recovery_root(vec![command], "main", "main");
    let mut log = root_log(root, NodeKindName::Sequence);
    let cmd = start(
        &mut log,
        "cmd-1",
        "cmd",
        NodeKindName::Command,
        ExecutionParentRef::sequence_child(TREE),
    );
    log.push(cmd, exited(1));
    let mut folded = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
    let original = folded.aggregate.node_executions.clone();

    // When
    let retry = folded.aggregate.restart_node_attempt_at(
        "cmd-1",
        "cmd-2".into(),
        10.0,
        NodeRestartMode::ExplicitRetry,
    );

    // Then
    assert!(retry.is_none());
    assert_eq!(folded.aggregate.node_executions, original);
    assert!(folded
        .aggregate
        .leaf_start_for("cmd-1")
        .unwrap_err()
        .to_string()
        .contains("main"));
}
