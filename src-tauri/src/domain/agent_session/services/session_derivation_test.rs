use super::*;
use crate::domain::workflow::{
    AgentActivityObservedFact, AgentSessionActivity, ExecutionTreeLaunch, NodeFact, NodeFactMeta,
    NodeFactRecord, ProcessExitedFact, SessionExecutionTreeRootFacts,
};

fn session_records() -> Vec<NodeFactRecord> {
    SessionExecutionTreeRootFacts::new("session-1", "workspace-1", "/repo", ProviderKind::Claude)
        .unwrap()
        .into_facts()
        .into_iter()
        .enumerate()
        .map(|(index, (meta, fact))| NodeFactRecord {
            meta,
            seq: i64::try_from(index + 1).unwrap(),
            timestamp_ms: i64::try_from(index + 1).unwrap() * 1_000,
            fact,
        })
        .collect()
}

fn push_fact(records: &mut Vec<NodeFactRecord>, meta: NodeFactMeta, fact: NodeFact) {
    let seq = i64::try_from(records.len() + 1).unwrap();
    records.push(NodeFactRecord {
        meta,
        seq,
        timestamp_ms: seq * 1_000,
        fact,
    });
}

#[test]
fn test_agent_session事実導出_session起動木の属性と3種lifecycleを導出する() {
    // Given
    let mut records = session_records();
    let meta = records[0].meta.clone();

    // When
    let open =
        derive_session_fields(&records, &context(), "session-1", "session-1", "session-1").unwrap();

    // Then
    assert_eq!(open.tree_location.tree_id(), "session-1");
    assert_eq!(open.tree_location.node_execution_id(), "session-1");
    assert_eq!(
        open.tree_location.launched_as(),
        ExecutionTreeLaunch::Session
    );
    assert_eq!(open.provider, ProviderKind::Claude);
    assert_eq!(open.workspace_identity, "workspace-1");
    assert_eq!(open.worktree_path, "/repo");
    assert_eq!(open.lifecycle, AgentSessionLifecycle::Open);
    assert!(!open.session_facts.exited);
    assert_eq!(
        open.session_facts.activity,
        AgentSessionActivity::AwaitingInstruction
    );

    push_fact(
        &mut records,
        meta.clone(),
        NodeFact::AgentActivityObserved(AgentActivityObservedFact {
            activity: AgentSessionActivity::Working,
        }),
    );
    let working =
        derive_session_fields(&records, &context(), "session-1", "session-1", "session-1").unwrap();
    assert_eq!(working.lifecycle, AgentSessionLifecycle::Open);
    assert_eq!(
        working.session_facts.activity,
        AgentSessionActivity::Working
    );

    push_fact(
        &mut records,
        meta.clone(),
        NodeFact::ProcessExited(ProcessExitedFact {
            exit_code: Some(0),
            result_summary: None,
            failure_reason: None,
            failure_kind: None,
        }),
    );
    let paused =
        derive_session_fields(&records, &context(), "session-1", "session-1", "session-1").unwrap();
    assert_eq!(paused.lifecycle, AgentSessionLifecycle::Paused);
    assert!(paused.session_facts.exited);
    assert_eq!(
        paused.session_facts.activity,
        AgentSessionActivity::AwaitingInstruction
    );

    push_fact(&mut records, meta, NodeFact::ArchiveRequested);
    let archived =
        derive_session_fields(&records, &context(), "session-1", "session-1", "session-1").unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycle::Archived);
    assert!(archived.session_facts.archived);
}

#[test]
fn test_agent_session事実導出_session起動木のtreeまたはnodeとsessionのid不一致を拒否する() {
    // Given
    let records = session_records();

    // When / Then
    assert_eq!(
        derive_session_fields(
            &records,
            &context(),
            "different-tree",
            "session-1",
            "session-1",
        )
        .unwrap_err(),
        AgentSessionDerivationError::SessionTreeRootIdentityMismatch
    );
    assert_eq!(
        derive_session_fields(
            &records,
            &context(),
            "session-1",
            "different-node",
            "session-1",
        )
        .unwrap_err(),
        AgentSessionDerivationError::SessionTreeRootIdentityMismatch
    );
}

fn context() -> SessionExecutionContext {
    SessionExecutionContext {
        workspace_identity: "workspace-1".into(),
        worktree_path: "/repo".into(),
        launched_as: ExecutionTreeLaunch::Session,
        provider: ProviderKind::Claude,
    }
}

#[test]
fn test_agent_session事実導出_workflow定義を渡さずsession自身の事実から復元できる() {
    // Given
    let mut records = session_records();
    let meta = records[0].meta.clone();
    push_fact(&mut records, meta, NodeFact::ArchiveRequested);
    records.retain(|record| !matches!(record.fact, NodeFact::Started(_)));

    // When
    let session =
        derive_session_fields(&records, &context(), "session-1", "session-1", "session-1").unwrap();

    // Then
    assert_eq!(session.lifecycle, AgentSessionLifecycle::Archived);
    assert_eq!(session.provider, ProviderKind::Claude);
}
