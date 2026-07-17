use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::RwLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentSessionNoticeOperation {
    Send,
    LoadSession,
    LoadOlder,
    CancelQueue,
    CloseSession,
    RestoreSession,
    ArchiveSession,
    ForkSession,
    SetTitle,
    RespondPermission,
    SetBackend,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSessionNotice {
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSessionNoticeSnapshot {
    pub session_id: String,
    pub revision: u64,
    pub notice: Option<AgentSessionNotice>,
}

#[derive(Debug)]
pub(super) struct StoredAgentSessionNotice {
    pub(super) operation: AgentSessionNoticeOperation,
    pub(super) message: String,
}

#[derive(Debug, Default)]
pub(crate) struct AgentSessionNoticeState {
    pub(super) revision: u64,
    pub(super) notices: HashMap<String, StoredAgentSessionNotice>,
    pub(super) notice_order: VecDeque<String>,
}

impl AgentSessionNoticeState {
    pub(super) fn snapshot(&self, session_id: &str) -> AgentSessionNoticeSnapshot {
        AgentSessionNoticeSnapshot {
            session_id: session_id.to_owned(),
            revision: self.revision,
            notice: self
                .notices
                .get(session_id)
                .map(|notice| AgentSessionNotice {
                    message: notice.message.clone(),
                }),
        }
    }
}

pub(crate) type SharedAgentSessionNoticeState = Arc<RwLock<AgentSessionNoticeState>>;

pub(crate) fn new_shared_agent_session_notice_state() -> SharedAgentSessionNoticeState {
    Arc::new(RwLock::new(AgentSessionNoticeState::default()))
}
