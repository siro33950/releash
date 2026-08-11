use std::sync::{Arc, Mutex};

use super::AgentSessionActivityEventTap;
use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::gateway::{TerminalSurfaceEvent, TerminalSurfaceEventSink};
use crate::domain::terminal_surface::{TerminalActivity, TerminalSurfaceOwner};
use crate::usecase::agent_session::{AgentSessionActivityUsecase, AgentSessionChangeNotifier};

#[derive(Default)]
struct RecordingNotifier {
    notified: Mutex<Vec<String>>,
}

impl AgentSessionChangeNotifier for RecordingNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.notified
            .lock()
            .unwrap()
            .push(worktree_path.to_string());
    }
}

#[derive(Default)]
struct RecordingTargetSink {
    published: Mutex<Vec<TerminalSurfaceEvent>>,
}

impl TerminalSurfaceEventSink for RecordingTargetSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        self.published.lock().unwrap().push(event);
    }
}

struct FixedSessionSurface;

impl ProviderAgentTerminalObservationGateway for FixedSessionSurface {
    fn owner_for_runtime_generation(
        &self,
        _session_key: &str,
        _runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        None
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        Vec::new()
    }

    fn session_exit_code(&self, _owner: &TerminalSurfaceOwner) -> Option<i32> {
        None
    }

    fn session_activity(&self, _owner: &TerminalSurfaceOwner) -> TerminalActivity {
        TerminalActivity::Idle
    }

    fn session_worktree_path(&self, session_key: &str) -> Option<String> {
        (session_key == "agent-surface").then(|| "/repo/worktree".to_string())
    }
}

fn output_event(session_key: &str) -> TerminalSurfaceEvent {
    TerminalSurfaceEvent::Output {
        session_key: session_key.to_string(),
        data: "output".into(),
        sequence: 1,
    }
}

#[tokio::test]
async fn test_agent_session_activity_tap_出力イベントを観測して通知しつつ転送する() {
    let target = Arc::new(RecordingTargetSink::default());
    let tap = AgentSessionActivityEventTap::new(target.clone());
    let notifier = Arc::new(RecordingNotifier::default());
    tap.bind(Arc::new(AgentSessionActivityUsecase::new(
        Arc::new(FixedSessionSurface),
        notifier.clone(),
        tokio::runtime::Handle::current(),
    )));

    tap.publish(output_event("agent-surface"));

    assert_eq!(
        notifier.notified.lock().unwrap().as_slice(),
        &["/repo/worktree"]
    );
    assert_eq!(
        target.published.lock().unwrap().as_slice(),
        &[output_event("agent-surface")]
    );
}

#[tokio::test]
async fn test_agent_session_activity_tap_usecase未結合でもイベントを転送する() {
    let target = Arc::new(RecordingTargetSink::default());
    let tap = AgentSessionActivityEventTap::new(target.clone());

    tap.publish(output_event("agent-surface"));

    assert_eq!(
        target.published.lock().unwrap().as_slice(),
        &[output_event("agent-surface")]
    );
}
