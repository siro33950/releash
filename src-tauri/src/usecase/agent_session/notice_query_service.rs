use super::notice_state::{AgentSessionNoticeSnapshot, SharedAgentSessionNoticeState};

#[derive(Clone)]
pub struct AgentSessionNoticeQueryService {
    state: SharedAgentSessionNoticeState,
}

impl AgentSessionNoticeQueryService {
    pub(crate) fn new(state: SharedAgentSessionNoticeState) -> Self {
        Self { state }
    }

    pub fn get(&self, session_id: &str) -> AgentSessionNoticeSnapshot {
        self.state.read().snapshot(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::notice_state::{
        AgentSessionNoticeOperation, AgentSessionNoticeState, StoredAgentSessionNotice,
    };
    use parking_lot::RwLock;
    use std::sync::Arc;

    #[test]
    fn query_returns_current_snapshot_without_mutating_state() {
        let state = Arc::new(RwLock::new(AgentSessionNoticeState::default()));
        {
            let mut stored = state.write();
            stored.revision = 7;
            stored.notices.insert(
                "session-a".to_string(),
                StoredAgentSessionNotice {
                    operation: AgentSessionNoticeOperation::Send,
                    message: "send failed".to_string(),
                },
            );
        }
        let query_service = AgentSessionNoticeQueryService::new(state.clone());

        let snapshot = query_service.get("session-a");

        assert_eq!(snapshot.session_id, "session-a");
        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.notice.unwrap().message, "send failed");
        assert_eq!(state.read().revision, 7);
    }
}
