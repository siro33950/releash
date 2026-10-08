use std::sync::{Arc, Mutex};

use super::{AgentSessionExitPort, AgentSessionExitUsecase};
use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Default)]
struct RecordingExits {
    observed: Mutex<Vec<(String, Option<i32>, String)>>,
}

#[async_trait::async_trait]
impl AgentSessionExitPort for RecordingExits {
    async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        _runtime_generation: u64,
        exit_code: Option<i32>,
        caller_request_id: &str,
    ) -> Result<(), super::AgentSessionLifecycleUsecaseError> {
        self.observed.lock().unwrap().push((
            agent_session_id.to_string(),
            exit_code,
            caller_request_id.to_string(),
        ));
        Ok(())
    }
}

struct TerminalObservations {
    owner: Option<(u64, TerminalSurfaceOwner)>,
    exit_code: Option<i32>,
    exited: Vec<(u64, TerminalSurfaceOwner, Option<i32>)>,
}

impl ProviderAgentTerminalObservationGateway for TerminalObservations {
    fn owner_for_runtime_generation(
        &self,
        _session_key: &str,
        runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        self.owner
            .as_ref()
            .filter(|(generation, _)| *generation == runtime_generation)
            .map(|(_, owner)| owner.clone())
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        self.exited.clone()
    }

    fn session_exit_code(&self, _owner: &TerminalSurfaceOwner) -> Option<i32> {
        self.exit_code
    }
}

#[tokio::test]
async fn test_agent_session_exit_session所有surfaceだけを通知する() {
    let exits = Arc::new(RecordingExits::default());
    let terminal = Arc::new(TerminalObservations {
        owner: Some((
            7,
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap(),
        )),
        exit_code: Some(137),
        exited: Vec::new(),
    });
    let usecase = AgentSessionExitUsecase::new(terminal, exits.clone());

    assert!(usecase
        .observe_exit("session-key", 7, "terminal-exit-7")
        .await
        .unwrap());
    assert_eq!(
        exits.observed.lock().unwrap().as_slice(),
        &[(
            "agent-1".to_string(),
            Some(137),
            "terminal-exit-7".to_string()
        )]
    );
}

#[tokio::test]
async fn test_agent_session_exit_lagged後にexited_surfaceを再照合する() {
    let exits = Arc::new(RecordingExits::default());
    let terminal = Arc::new(TerminalObservations {
        owner: Some((
            1,
            TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
        )),
        exit_code: None,
        exited: vec![
            (
                2,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
                Some(0),
            ),
            (
                3,
                TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-2").unwrap(),
                Some(1),
            ),
        ],
    });
    let usecase = AgentSessionExitUsecase::new(terminal, exits.clone());

    assert!(!usecase
        .observe_exit("workspace-key", 1, "terminal-exit-workspace")
        .await
        .unwrap());
    assert_eq!(
        usecase
            .reconcile_exited("terminal-reconcile")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        exits.observed.lock().unwrap().as_slice(),
        &[(
            "agent-2".to_string(),
            Some(1),
            "terminal-reconcile.agent-2".to_string()
        )]
    );
}

#[tokio::test]
async fn test_agent_session_exit_旧runtime世代の遅延exitで現行sessionを停止しない() {
    let exits = Arc::new(RecordingExits::default());
    let terminal = Arc::new(TerminalObservations {
        owner: Some((
            8,
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap(),
        )),
        exit_code: Some(0),
        exited: Vec::new(),
    });
    let usecase = AgentSessionExitUsecase::new(terminal, exits.clone());

    assert!(!usecase
        .observe_exit("session-key", 7, "terminal-exit-stale-generation")
        .await
        .unwrap());
    assert!(exits.observed.lock().unwrap().is_empty());
}
