use crate::domain::agent_session::aggregates::AgentSessionTreeLocation;

pub fn session_location(id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::session_tree_root(id).unwrap()
}

pub fn workflow_location(tree_id: &str, node_execution_id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::workflow_node(tree_id, node_execution_id).unwrap()
}

/// 事実ログから Archived を導出した状態にする（本番は実行木の Archive の事実から導出する）。
#[cfg(test)]
pub(crate) fn archive_derived(
    session: &mut crate::domain::agent_session::aggregates::AgentSession,
) {
    session.take_uncommitted_events();
    let activity = session.activity();
    session.restore_derived_lifecycle(
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Archived,
        activity,
    );
}
