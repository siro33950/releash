use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycleEvent};
use crate::usecase::workspace_state::session_tabs::SessionTabsUsecase;

pub(super) async fn follow(
    session_tabs: &SessionTabsUsecase,
    session: &AgentSession,
    events: &[AgentSessionLifecycleEvent],
) {
    if !events.iter().any(|event| {
        matches!(
            event,
            AgentSessionLifecycleEvent::ProcessExited { .. } | AgentSessionLifecycleEvent::Resumed
        )
    }) {
        return;
    }
    if let Err(error) = session_tabs.follow_session(session, events).await {
        log::warn!(
            "failed to follow AgentSession '{}' facts in the Session tabs: {error:?}",
            session.id()
        );
    }
}

#[cfg(test)]
#[path = "session_tab_follow_test.rs"]
mod session_tab_follow_tests;
