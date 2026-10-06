pub(crate) mod tests {
    use super::super::*;

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
