use crate::usecase::agent_session::AgentSessionChangeNotifier;

pub(crate) struct ClientAgentSessionChangeNotifier {
    publisher: crate::usecase::state_subscription::StateSubscriptionOutputRef,
}

impl ClientAgentSessionChangeNotifier {
    pub(crate) fn new(
        publisher: crate::usecase::state_subscription::StateSubscriptionOutputRef,
    ) -> Self {
        Self { publisher }
    }
}

impl AgentSessionChangeNotifier for ClientAgentSessionChangeNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.publisher.invalidate(
            crate::usecase::state_subscription::StateChangeSource::Worktree(worktree_path.into()),
        );
    }
}

#[cfg(test)]
#[path = "agent_session_change_test.rs"]
mod agent_session_change_tests;
