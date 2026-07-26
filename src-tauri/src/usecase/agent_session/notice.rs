use std::sync::Arc;

use parking_lot::RwLock;

use super::notice_query_service::AgentSessionNoticeQueryService;
#[cfg(test)]
use super::notice_state::new_shared_agent_session_notice_state;
pub use super::notice_state::{AgentSessionNoticeOperation, AgentSessionNoticeSnapshot};
use super::notice_state::{SharedAgentSessionNoticeState, StoredAgentSessionNotice};

pub trait AgentSessionNoticePublisher: Send + Sync {
    fn publish(&self, snapshot: AgentSessionNoticeSnapshot);
}

#[cfg(test)]
pub trait AgentSessionNoticeSessionLookup: Send + Sync {
    fn contains_session(&self, session_id: &str) -> bool;
}

pub(crate) const MAX_AGENT_SESSION_NOTICE_ENTRIES: usize = 512;
pub(crate) const MAX_AGENT_SESSION_NOTICE_MESSAGE_BYTES: usize = 2 * 1024;

fn bounded_notice_message(message: &str) -> String {
    if message.len() <= MAX_AGENT_SESSION_NOTICE_MESSAGE_BYTES {
        return message.to_string();
    }
    const MARKER: &str = "…";
    let mut cut = MAX_AGENT_SESSION_NOTICE_MESSAGE_BYTES - MARKER.len();
    while cut > 0 && !message.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &message[..cut], MARKER)
}

#[cfg(test)]
struct AllSessionsKnown;

#[cfg(test)]
impl AgentSessionNoticeSessionLookup for AllSessionsKnown {
    fn contains_session(&self, _session_id: &str) -> bool {
        true
    }
}

pub enum AgentSessionNoticeUpdate {
    Failure {
        operation: AgentSessionNoticeOperation,
        message: String,
    },
    Success {
        operation: AgentSessionNoticeOperation,
    },
    Dismiss,
}

/// Session-scoped transient notices and their recovery policy.
///
/// This legacy UI projection is intentionally isolated from R-011 durable
/// feedback. Durable feedback creation and control must go through
/// `SessionFeedbackUsecase`, where identity and revision guards are mandatory.
pub struct AgentSessionNoticeUsecase {
    state: SharedAgentSessionNoticeState,
    query_service: AgentSessionNoticeQueryService,
    #[cfg(test)]
    session_lookup: Option<Arc<dyn AgentSessionNoticeSessionLookup>>,
    publishers: RwLock<Vec<Arc<dyn AgentSessionNoticePublisher>>>,
}

#[cfg(test)]
impl Default for AgentSessionNoticeUsecase {
    fn default() -> Self {
        let state = new_shared_agent_session_notice_state();
        Self::new_for_test(state)
    }
}

impl AgentSessionNoticeUsecase {
    #[cfg(test)]
    pub(crate) fn new(
        state: SharedAgentSessionNoticeState,
        query_service: AgentSessionNoticeQueryService,
        session_lookup: Arc<dyn AgentSessionNoticeSessionLookup>,
    ) -> Self {
        Self {
            state,
            query_service,
            session_lookup: Some(session_lookup),
            publishers: RwLock::new(Vec::new()),
        }
    }

