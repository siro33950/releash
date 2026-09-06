use super::{Artifact, NodeExecutionFailureKind, NodeKindName, TokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeCompletionSignalState {
    #[default]
    Pending,
    SubmitReceived,
    StopReceived,
    Ready,
}

impl NodeCompletionSignalState {
    pub fn is_ready(self) -> bool {
        self == Self::Ready
    }

    pub fn is_partial(self) -> bool {
        matches!(self, Self::SubmitReceived | Self::StopReceived)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCompletionSignal {
    Submit,
    Stop,
}

/// 実行木上の親参照。root の実行インスタンス以外のすべての NodeExecution が持つ。
///
/// 親は合成子（sequence / fanout）の実行インスタンスを node_execution_id で
/// 直接指す。ループで同名 node のインスタンスが複数並ぶ実行木でも一意に決まる。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionParentRef {
    /// 親（合成子インスタンス）の node_execution_id。
    pub parent_id: String,
    /// fanout の子のみ: 展開座標（items 行 / children 列）。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fanout_slot: Option<FanoutSlot>,
}

impl ExecutionParentRef {
    pub fn sequence_child(parent_id: impl Into<String>) -> Self {
        Self {
            parent_id: parent_id.into(),
            fanout_slot: None,
        }
    }

    pub fn fanout_child(
        parent_id: impl Into<String>,
        item_index: Option<usize>,
        child_index: usize,
    ) -> Self {
        Self {
            parent_id: parent_id.into(),
            fanout_slot: Some(FanoutSlot {
                item_index,
                child_index,
            }),
        }
    }

    pub fn is_fanout_child(&self) -> bool {
        self.fanout_slot.is_some()
    }
}

/// fanout 展開の座標（宣言順上の位置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanoutSlot {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub item_index: Option<usize>,
    pub child_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeExecutionStatus {
    Unresolved,
    Running,
    Paused,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

impl NodeExecutionStatus {
    #[cfg(test)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::WaitingApproval => "waiting_approval",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Paused | Self::WaitingApproval | Self::Unresolved
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeExecutionFailure {
    pub reason: String,
    pub kind: NodeExecutionFailureKind,
}

/// event replay から構築する node 実行 1 回分の read model。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeExecution {
    pub recovery_reason: Option<String>,
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub status: NodeExecutionStatus,
    pub session_id: Option<String>,
    pub display_command: Option<String>,
    pub result_summary: Option<String>,
    pub artifact: Option<Artifact>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<NodeExecutionFailure>,
    pub parent: Option<ExecutionParentRef>,
    pub completion_signals: NodeCompletionSignalState,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

impl NodeExecution {
    pub fn is_fanout_child(&self) -> bool {
        self.parent
            .as_ref()
            .is_some_and(ExecutionParentRef::is_fanout_child)
    }

    pub fn can_retry(&self) -> bool {
        if self.recovery_reason.is_some() {
            return false;
        }
        self.status == NodeExecutionStatus::Failed
            || (matches!(
                self.status,
                NodeExecutionStatus::Running | NodeExecutionStatus::Paused
            ) && self.completion_signals.is_partial())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_execution_status_reports_active_states() {
        assert!(NodeExecutionStatus::Running.is_active());
        assert!(NodeExecutionStatus::WaitingApproval.is_active());
        assert!(!NodeExecutionStatus::Succeeded.is_active());
        assert_eq!(NodeExecutionStatus::Succeeded.as_str(), "succeeded");
    }

    #[test]
    fn retry_admission_belongs_to_the_node_execution() {
        let mut node = NodeExecution {
            recovery_reason: None,
            id: "node-1".to_string(),
            execution_id: "execution-1".to_string(),
            node_name: "review".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            status: NodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure: None,
            parent: None,
            completion_signals: NodeCompletionSignalState::StopReceived,
            started_at: 1.0,
            completed_at: None,
        };

        assert!(node.can_retry());
        node.completion_signals = NodeCompletionSignalState::Pending;
        assert!(!node.can_retry());
        node.status = NodeExecutionStatus::Failed;
        assert!(node.can_retry());
    }

    #[test]
    fn sequence_child_has_no_fanout_slot() {
        let parent = ExecutionParentRef::sequence_child("seq-1");
        assert_eq!(parent.parent_id, "seq-1");
        assert_eq!(parent.fanout_slot, None);
        assert!(!parent.is_fanout_child());
    }

    #[test]
    fn fanout_child_carries_its_expansion_coordinates() {
        let parent = ExecutionParentRef::fanout_child("fan-1", Some(2), 1);
        assert_eq!(parent.parent_id, "fan-1");
        assert_eq!(
            parent.fanout_slot,
            Some(FanoutSlot {
                item_index: Some(2),
                child_index: 1,
            })
        );
        assert!(parent.is_fanout_child());

        let static_child = ExecutionParentRef::fanout_child("fan-1", None, 0);
        assert!(static_child.is_fanout_child());
        assert_eq!(
            static_child.fanout_slot,
            Some(FanoutSlot {
                item_index: None,
                child_index: 0,
            })
        );
    }
}
