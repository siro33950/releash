use std::sync::{Arc, Mutex};

use super::agent_session_initial_instruction::AgentSessionInitialInstructionDeliveryOutcome;
use super::{AgentSessionInitialInstructionUsecase, AgentSessionUsecase};
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::aggregates::AgentSessionOrigin;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Default)]
struct FailingTerminalInput {
    writes: Mutex<Vec<(TerminalSurfaceOwner, String)>>,
    write_observed: tokio::sync::Notify,
}

impl ProviderAgentTerminalInputGateway for FailingTerminalInput {
    fn write(
        &self,
        owner: &TerminalSurfaceOwner,
        input: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.writes
            .lock()
            .unwrap()
            .push((owner.clone(), input.to_string()));
        self.write_observed.notify_one();
        Err(ProviderAgentTerminalGatewayError::Unavailable)
    }
}

#[tokio::test]
async fn test_agent_session_initial_instruction_session操作lock解放後に送る() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    sessions
        .create(
            "agent-workflow-locked",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::workflow_node("workflow-execution-1", "node-execution-1").unwrap(),
            "create-workflow-locked",
        )
        .await
        .unwrap();
    let terminal = Arc::new(FailingTerminalInput::default());
    let usecase = Arc::new(AgentSessionInitialInstructionUsecase::new(
        sessions.clone(),
        terminal.clone(),
    ));
    let operation = sessions
        .lock_operation("agent-workflow-locked")
        .await
        .unwrap();
    let dispatch = tokio::spawn({
        let usecase = usecase.clone();
        async move {
            usecase
                .dispatch(
                    "agent-workflow-locked",
                    "initial instruction",
                    "dispatch-locked",
                )
                .await
        }
    });

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            terminal.write_observed.notified(),
        )
        .await
        .is_err(),
        "同じAgentSessionの操作lock中に初期指示を書き込んではならない"
    );

    drop(operation);
    assert_eq!(
        dispatch.await.unwrap().unwrap(),
        AgentSessionInitialInstructionDeliveryOutcome::DeliveryUnknown
    );
}

#[tokio::test]
async fn test_agent_session_initial_instruction_delivery不明でも永続化後に一度だけ送る() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    sessions
        .create(
            "agent-workflow",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::workflow_node("workflow-execution-1", "node-execution-1").unwrap(),
            "create-workflow",
        )
        .await
        .unwrap();
    let terminal = Arc::new(FailingTerminalInput::default());
    let usecase = AgentSessionInitialInstructionUsecase::new(sessions.clone(), terminal.clone());

    assert_eq!(
        usecase
            .dispatch(
                "agent-workflow",
                "system policy\n\ninitial instruction",
                "dispatch-1",
            )
            .await
            .unwrap(),
        AgentSessionInitialInstructionDeliveryOutcome::DeliveryUnknown
    );
    assert_eq!(
        usecase
            .dispatch(
                "agent-workflow",
                "system policy\n\ninitial instruction",
                "dispatch-2",
            )
            .await
            .unwrap(),
        AgentSessionInitialInstructionDeliveryOutcome::AlreadyDispatched
    );
    assert_eq!(
        terminal.writes.lock().unwrap().as_slice(),
        &[(
            {
                TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-workflow")
                    .unwrap()
            },
            "\u{1b}[200~system policy\n\ninitial instruction\u{1b}[201~\r".to_string()
        )]
    );
    assert!(sessions
        .find("agent-workflow")
        .await
        .unwrap()
        .unwrap()
        .session()
        .initial_instruction_admitted());
}
