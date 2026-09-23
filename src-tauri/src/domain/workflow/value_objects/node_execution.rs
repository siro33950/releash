use super::{Artifact, NodeKindName, TokenUsage};

pub fn startup_restart_delay(restarts: u32) -> Option<std::time::Duration> {
    (restarts < 4).then(|| std::time::Duration::from_secs(1 << restarts))
}

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCompletionSignal {
    Submit,
    Stop,
}

/// 実行木上の親参照。root の実行インスタンス以外のすべての NodeExecution が持つ。
///
/// 親は sequence / fanout / delegate を宣言した Session の実行インスタンスを node_execution_id で
/// 直接指す。ループで同名 node のインスタンスが複数並ぶ実行木でも一意に決まる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionParentRef {
    pub parent_id: String,
    relation: ExecutionParentRelation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionParentRelation {
    Sequence,
    Fanout(FanoutSlot),
    Delegate,
}

impl ExecutionParentRef {
    pub fn sequence_child(parent_id: impl Into<String>) -> Self {
        Self {
            parent_id: parent_id.into(),
            relation: ExecutionParentRelation::Sequence,
        }
    }

    pub fn fanout_child(
        parent_id: impl Into<String>,
        item_index: Option<usize>,
        child_index: usize,
    ) -> Self {
        Self {
            parent_id: parent_id.into(),
            relation: ExecutionParentRelation::Fanout(FanoutSlot {
                item_index,
                child_index,
            }),
        }
    }

    pub fn delegate_child(parent_id: impl Into<String>) -> Self {
        Self {
            parent_id: parent_id.into(),
            relation: ExecutionParentRelation::Delegate,
        }
    }

    pub fn fanout_slot(&self) -> Option<FanoutSlot> {
        match self.relation {
            ExecutionParentRelation::Fanout(slot) => Some(slot),
            _ => None,
        }
    }

    pub fn is_delegate_child(&self) -> bool {
        matches!(self.relation, ExecutionParentRelation::Delegate)
    }

    pub fn is_fanout_child(&self) -> bool {
        self.fanout_slot().is_some()
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
    Running,
    WaitingApproval,
    Succeeded,
    Aborted,
}

impl NodeExecutionStatus {
    pub fn can_retry(self, kind: NodeKindName, presence: NodeProcessPresence) -> bool {
        self == Self::Running
            && kind == NodeKindName::Command
            && presence == NodeProcessPresence::ConfirmedAbsent
    }

    #[cfg(test)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Succeeded => "succeeded",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

/// event replay から構築する node 実行 1 回分の read model。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeExecution {
    pub worktree: Option<crate::domain::workflow::IsolatedWorktree>,
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub status: NodeExecutionStatus,
    pub process_presence: NodeProcessPresence,
    pub session_id: Option<String>,
    pub display_command: Option<String>,
    pub result_summary: Option<String>,
    pub artifact: Option<Artifact>,
    pub token_usage: Option<TokenUsage>,
    pub parent: Option<ExecutionParentRef>,
    pub completion_signals: NodeCompletionSignalState,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeProcessPresence {
    Live,
    ConfirmedAbsent,
    #[default]
    Unknown,
}

impl NodeProcessPresence {
    pub fn can_resume_session(self, kind: NodeKindName) -> bool {
        kind == NodeKindName::Session && self == Self::ConfirmedAbsent
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::ConfirmedAbsent => "confirmed_absent",
            Self::Unknown => "unknown",
        }
    }
}

impl NodeExecution {
    pub fn can_retry(&self) -> bool {
        self.status.can_retry(self.kind, self.process_presence)
    }

    pub fn can_resume_session(&self) -> bool {
        self.process_presence.can_resume_session(self.kind)
    }

    pub fn is_fanout_child(&self) -> bool {
        self.parent
            .as_ref()
            .is_some_and(ExecutionParentRef::is_fanout_child)
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
    fn sequence_child_has_no_fanout_slot() {
        let parent = ExecutionParentRef::sequence_child("seq-1");
        assert_eq!(parent.parent_id, "seq-1");
        assert_eq!(parent.fanout_slot(), None);
        assert!(!parent.is_fanout_child());
    }

    #[test]
    fn fanout_child_carries_its_expansion_coordinates() {
        let parent = ExecutionParentRef::fanout_child("fan-1", Some(2), 1);
        assert_eq!(parent.parent_id, "fan-1");
        assert_eq!(
            parent.fanout_slot(),
            Some(FanoutSlot {
                item_index: Some(2),
                child_index: 1,
            })
        );
        assert!(parent.is_fanout_child());

        let static_child = ExecutionParentRef::fanout_child("fan-1", None, 0);
        assert!(static_child.is_fanout_child());
        assert_eq!(
            static_child.fanout_slot(),
            Some(FanoutSlot {
                item_index: None,
                child_index: 0,
            })
        );
    }
    #[test]
    fn manual_node_actions_require_confirmed_absence_and_the_matching_leaf_kind() {
        use NodeExecutionStatus as S;
        use NodeProcessPresence as P;
        for (status, retry_command) in [
            (S::Running, true),
            (S::WaitingApproval, false),
            (S::Succeeded, false),
            (S::Aborted, false),
        ] {
            for presence in [P::Live, P::Unknown, P::ConfirmedAbsent] {
                let absent = presence == P::ConfirmedAbsent;
                assert_eq!(
                    status.can_retry(NodeKindName::Command, presence),
                    absent && retry_command
                );
                assert!(!status.can_retry(NodeKindName::Session, presence));
                for kind in [NodeKindName::Sequence, NodeKindName::Fanout] {
                    assert!(!status.can_retry(kind, presence));
                }
            }
        }
    }

    #[test]
    fn test_session再開可否_nodeの状態に関係なくsessionのプロセス不在だけで決まる() {
        use NodeProcessPresence as P;
        for presence in [P::Live, P::Unknown, P::ConfirmedAbsent] {
            // When / Then
            assert_eq!(
                presence.can_resume_session(NodeKindName::Session),
                presence == P::ConfirmedAbsent
            );
            for kind in [
                NodeKindName::Command,
                NodeKindName::Sequence,
                NodeKindName::Fanout,
            ] {
                assert!(!presence.can_resume_session(kind));
            }
        }
    }
}
