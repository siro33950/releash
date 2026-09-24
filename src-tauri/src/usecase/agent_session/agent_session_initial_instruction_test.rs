use std::sync::{Arc, Mutex};

use super::agent_session_initial_instruction::AgentSessionInitialInstructionDeliveryOutcome;
use super::{AgentSessionInitialInstructionUsecase, AgentSessionUsecase};
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::AgentSessionTreeLocation;
use crate::domain::agent_session::{
    ProviderAgentTerminalGatewayError, ProviderAgentTerminalInputGateway,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

fn workflow_location(tree_id: &str, node_execution_id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::workflow_node(tree_id, node_execution_id).unwrap()
}

/// workflow engine が所有する実行木を模して、session が attach 済みの
/// node を持つ tree を node_events に seed する。
async fn seed_workflow_tree(
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
    .await
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
async fn test_agent_session_continuation_session操作lock解放後に送る() {
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
    )
    .await;
    sessions
        .create(
            "agent-workflow-locked",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            workflow_location("workflow-execution-1", "node-execution-1"),
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
                .dispatch_continuation(
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

#[derive(Default)]
struct ContinuationTerminalInput {
    writes: Mutex<Vec<(TerminalSurfaceOwner, String)>>,
    fail: bool,
}

impl ProviderAgentTerminalInputGateway for ContinuationTerminalInput {
    fn write(
        &self,
        owner: &TerminalSurfaceOwner,
        input: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        if self.fail {
            return Err(ProviderAgentTerminalGatewayError::Unavailable);
        }
        self.writes
            .lock()
            .unwrap()
            .push((owner.clone(), input.into()));
        Ok(())
    }
}

#[tokio::test]
async fn test_delegate_続行指示は識別子ごとに一度だけ送り再送は受理済みとして書かない() {
    use super::agent_session_initial_instruction::AgentSessionInitialInstructionError;
    use crate::domain::agent_session::aggregates::AgentSessionInitialInstructionOutcome;
    for fail in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
            LocalAgentSessionRepository::new(store.clone()),
        )));
        seed_workflow_tree(&store, "tree", "node", "agent", ProviderKind::Codex).await;
        sessions
            .create(
                "agent",
                WorkspaceIdentity::new("/repo"),
                "/repo/worktree",
                ProviderKind::Codex,
                workflow_location("tree", "node"),
                "create",
            )
            .await
            .unwrap();
        let terminal = Arc::new(ContinuationTerminalInput {
            fail,
            ..Default::default()
        });
        let usecase =
            AgentSessionInitialInstructionUsecase::new(sessions.clone(), terminal.clone());
        // When / Then
        assert_eq!(
            usecase
                .dispatch_continuation("agent", " \n", "child-1")
                .await,
            Err(AgentSessionInitialInstructionError::InvalidInput)
        );
        assert_eq!(
            usecase
                .dispatch_continuation("agent", "continue", " ")
                .await,
            Err(AgentSessionInitialInstructionError::InvalidInput)
        );
        assert_eq!(
            usecase
                .dispatch_continuation("missing", "continue", "child-1")
                .await,
            Err(AgentSessionInitialInstructionError::NotFound)
        );
        let first_delivery = if fail {
            AgentSessionInitialInstructionDeliveryOutcome::DeliveryUnknown
        } else {
            AgentSessionInitialInstructionDeliveryOutcome::Delivered
        };
        assert_eq!(
            usecase
                .dispatch_continuation("agent", "first\n", "child-1")
                .await
                .unwrap(),
            first_delivery
        );
        assert_eq!(
            usecase
                .dispatch_continuation("agent", "second\r\n", "child-2")
                .await
                .unwrap(),
            first_delivery
        );
        assert_eq!(
            usecase
                .dispatch_continuation("agent", "first again\n", "child-1")
                .await
                .unwrap(),
            AgentSessionInitialInstructionDeliveryOutcome::AlreadyDispatched
        );
        let writes = terminal.writes.lock().unwrap();
        if fail {
            assert!(writes.is_empty());
        } else {
            assert_eq!(writes.len(), 2);
            assert_eq!(
                writes[0].0,
                TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent").unwrap()
            );
            assert_eq!(writes[0].1, "\u{1b}[200~first\u{1b}[201~\r");
            assert_eq!(writes[1].1, "\u{1b}[200~second\u{1b}[201~\r");
        }
        // 受理は terminal への書き込み結果によらず永続化され、再読込後も同じ識別子を拒む。
        let reloaded =
            AgentSessionUsecase::new(Arc::new(LocalAgentSessionRepository::new(store.clone())));
        assert_eq!(
            reloaded
                .admit_continuation("agent", "child-1")
                .await
                .unwrap(),
            AgentSessionInitialInstructionOutcome::AlreadyAdmitted
        );
        assert_eq!(
            reloaded
                .admit_continuation("agent", "child-3")
                .await
                .unwrap(),
            AgentSessionInitialInstructionOutcome::Admitted
        );
    }
}

#[tokio::test]
async fn test_terminal投入_継続指示は同じ末尾改行処理とpaste形式を使う() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(store.clone()),
    )));
    seed_workflow_tree(&store, "tree", "node", "agent", ProviderKind::Codex).await;
    sessions
        .create(
            "agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            workflow_location("tree", "node"),
            "create",
        )
        .await
        .unwrap();
    let terminal = Arc::new(ContinuationTerminalInput::default());
    let usecase = AgentSessionInitialInstructionUsecase::new(sessions, terminal.clone());
    // When
    assert_eq!(
        usecase
            .dispatch_continuation("agent", "first\nsecond \r\n\r", "initial")
            .await
            .unwrap(),
        AgentSessionInitialInstructionDeliveryOutcome::Delivered
    );
    usecase
        .dispatch_continuation("agent", "first\nsecond \r\n\r", "child-1")
        .await
        .unwrap();
    // Then
    let writes = terminal.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0], writes[1]);
    assert_eq!(writes[0].1, "\u{1b}[200~first\nsecond \u{1b}[201~\r");
}

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    use crate::usecase::agent_session::agent_session_initial_instruction::AgentSessionInitialInstructionError;
    // Given
    let cases = [
        (
            AgentSessionInitialInstructionError::InvalidInput,
            F::InvalidInput,
        ),
        (AgentSessionInitialInstructionError::NotFound, F::Missing),
        (
            AgentSessionInitialInstructionError::Conflict(
                crate::domain::failure::FailureKind::RestartRequired,
            ),
            F::RestartRequired,
        ),
        (
            AgentSessionInitialInstructionError::StorageUnavailable,
            F::Temporary,
        ),
        (AgentSessionInitialInstructionError::Corrupt, F::Corrupt),
        (
            AgentSessionInitialInstructionError::Store(F::Temporary),
            F::Temporary,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Expired),
            F::Expired,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Missing),
            F::Missing,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Permission),
            F::Permission,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Capacity),
            F::Capacity,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Internal),
            F::Internal,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Corrupt),
            F::Corrupt,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Cancelled),
            F::Cancelled,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::Unknown),
            F::Unknown,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            AgentSessionInitialInstructionError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