    pub(crate) fn new_for_runtime(
        state: SharedAgentSessionNoticeState,
        query_service: AgentSessionNoticeQueryService,
    ) -> Self {
        Self {
            state,
            query_service,
            #[cfg(test)]
            session_lookup: None,
            publishers: RwLock::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(state: SharedAgentSessionNoticeState) -> Self {
        Self::new(
            state.clone(),
            AgentSessionNoticeQueryService::new(state),
            Arc::new(AllSessionsKnown),
        )
    }

    pub fn get_notice(&self, session_id: &str) -> AgentSessionNoticeSnapshot {
        self.query_service.get(session_id)
    }

    pub fn update(
        &self,
        session_id: &str,
        update: AgentSessionNoticeUpdate,
    ) -> AgentSessionNoticeSnapshot {
        #[cfg(test)]
        if self
            .session_lookup
            .as_ref()
            .is_some_and(|lookup| !lookup.contains_session(session_id))
        {
            return self.get_notice(session_id);
        }
        let mut state = self.state.write();
        let changed = match update {
            AgentSessionNoticeUpdate::Failure { operation, message } => {
                if !state.notices.contains_key(session_id)
                    && state.notices.len() >= MAX_AGENT_SESSION_NOTICE_ENTRIES
                {
                    return state.snapshot(session_id);
                }
                state
                    .notice_order
                    .retain(|stored_id| stored_id != session_id);
                state.notice_order.push_back(session_id.to_owned());
                state.notices.insert(
                    session_id.to_owned(),
                    StoredAgentSessionNotice {
                        operation,
                        message: bounded_notice_message(&message),
                    },
                );
                true
            }
            AgentSessionNoticeUpdate::Success { operation } => {
                if state
                    .notices
                    .get(session_id)
                    .is_some_and(|notice| notice.operation == operation)
                {
                    state.notices.remove(session_id);
                    state
                        .notice_order
                        .retain(|stored_id| stored_id != session_id);
                    true
                } else {
                    false
                }
            }
            AgentSessionNoticeUpdate::Dismiss => {
                let removed = state.notices.remove(session_id).is_some();
                if removed {
                    state
                        .notice_order
                        .retain(|stored_id| stored_id != session_id);
                }
                removed
            }
        };

        if changed {
            state.revision += 1;
        }
        let snapshot = state.snapshot(session_id);
        drop(state);

        if changed {
            let publishers = self.publishers.read().clone();
            for publisher in publishers {
                publisher.publish(snapshot.clone());
            }
        }

        snapshot
    }

    pub fn register_publisher(&self, publisher: Arc<dyn AgentSessionNoticePublisher>) {
        self.publishers.write().push(publisher);
    }

    pub fn record_operation_result<T>(
        &self,
        session_id: &str,
        operation: AgentSessionNoticeOperation,
        result: &Result<T, String>,
        failure_label: &str,
    ) {
        let update = match result {
            Ok(_) => AgentSessionNoticeUpdate::Success { operation },
            Err(error) => AgentSessionNoticeUpdate::Failure {
                operation,
                message: format!("{failure_label}: {error}"),
            },
        };
        self.update(session_id, update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashSet;

    use crate::usecase::agent_session::notice_state::AgentSessionNotice;

    struct RecordingPublisher {
        changes: Arc<Mutex<Vec<AgentSessionNoticeSnapshot>>>,
    }

    struct KnownSessions {
        ids: HashSet<String>,
    }

    impl AgentSessionNoticeSessionLookup for KnownSessions {
        fn contains_session(&self, session_id: &str) -> bool {
            self.ids.contains(session_id)
        }
    }

    fn usecase_for_known_sessions(
        session_ids: impl IntoIterator<Item = String>,
    ) -> AgentSessionNoticeUsecase {
        let state = new_shared_agent_session_notice_state();
        AgentSessionNoticeUsecase::new(
            state.clone(),
            AgentSessionNoticeQueryService::new(state),
            Arc::new(KnownSessions {
                ids: session_ids.into_iter().collect(),
            }),
        )
    }

    impl AgentSessionNoticePublisher for RecordingPublisher {
        fn publish(&self, snapshot: AgentSessionNoticeSnapshot) {
            self.changes.lock().push(snapshot);
        }
    }

    fn failure(operation: AgentSessionNoticeOperation, message: &str) -> AgentSessionNoticeUpdate {
        AgentSessionNoticeUpdate::Failure {
            operation,
            message: message.to_owned(),
        }
    }

    #[test]
    fn test_session_notice_update_sessions_are_isolated() {
        let usecase = AgentSessionNoticeUsecase::default();

        usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );
        usecase.update(
            "session-b",
            failure(AgentSessionNoticeOperation::LoadSession, "load failed"),
        );

        assert_eq!(
            usecase
                .update(
                    "session-a",
                    AgentSessionNoticeUpdate::Success {
                        operation: AgentSessionNoticeOperation::LoadSession,
                    },
                )
                .notice,
            Some(AgentSessionNotice {
                message: "send failed".to_owned(),
            })
        );
        assert_eq!(
            usecase
                .update(
                    "session-b",
                    AgentSessionNoticeUpdate::Success {
                        operation: AgentSessionNoticeOperation::LoadSession,
                    },
                )
                .notice,
            None
        );
    }

    #[test]
    fn test_session_notice_update_matching_success_recovers_only_same_operation() {
        let usecase = AgentSessionNoticeUsecase::default();
        usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );

        let unrelated = usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Success {
                operation: AgentSessionNoticeOperation::LoadOlder,
            },
        );
        assert_eq!(
            unrelated.notice,
            Some(AgentSessionNotice {
                message: "send failed".to_owned(),
            })
        );

        let recovered = usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Success {
                operation: AgentSessionNoticeOperation::Send,
            },
        );
        assert_eq!(recovered.notice, None);
    }

    #[test]
    fn test_session_notice_update_latest_failure_replaces_previous_notice() {
        let usecase = AgentSessionNoticeUsecase::default();
        usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );

