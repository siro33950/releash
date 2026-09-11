use std::collections::{HashMap, HashSet};

use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
use crate::domain::workflow::{
    Artifact, ItemsSource, NodeDefinition, NodeFact, NodeFactRecord, TreeRootFact,
};

use super::{fact_replay, reference};

struct NodeArtifact {
    node: RuntimeNodeExecution,
    settled_seq: i64,
}

struct ArtifactQuery<'a> {
    root: &'a TreeRootFact,
    starts: Vec<&'a NodeFactRecord>,
    by_node: HashMap<&'a str, Vec<&'a NodeFactRecord>>,
    children: HashMap<&'a str, Vec<&'a NodeFactRecord>>,
    aborts: Vec<&'a NodeFactRecord>,
    submitted_artifact_sequences: HashSet<i64>,
}

pub fn derive_node_artifact(
    tree_id: &str,
    records: &[NodeFactRecord],
    node_name: &str,
) -> Result<Option<Artifact>, String> {
    let Some(query) = ArtifactQuery::new(tree_id, records)? else {
        return Ok(None);
    };
    let Some(definition) = query.root.definition.node_by_name(node_name) else {
        return Ok(None);
    };
    let mut selected = None;
    for start in query
        .starts
        .iter()
        .filter(|start| start.meta.node_name == node_name)
    {
        if let Some(output) = query.node_artifact(start, i64::MAX)? {
            let timestamp = output.node.completed_at.unwrap_or(output.node.started_at);
            if let Some(value) = output.node.artifact {
                let submitted = query.by_node[start.meta.node_execution_id.as_str()]
                    .iter()
                    .rev()
                    .find(|record| {
                        matches!(
                            record.fact,
                            NodeFact::SubmitReceived(_) | NodeFact::ArtifactProduced(_)
                        )
                    })
                    .map(|record| record.seq);
                if selected
                    .as_ref()
                    .is_none_or(|(previous, previous_timestamp, _)| {
                        submitted
                            .cmp(previous)
                            .then_with(|| timestamp.total_cmp(previous_timestamp))
                            .is_ge()
                    })
                {
                    selected = Some((submitted, timestamp, value));
                }
            }
        }
    }
    Ok(selected.map(|(_, produced_at, value)| Artifact {
        node_name: node_name.to_string(),
        contract: definition.artifact.clone(),
        value,
        produced_at,
    }))
}

impl<'a> ArtifactQuery<'a> {
    fn new(tree_id: &str, records: &'a [NodeFactRecord]) -> Result<Option<Self>, String> {
        let Some(first) = records.first() else {
            return Ok(None);
        };
        let NodeFact::Started(started) = &first.fact else {
            return Err(format!("tree {tree_id} does not begin with a started fact"));
        };
        let Some(root) = started.root.as_deref() else {
            return Err(format!(
                "tree {tree_id} root started carries no tree root fact"
            ));
        };
        let mut query = Self {
            root,
            starts: Vec::new(),
            by_node: HashMap::new(),
            children: HashMap::new(),
            aborts: Vec::new(),
            submitted_artifact_sequences: records
                .windows(2)
                .filter(|pair| fact_replay::is_submitted_artifact_pair(&pair[0], &pair[1]))
                .map(|pair| pair[1].seq)
                .collect(),
        };
        for record in records {
            if record.meta.tree_id != tree_id {
                return Err(format!(
                    "node fact belongs to tree {} instead of {tree_id}",
                    record.meta.tree_id
                ));
            }
            if let NodeFact::Started(started) = &record.fact {
                if let Some(parent) = &started.parent {
                    if !query
                        .by_node
                        .get(parent.parent_id.as_str())
                        .and_then(|records| records.first())
                        .is_some_and(|record| {
                            query
                                .root
                                .definition
                                .node_by_name(&record.meta.node_name)
                                .is_some_and(NodeDefinition::has_child_executions)
                                && matches!(record.fact, NodeFact::Started(_))
                        })
                    {
                        return Err(format!(
                            "parent scope '{}' has not started",
                            parent.parent_id
                        ));
                    }
                    query
                        .children
                        .entry(&parent.parent_id)
                        .or_default()
                        .push(record);
                }
                query.starts.push(record);
            }
            if matches!(record.fact, NodeFact::AbortRequested) {
                query.aborts.push(record);
            }
            query
                .by_node
                .entry(&record.meta.node_execution_id)
                .or_default()
                .push(record);
        }
        Ok(Some(query))
    }

    fn node_artifact(
        &self,
        start: &NodeFactRecord,
        before: i64,
    ) -> Result<Option<NodeArtifact>, String> {
        let Some(definition) = self.root.definition.node_by_name(&start.meta.node_name) else {
            return Ok(None);
        };
        if self
            .root
            .definition_resolution
            .node_error(&self.root.definition, &start.meta.node_name)
            .is_some()
            || definition.kind_name() != start.meta.kind
        {
            return Ok(None);
        }
        if definition.has_child_executions() {
            return self.composite_artifact(start, definition, before);
        }
        let mut records: Vec<_> = self.by_node[start.meta.node_execution_id.as_str()]
            .iter()
            .copied()
            .filter(|record| record.seq < before)
            .collect();
        records.extend(self.aborts.iter().copied().filter(|record| {
            record.seq > start.seq
                && record.seq < before
                && record.meta.node_execution_id != start.meta.node_execution_id
        }));
        records.sort_by_key(|record| record.seq);
        Ok(fact_replay::fold_leaf_artifact(
            self.root,
            &records,
            &self.submitted_artifact_sequences,
        )?
        .map(|(node, settled_seq)| NodeArtifact { node, settled_seq }))
    }

