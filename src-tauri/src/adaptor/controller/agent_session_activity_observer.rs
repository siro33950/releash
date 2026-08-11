use std::sync::{Arc, OnceLock};

use crate::domain::terminal_surface::gateway::{TerminalSurfaceEvent, TerminalSurfaceEventSink};
use crate::usecase::agent_session::AgentSessionActivityUsecase;

/// terminal surface の event sink を包み、出力／終了イベントを
/// AgentSession activity usecase へ同期観測させる tap。
///
/// event hub の全体購読には Exit しか流れないため、出力recencyの観測は
/// gateway が publish する時点（この tap）で行う。activity usecase は
/// terminal 側 composition より後に組み立つため、`bind` で後結合する。
pub(crate) struct AgentSessionActivityEventTap {
    target: Arc<dyn TerminalSurfaceEventSink>,
    activity: OnceLock<Arc<AgentSessionActivityUsecase>>,
}

impl AgentSessionActivityEventTap {
    pub(crate) fn new(target: Arc<dyn TerminalSurfaceEventSink>) -> Self {
        Self {
            target,
            activity: OnceLock::new(),
        }
    }

    pub(crate) fn bind(&self, activity: Arc<AgentSessionActivityUsecase>) {
        let _ = self.activity.set(activity);
    }
}

impl TerminalSurfaceEventSink for AgentSessionActivityEventTap {
    fn publish(&self, event: TerminalSurfaceEvent) {
        if let Some(activity) = self.activity.get() {
            match &event {
                TerminalSurfaceEvent::Output { session_key, .. } => {
                    activity.observe_output(session_key);
                }
                TerminalSurfaceEvent::Exit { session_key, .. } => {
                    activity.observe_exit(session_key);
                }
                TerminalSurfaceEvent::Resize { .. }
                | TerminalSurfaceEvent::InputUnavailable { .. } => {}
            }
        }
        self.target.publish(event);
    }
}

#[cfg(test)]
#[path = "agent_session_activity_observer_test.rs"]
mod agent_session_activity_observer_tests;
