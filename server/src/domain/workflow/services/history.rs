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