        let current = usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::LoadOlder, "load failed"),
        );

        assert_eq!(
            current.notice,
            Some(AgentSessionNotice {
                message: "load failed".to_owned(),
            })
        );
        assert_eq!(
            usecase
                .update(
                    "session-a",
                    AgentSessionNoticeUpdate::Success {
                        operation: AgentSessionNoticeOperation::Send,
                    },
                )
                .notice,
            Some(AgentSessionNotice {
                message: "load failed".to_owned(),
            })
        );
    }

    #[test]
    fn test_session_notice_update_dismiss_discards_transient_notice() {
        let usecase = AgentSessionNoticeUsecase::default();
        usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );
        assert_eq!(
            usecase
                .update("session-a", AgentSessionNoticeUpdate::Dismiss)
                .notice,
            None
        );
    }

    #[test]
    fn test_session_notice_revision_increases_only_when_state_changes() {
        let usecase = AgentSessionNoticeUsecase::default();
        let failed = usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );
        let unrelated = usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Success {
                operation: AgentSessionNoticeOperation::LoadSession,
            },
        );
        let dismissed = usecase.update("session-a", AgentSessionNoticeUpdate::Dismiss);

        assert_eq!(failed.revision, 1);
        assert_eq!(unrelated.revision, 1);
        assert_eq!(dismissed.revision, 2);
    }

    #[test]
    fn test_session_notice_publisher_receives_backend_snapshot_deltas() {
        let state = new_shared_agent_session_notice_state();
        let usecase = AgentSessionNoticeUsecase::new(
            state.clone(),
            AgentSessionNoticeQueryService::new(state.clone()),
            Arc::new(KnownSessions {
                ids: HashSet::from(["session-a".to_string()]),
            }),
        );
        let changes = Arc::new(Mutex::new(Vec::new()));
        usecase.register_publisher(Arc::new(RecordingPublisher {
            changes: changes.clone(),
        }));

        usecase.update(
            "session-a",
            failure(
                AgentSessionNoticeOperation::RestoreSession,
                "restore failed",
            ),
        );
        let queried = state.read().snapshot("session-a");
        usecase.update("session-a", AgentSessionNoticeUpdate::Dismiss);

        assert_eq!(queried.revision, 1);
        assert_eq!(
            changes.lock().as_slice(),
            [
                AgentSessionNoticeSnapshot {
                    session_id: "session-a".to_string(),
                    revision: 1,
                    notice: Some(AgentSessionNotice {
                        message: "restore failed".to_string(),
                    }),
                },
                AgentSessionNoticeSnapshot {
                    session_id: "session-a".to_string(),
                    revision: 2,
                    notice: None,
                },
            ]
        );
    }

    #[test]
    fn test_session_notice_update_unknown_session_is_not_retained() {
        let usecase = usecase_for_known_sessions(["known".to_string()]);

        let snapshot = usecase.update(
            "unknown",
            failure(AgentSessionNoticeOperation::Send, "send failed"),
        );

        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.notice, None);
        assert_eq!(usecase.state.read().notices.len(), 0);
    }

    #[test]
    fn test_session_notice_update_oversize_message_is_utf8_bounded() {
        let usecase = usecase_for_known_sessions(["session-a".to_string()]);
        let oversized = "x".repeat(MAX_AGENT_SESSION_NOTICE_MESSAGE_BYTES + 1);

        let snapshot = usecase.update(
            "session-a",
            failure(AgentSessionNoticeOperation::Send, &oversized),
        );

        assert_eq!(snapshot.revision, 1);
        let message = snapshot.notice.unwrap().message;
        assert!(message.len() <= MAX_AGENT_SESSION_NOTICE_MESSAGE_BYTES);
        assert!(message.ends_with('…'));
        assert_eq!(usecase.state.read().notices.len(), 1);
    }

    #[test]
    fn test_session_notice_update_capacity_preserves_existing_entries() {
        let session_ids = (0..=MAX_AGENT_SESSION_NOTICE_ENTRIES)
            .map(|index| format!("session-{index}"))
            .collect::<Vec<_>>();
        let usecase = usecase_for_known_sessions(session_ids.clone());
        for session_id in session_ids.iter().take(MAX_AGENT_SESSION_NOTICE_ENTRIES) {
            usecase.update(
                session_id,
                failure(AgentSessionNoticeOperation::Send, "send failed"),
            );
        }

        usecase.update(
            &session_ids[MAX_AGENT_SESSION_NOTICE_ENTRIES],
            failure(AgentSessionNoticeOperation::LoadSession, "load failed"),
        );

        let state = usecase.state.read();
        assert_eq!(state.notices.len(), MAX_AGENT_SESSION_NOTICE_ENTRIES);
        assert!(state.notices.contains_key(&session_ids[0]));
        assert!(!state
            .notices
            .contains_key(&session_ids[MAX_AGENT_SESSION_NOTICE_ENTRIES]));
    }
}
