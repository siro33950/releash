use std::sync::{Arc, Mutex};

use super::agent_session_initial_instruction::AgentSessionInitialInstructionDeliveryOutcome;
use super::{AgentSessionInitialInstructionUsecase, AgentSessionUsecase};
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::fact_log;
use crate::domain::agent_session::aggregates::AgentSessionTreeParent;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionParentRef, NodeDefinition, NodeFact, NodeFactMeta, NodeKind,
    NodeKindName, SequenceSpec, SessionAttachedFact, SessionSpec, StartedFact, TreeRootFact,
    WorkflowDefinition, WorkflowRootFact,
};
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
    let definition = WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    output: None,
                    children: Vec::new(),
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "impl".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider,
                    model: None,
                    permission: None,
                    facets: Default::default(),
                }),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    };
    let root_meta = NodeFactMeta {
        tree_id: tree_id.to_string(),
        node_execution_id: tree_id.to_string(),
        parent_id: None,
        node_name: "main".to_string(),
        kind: NodeKindName::Sequence,
        attempt: 1,
    };
    let node_meta = NodeFactMeta {
        tree_id: tree_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        parent_id: Some(tree_id.to_string()),
        node_name: "impl".to_string(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    fact_log::append_single_fact(
        store,
        &root_meta,
        &NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact::Workflow(WorkflowRootFact {
                workflow_name: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::DesktopUi,
                request: "initial instruction".to_string(),
                definition,
            })),
        }),
        1,
    )
    .unwrap();
    fact_log::append_single_fact(
        store,
        &node_meta,
        &NodeFact::Started(StartedFact {
            parent: Some(ExecutionParentRef::sequence_child(tree_id)),
            root: None,
        }),
        2,
    )
    .unwrap();
    fact_log::append_single_fact(
        store,
        &node_meta,
        &NodeFact::SessionAttached(SessionAttachedFact {
            session_id: session_id.to_string(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
        }),
        3,
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
