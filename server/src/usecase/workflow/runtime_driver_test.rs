pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::entities::workflow_execution::TransitionRejection;
    use crate::domain::workflow::{NodeKindName, RuntimeExecutionState};

    fn execution_with_attached_session() -> ExecutionTree {
        let mut execution = ExecutionTree::restore(RuntimeExecutionState::Running);
        execution
            .begin_node_attempt(
                "session".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution".to_string(),
                1.0,
            )
            .unwrap();
        execution.attach_node_session("node-execution", "agent-session".to_string(), 1.0);
        execution
    }

    fn aborted_event() -> WorkflowEvent {
        WorkflowEvent::ExecutionAborted {
            execution_id: "execution".into(),
            aborted_node: None,
            timestamp: 1.0,
        }
    }

    #[tokio::test]
    async fn persistence_failure_keeps_exact_pre_commit_aggregate_and_releases_no_effects() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let before = live.clone();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        let result = prepared
            .persist(&mut live, |_| std::future::ready(Err("disk")))
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert_eq!(live, before);
    }

    #[tokio::test]
    async fn effects_become_available_only_after_durable_persistence() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        let durable = prepared
            .persist(&mut live, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::Applied);
        assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
        assert_eq!(live.state(), &RuntimeExecutionState::Aborted);
    }

    #[tokio::test]
    async fn already_applied_observation_persists_event_without_changing_aggregate() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let before = live.clone();
        let event = aborted_event();
        let prepared = PreparedWorkflowTransaction::capture_with_outcome(
            before.clone(),
            before.clone(),
            TransitionOutcome::AlreadyApplied,
            vec![event.clone()],
            Vec::new(),
        )
        .unwrap();
        let mut persisted = Vec::new();

        let durable = prepared
            .persist(&mut live, |events| {
                persisted.extend(events);
                std::future::ready(Ok::<_, ()>(()))
            })
            .await
            .unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::AlreadyApplied);
        assert_eq!(persisted, vec![event]);
        assert_eq!(live, before);
    }

    #[tokio::test]
    async fn newly_terminal_session_stop_effect_becomes_available_after_persistence() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort_node_execution("node-execution", 2.0);
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let durable = prepared
            .persist(&mut live, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

        assert_eq!(
            durable.into_effects(),
            vec![
                WorkflowRuntimeEffect::BroadcastState,
                WorkflowRuntimeEffect::StopWorkflowAgentSession {
                    node_execution_id: "node-execution".to_string(),
                    agent_session_id: "agent-session".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn newly_terminal_session_persistence_failure_keeps_active_aggregate() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort_node_execution("node-execution", 2.0);
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let result = prepared
            .persist(&mut live, |_| std::future::ready(Err("disk")))
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert!(live
            .node_execution("node-execution")
            .unwrap()
            .status
            .is_active());
    }

    #[tokio::test]
    async fn stale_candidate_is_rejected_without_persistence() {
        let live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut stale = ExecutionTree::restore(RuntimeExecutionState::Running);
        stale.abort();
        let mut persisted = false;

        let result = prepared
            .persist(&mut stale, |_| {
                persisted = true;
                std::future::ready(Ok::<_, ()>(()))
            })
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::StaleCandidate)
        ));
        assert!(!persisted);
    }

    #[tokio::test]
    async fn aggregate_rejection_is_preserved_as_a_typed_decision() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Completed);
        let rejection = match live.replay_started() {
            crate::domain::workflow::entities::workflow_execution::ReplayOutcome::Rejected(
                reason,
            ) => reason,
            outcome => panic!("unexpected replay outcome: {outcome:?}"),
        };
        let prepared = PreparedWorkflowTransaction::observe(&live, |_candidate| {
            Ok(WorkflowRuntimeDecision {
                outcome: TransitionOutcome::Rejected(rejection),
                events: Vec::new(),
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut candidate = live.clone();

        let durable = prepared
            .persist(&mut candidate, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

        assert_eq!(
            durable.outcome(),
            TransitionOutcome::Rejected(TransitionRejection::NotActive)
        );
    }
}
