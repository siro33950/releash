use std::sync::{Arc, Mutex};

use super::agent_session_initial_instruction::AgentSessionInitialInstructionDeliveryOutcome;
use super::{AgentSessionInitialInstructionUsecase, AgentSessionUsecase};
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::AgentSessionTreeParent;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

/// workflow engine が所有する実行木を模して、session が attach 済みの
/// node を持つ tree を node_events に seed する。
fn seed_workflow_tree(
    store: &Arc<LocalEventStore>,
    tree_id: &str,
    node_execution_id: &str,
    session_id: &str,
    provider: ProviderKind,
) {
    seed_workflow_session_facts(
        store,
        WorkflowSessionFactSeed {
            workflow_name: "wf",
            request: "initial instruction",
            worktree_path: "/repo",
            provider,
            workflow_execution_id: tree_id,
            node_execution_id,
            session_id,
            // dispatch 前の未配送状態を検証する fixture。
            initial_instruction_admitted: false,
        },
    )
    .unwrap();
}

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
        LocalAgentSessionRepository::new(store.clone()),
    )));
    seed_workflow_tree(
        &store,
        "workflow-execution-1",
        "node-execution-1",
        "agent-workflow-locked",
        ProviderKind::Claude,
    );
    sessions
        .create(
            "agent-workflow-locked",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
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
        LocalAgentSessionRepository::new(store.clone()),
    )));
    seed_workflow_tree(
        &store,
        "workflow-execution-1",
        "node-execution-1",
        "agent-workflow",
        ProviderKind::Codex,
    );
    sessions
        .create(
            "agent-workflow",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
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
