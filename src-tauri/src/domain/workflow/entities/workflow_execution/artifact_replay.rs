use super::*;
use crate::domain::workflow::NodeFactRecord;

pub(super) struct ReplayedNodeStart<'a> {
    pub node_execution_id: &'a str,
    pub node_name: &'a str,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub parent: Option<ExecutionParentRef>,
    pub timestamp: f64,
}

pub(super) enum ReplayScope {
    Recorded,
    ArtifactRoot(Option<Vec<serde_json::Value>>),
    ArtifactChild,
}

impl WorkflowExecution {
    pub(crate) fn replay_artifact_scope(
        &mut self,
        start: &NodeFactRecord,
        items: Option<Vec<serde_json::Value>>,
    ) -> Result<(), String> {
        self.replay_node_start(
            ReplayedNodeStart {
                node_execution_id: &start.meta.node_execution_id,
                node_name: &start.meta.node_name,
                kind: start.meta.kind,
                attempt: start.meta.attempt,
                parent: None,
                timestamp: start.timestamp_ms as f64 / 1000.0,
            },
            ReplayScope::ArtifactRoot(items),
        )
    }

    pub(crate) fn replay_artifact_child_start(
        &mut self,
        start: &NodeFactRecord,
    ) -> Result<(), String> {
        let crate::domain::workflow::NodeFact::Started(fact) = &start.fact else {
            return Err("artifact child must begin with a started fact".into());
        };
        self.replay_node_start(
            ReplayedNodeStart {
                node_execution_id: &start.meta.node_execution_id,
                node_name: &start.meta.node_name,
                kind: start.meta.kind,
                attempt: start.meta.attempt,
                parent: fact.parent.clone(),
                timestamp: start.timestamp_ms as f64 / 1000.0,
            },
            ReplayScope::ArtifactChild,
        )
    }

    pub(crate) fn derive_artifact_child_settlement(
        &mut self,
        child: &RuntimeNodeExecution,
    ) -> Result<(), String> {
        let timestamp = child.completed_at.unwrap_or(child.started_at);
        match child.status {
            RuntimeNodeExecutionStatus::Succeeded => {
                self.record_pending_result(
                    &child.id,
                    child.result_summary.clone(),
                    child.artifact.clone(),
                    None,
                    child.token_usage.clone(),
                    timestamp,
                );
                self.derive_leaf_completed(&child.id, timestamp)
            }
            RuntimeNodeExecutionStatus::Failed => {
                let failure = child
                    .failure
                    .as_ref()
                    .ok_or("failed composite has no failure")?;
                self.derive_leaf_failed(&child.id, failure.reason.clone(), failure.kind, timestamp)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn derive_empty_isolated_fanouts(
        &mut self,
        failing_node: Option<&str>,
    ) -> Result<(), String> {
        let Some(id) = self.pending_empty_fanout.take() else {
            return Ok(());
        };
        if !self.is_active() || failing_node == Some(id.as_str()) {
            return Ok(());
        }
        let Some(node) = self
            .node_execution(&id)
            .filter(|node| node.status == RuntimeNodeExecutionStatus::Running)
        else {
            return Ok(());
        };
        let timestamp = node.started_at;
        self.complete_scope(&id, false, &mut AdvanceEffects::Derive, timestamp)
            .map_err(|error| error.to_string())
    }
}
