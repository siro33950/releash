use std::sync::{Arc, Mutex};

use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceEventSource,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionExitPort, AgentSessionExitUsecase, AgentSessionLifecycleUsecaseError,
};

#[derive(Default)]
struct RecordingExitPort {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl AgentSessionExitPort for RecordingExitPort {
    async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        _runtime_generation: u64,
        _exit_code: Option<i32>,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        self.calls
            .lock()
            .unwrap()
            .push(agent_session_id.to_string());
        Ok(())
    }
}

struct FixedTerminalObservation;

impl ProviderAgentTerminalObservationGateway for FixedTerminalObservation {
    fn owner_for_runtime_generation(
        &self,
        session_key: &str,
        runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        (session_key == "agent-surface" && runtime_generation == 3).then(|| {
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap()
        })
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        Vec::new()
    }

    fn session_exit_code(&self, _owner: &TerminalSurfaceOwner) -> Option<i32> {
        Some(0)
    }

    fn session_activity(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> crate::domain::terminal_surface::TerminalActivity {
        crate::domain::terminal_surface::TerminalActivity::Idle
    }

    fn session_worktree_path(&self, _session_key: &str) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn test_agent_session_exit_observer_frontendなしでexitをusecaseへ渡す() {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let stream = hub.subscribe();
    let cancellation = stream.cancellation.clone();
    let exits = Arc::new(RecordingExitPort::default());
    let usecase = Arc::new(AgentSessionExitUsecase::new(
        Arc::new(FixedTerminalObservation),
        exits.clone(),
    ));
    let task = tokio::spawn(super::run_agent_session_exit_observer(stream, usecase));

    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: "agent-surface".to_string(),
        runtime_generation: 3,
        exit_code: Some(0),
        sequence: 7,
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !exits.calls.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    task.await.unwrap();

    assert_eq!(exits.calls.lock().unwrap().as_slice(), &["agent-1"]);
}
