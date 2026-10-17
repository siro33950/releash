pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn aborted_node_history_entry_keeps_session_and_token_usage() {
        let entry = aborted_node_history_entry(
            "review".to_string(),
            3,
            Some("session-review".to_string()),
            TokenUsage {
                input_tokens: 5,
                output_tokens: 8,
            },
            12.0,
        );

        assert_eq!(entry.node_name, "review");
        assert_eq!(entry.attempt, 3);
        assert_eq!(entry.session_id.as_deref(), Some("session-review"));
        assert_eq!(entry.token_usage.unwrap().input_tokens, 5);
        assert_eq!(entry.state, "aborted");
    }
}

use super::*;
use crate::domain::workflow::{NodeFact, NodeFactMeta, NodeFactRecord, NodeKindName, StartedFact};

fn fact(id: &str, name: &str, parent: Option<&str>, fact: NodeFact) -> NodeFactRecord {
    NodeFactRecord {
        seq: 1,
        timestamp_ms: 1000,
        meta: NodeFactMeta {
            tree_id: "tree".into(),
            node_execution_id: id.into(),
            node_name: name.into(),
            parent_id: parent.map(str::to_string),
            kind: NodeKindName::Session,
            attempt: 1,
        },
        fact,
    }
}

fn started(id: &str, name: &str, parent: Option<&str>) -> NodeFactRecord {
    fact(
        id,
        name,
        parent,
        NodeFact::Started(StartedFact {
            root: None,
            parent: None,
            worktree: None,
        }),
    )
}

#[test]
fn test_再試行の対応_直後の開始が同名かつ同親の場合だけ消費する() {
    for (name, parent) in [("other", Some("parent")), ("main", Some("other-parent"))] {
        let records = [
            fact("old", "main", Some("parent"), NodeFact::RetryRequested),
            started("unrelated", name, parent),
            started("later", "main", Some("parent")),
        ];
        assert!(retry_predecessors(&records).is_empty());
        assert_eq!(
            read_lineage_roots("later", &retry_predecessors(&records)),
            ["later"]
        );
    }
}

#[test]
fn test_再試行の系譜_複数試行をたどり循環で打ち切る() {
    let records = [
        fact("first", "main", None, NodeFact::RetryRequested),
        started("second", "main", None),
        fact("second", "main", None, NodeFact::RetryRequested),
        started("third", "main", None),
    ];
    assert_eq!(
        read_lineage_roots("third", &retry_predecessors(&records)),
        ["third", "second", "first"]
    );
    let mut cycle = records.to_vec();
    cycle.extend([
        fact("third", "main", None, NodeFact::RetryRequested),
        started("first", "main", None),
    ]);
    assert_eq!(
        read_lineage_roots("third", &retry_predecessors(&cycle)),
        ["third", "second", "first"]
    );
}
