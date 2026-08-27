use super::*;
use crate::domain::workflow::{
    ExecutionTreeLaunch, NodeFact, NodeFactMeta, NodeFactRecord, ProcessExitedFact,
    SessionExecutionTreeRootFacts,
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
        derive_agent_session_fields(&records, "session-1", "session-1", "session", "session-1")
            .unwrap();

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
        derive_agent_session_fields(&records, "session-1", "session-1", "session", "session-1")
            .unwrap();
    assert_eq!(paused.lifecycle, AgentSessionLifecycle::Paused);
    assert!(paused.session_facts.exited);

    push_fact(&mut records, meta, NodeFact::ArchiveRequested);
    let archived =
        derive_agent_session_fields(&records, "session-1", "session-1", "session", "session-1")
            .unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycle::Archived);
    assert!(archived.session_facts.archived);
}

#[test]
fn test_agent_session事実導出_session起動木のtreeまたはnodeとsessionのid不一致を拒否する() {
    // Given
    let records = session_records();

    // When / Then
    assert_eq!(
        derive_agent_session_fields(
            &records,
            "different-tree",
            "session-1",
            "session",
            "session-1",
        )
        .unwrap_err(),
        AgentSessionDerivationError::SessionTreeRootIdentityMismatch
    );
    assert_eq!(
        derive_agent_session_fields(
            &records,
            "session-1",
            "different-node",
            "session",
            "session-1",
        )
        .unwrap_err(),
        AgentSessionDerivationError::SessionTreeRootIdentityMismatch
    );
}

#[test]
fn test_agent_session事実導出_root事実欠落を拒否する() {
    // Given
    let records = session_records();

    // When / Then
    assert_eq!(
        derive_agent_session_fields(&[], "session-1", "session-1", "session", "session-1")
            .unwrap_err(),
        AgentSessionDerivationError::MissingTreeRoot
    );
    assert_eq!(
        derive_agent_session_fields(
            &records[1..],
            "session-1",
            "session-1",
            "session",
            "session-1",
        )
        .unwrap_err(),
        AgentSessionDerivationError::MissingTreeRoot
    );
}

#[test]
fn test_agent_session事実導出_definitionからsession_nodeのproviderを解決できなければ拒否する() {
    // Given
    let records = session_records();

    // When
    let error = derive_agent_session_fields(
        &records,
        "session-1",
        "session-1",
        "missing-node",
        "session-1",
    )
    .unwrap_err();

    // Then
    assert_eq!(error, AgentSessionDerivationError::SessionProviderMissing);
}
