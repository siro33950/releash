use crate::domain::workflow::NodeExecutionStatus;
use crate::domain::workspace_tree::WorkspaceStructureFact;

#[derive(Clone, Copy)]
pub(crate) enum NodeTimestampObservation {
    Started(f64),
    Bound(f64),
    Prepared(f64),
    Artifact(f64),
    Approval(f64),
    Completed(f64),
    Failed(f64),
}

impl NodeTimestampObservation {
    pub(crate) fn snapshot(
        started: f64,
        completed: Option<f64>,
        status: NodeExecutionStatus,
        has_artifact: bool,
        execution_updated: f64,
    ) -> impl Iterator<Item = Self> {
        let settled = completed.unwrap_or(execution_updated);
        [
            Some(Self::Started(started)),
            has_artifact.then_some(Self::Artifact(settled)),
            match status {
                NodeExecutionStatus::Running => None,
                NodeExecutionStatus::WaitingApproval => Some(Self::Approval(execution_updated)),
                NodeExecutionStatus::Succeeded => Some(Self::Completed(settled)),
                NodeExecutionStatus::Aborted => Some(Self::Failed(settled)),
            },
        ]
        .into_iter()
        .flatten()
    }

    pub(crate) fn from_structure_fact(fact: &WorkspaceStructureFact) -> Option<(&str, &str, Self)> {
        use WorkspaceStructureFact as F;
        let (execution, node, change) = match fact {
            F::NodeStarted {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Started(*timestamp)),
            F::NodeAgentBound {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Bound(*timestamp)),
            F::NodeCommandPrepared {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Prepared(*timestamp)),
            F::NodeArtifactProduced {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Artifact(*timestamp)),
            F::NodeApprovalRequested {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Approval(*timestamp)),
            F::NodeCompleted {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Completed(*timestamp)),
            F::NodeFailed {
                execution_id,
                node_execution_id,
                timestamp,
                ..
            } => (execution_id, node_execution_id, Self::Failed(*timestamp)),
            _ => return None,
        };
        Some((execution, node, change))
    }

    pub(crate) fn timestamp(self) -> f64 {
        match self {
            Self::Started(time)
            | Self::Bound(time)
            | Self::Prepared(time)
            | Self::Artifact(time)
            | Self::Approval(time)
            | Self::Completed(time)
            | Self::Failed(time) => time,
        }
    }

    pub(crate) fn apply(self, current: u64) -> u64 {
        super::super::entities::max_f64_bits(current, self.timestamp().to_bits())
    }
}