    fn composite_artifact(
        &self,
        start: &NodeFactRecord,
        definition: &NodeDefinition,
        before: i64,
    ) -> Result<Option<NodeArtifact>, String> {
        enum Observation<'a> {
            Fact(&'a NodeFactRecord),
            Child(Box<NodeArtifact>),
        }
        impl Observation<'_> {
            fn order(&self) -> (i64, u8) {
                match self {
                    Self::Fact(record) => (record.seq, 0),
                    Self::Child(child) => (child.settled_seq, 1),
                }
            }
        }
        let id = start.meta.node_execution_id.as_str();
        let mut observations: Vec<_> = self.by_node[id]
            .iter()
            .copied()
            .chain(self.aborts.iter().copied())
            .filter(|record| record.seq > start.seq && record.seq < before)
            .map(Observation::Fact)
            .collect();
        for child in self
            .children
            .get(id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|child| child.seq < before)
        {
            if self
                .root
                .definition
                .node_by_name(&child.meta.node_name)
                .is_some_and(NodeDefinition::has_child_executions)
            {
                observations.push(Observation::Fact(child));
                if let Some(output) = self.node_artifact(child, before)? {
                    observations.push(Observation::Child(Box::new(output)));
                }
            } else {
                observations.extend(
                    self.by_node[child.meta.node_execution_id.as_str()]
                        .iter()
                        .copied()
                        .filter(|record| record.seq < before)
                        .map(Observation::Fact),
                );
            }
        }
        observations.sort_by_key(Observation::order);
        observations.dedup_by_key(|observation| observation.order());
        let items = definition
            .fanout()
            .map(|spec| self.fanout_items(start, spec))
            .transpose()?
            .flatten();
        let mut aggregate = fact_replay::restore_artifact_scope(self.root, start, definition);
        aggregate.replay_artifact_scope(start, items)?;
        let mut settled_seq = start.seq;
        for (index, observation) in observations.iter().enumerate() {
            let previous = aggregate
                .node_execution(id)
                .map(|node| (node.status, node.completed_at));
            match observation {
                Observation::Fact(record) => {
                    if matches!(record.fact, NodeFact::Started(_)) {
                        aggregate.derive_empty_isolated_fanouts(None)?;
                        aggregate.replay_artifact_child_start(record)?;
                    } else {
                        let defer = observations.get(index + 1).is_some_and(|next| {
                            matches!(next, Observation::Fact(next) if
                                self.submitted_artifact_sequences.contains(&next.seq)
                                    && fact_replay::is_submitted_artifact_pair(record, next))
                        });
                        fact_replay::apply_record(&mut aggregate, record, defer)?;
                        if self.submitted_artifact_sequences.contains(&record.seq) {
                            aggregate.derive_session_settlement(
                                &record.meta.node_execution_id,
                                record.timestamp_ms as f64 / 1000.0,
                            )?;
                        }
                    }
                }
                Observation::Child(child) => {
                    aggregate.derive_artifact_child_settlement(&child.node)?;
                }
            }
            if aggregate
                .node_execution(id)
                .map(|node| (node.status, node.completed_at))
                != previous
            {
                settled_seq = observation.order().0;
            }
        }
        aggregate.derive_empty_isolated_fanouts(None)?;
        Ok(aggregate
            .node_execution(id)
            .cloned()
            .map(|node| NodeArtifact { node, settled_seq }))
    }

    fn fanout_items(
        &self,
        start: &NodeFactRecord,
        spec: &crate::domain::workflow::FanoutSpec,
    ) -> Result<Option<Vec<serde_json::Value>>, String> {
        match &spec.items {
            None => Ok(None),
            Some(ItemsSource::Literal(items)) => Ok(Some(items.clone())),
            Some(ItemsSource::ArtifactField { node, field_path }) => {
                let source = start
                    .meta
                    .parent_id
                    .as_deref()
                    .and_then(|parent| self.children.get(parent))
                    .and_then(|children| {
                        children.iter().rev().find(|candidate| {
                            candidate.seq < start.seq && candidate.meta.node_name == *node
                        })
                    });
                let value = source
                    .map(|source| self.node_artifact(source, start.seq))
                    .transpose()?
                    .flatten()
                    .and_then(|output| output.node.artifact);
                value
                    .as_ref()
                    .and_then(|value| reference::resolve_value_at_path(value, field_path))
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| {
                        format!(
                            "fanout items source '{node}.{}' is unavailable or not an array",
                            field_path.as_string()
                        )
                    })
            }
        }
    }
}

#[cfg(test)]
#[path = "artifact_query_test.rs"]
mod artifact_query_tests;
