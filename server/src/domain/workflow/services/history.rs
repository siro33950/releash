//! Workflow history entry construction rules.

use crate::domain::workflow::value_objects::{NodeHistoryEntry, TokenUsage, NODE_STATUS_ABORTED};

pub fn aborted_node_history_entry(
    node_name: String,
    attempt: u32,
    session_id: Option<String>,
    token_usage: TokenUsage,
    timestamp: f64,
) -> NodeHistoryEntry {
    NodeHistoryEntry {
        node_name,
        completed_at: timestamp,
        result: None,
        session_id,
        token_usage: Some(token_usage),
        artifact: None,
        attempt,
        fanout_children: None,
        state: NODE_STATUS_ABORTED.to_string(),
    }
}

#[cfg(test)]
#[path = "history_test.rs"]
mod history_tests;

pub(crate) fn retry_history_chains<'a>(
    nodes: impl IntoIterator<Item = (&'a str, Option<&'a str>, bool)>,
) -> Vec<(String, Vec<String>)> {
    use std::collections::{HashMap, HashSet};
    let nodes: HashMap<_, _> = nodes
        .into_iter()
        .map(|(id, predecessor, terminal)| (id, (predecessor, terminal)))
        .collect();
    let predecessors: HashSet<_> = nodes
        .values()
        .filter_map(|(predecessor, _)| *predecessor)
        .collect();
    let mut chains = Vec::new();
    for (&head, &(predecessor, _)) in &nodes {
        if predecessor.is_none() || predecessors.contains(head) {
            continue;
        }
        let mut chain = Vec::new();
        let mut cursor = predecessor;
        let mut visited = HashSet::new();
        while let Some(id) = cursor {
            if !visited.insert(id) {
                break;
            }
            let Some(&(previous, true)) = nodes.get(id) else {
                break;
            };
            chain.push(id.to_string());
            cursor = previous;
        }
        chain.reverse();
        chains.push((head.to_string(), chain));
    }
    chains
}

pub(crate) fn same_retry_target(
    previous_name: &str,
    previous_parent: Option<&str>,
    name: &str,
    parent: Option<&str>,
) -> bool {
    previous_name == name && previous_parent == parent
}

pub(crate) fn retry_predecessors(
    records: &[crate::domain::workflow::NodeFactRecord],
) -> std::collections::HashMap<String, String> {
    use crate::domain::workflow::NodeFact;
    let mut pending = None;
    let mut predecessors = std::collections::HashMap::new();
    for record in records {
        match record.fact {
            NodeFact::RetryRequested => {
                pending = Some(PendingRestart {
                    node_execution_id: record.meta.node_execution_id.clone(),
                    node_name: record.meta.node_name.clone(),
                    parent_scope_id: record.meta.parent_id.clone(),
                })
            }
            NodeFact::Started(_) => {
                if let Some(previous) = consume_restart(
                    &mut pending,
                    &record.meta.node_name,
                    record.meta.parent_id.as_deref(),
                ) {
                    predecessors.insert(record.meta.node_execution_id.clone(), previous);
                }
            }
            _ => {}
        }
    }
    predecessors
}

pub(crate) fn read_lineage_roots(
    target: &str,
    predecessors: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut roots = vec![target.to_string()];
    let mut visited = std::collections::HashSet::from([target]);
    while let Some(previous) = predecessors.get(roots.last().expect("target")) {
        if !visited.insert(previous) {
            break;
        }
        roots.push(previous.clone());
    }
    roots
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingRestart {
    pub node_execution_id: String,
    pub node_name: String,
    pub parent_scope_id: Option<String>,
}

pub(crate) fn consume_restart(
    pending: &mut Option<PendingRestart>,
    name: &str,
    parent: Option<&str>,
) -> Option<String> {
    pending
        .take()
        .filter(|previous| {
            same_retry_target(
                &previous.node_name,
                previous.parent_scope_id.as_deref(),
                name,
                parent,
            )
        })
        .map(|previous| previous.node_execution_id)
}
