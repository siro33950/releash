use super::*;
use crate::domain::workflow::{
    ChildEntry, ExecutionParentRef, NodeExecutionStatus as S, NodeKind, SequenceSpec,
};

#[test]
fn test_合成子の成果_再試行前の子を除き開始順と最新成果を保つ() {
    let artifacts = [
        serde_json::json!("old"),
        serde_json::json!("other"),
        serde_json::json!("new"),
    ];
    let parent = ExecutionParentRef::sequence_child("scope");
    let children = [
        ScopeChild {
            id: "old",
            name: "a",
            parent: Some(&parent),
            predecessor: None,
            status: S::Aborted,
            artifact: Some(&artifacts[0]),
        },
        ScopeChild {
            id: "other",
            name: "b",
            parent: Some(&parent),
            predecessor: None,
            status: S::Succeeded,
            artifact: Some(&artifacts[1]),
        },
        ScopeChild {
            id: "new",
            name: "a",
            parent: Some(&parent),
            predecessor: Some("old"),
            status: S::Succeeded,
            artifact: Some(&artifacts[2]),
        },
    ];
    assert_eq!(
        current_children(&children)
            .map(|child| child.id)
            .collect::<Vec<_>>(),
        ["other", "new"]
    );
    let node = NodeDefinition {
        name: "scope".into(),
        kind: NodeKind::Sequence(SequenceSpec {
            entry: None,
            children: vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
        }),
        ..Default::default()
    };
    assert_eq!(
        finish_composite(&node, S::Running, false, None, &children).unwrap(),
        Some((
            S::Succeeded,
            Some(serde_json::json!({"a": "new", "b": "other"}))
        ))
    );
}
