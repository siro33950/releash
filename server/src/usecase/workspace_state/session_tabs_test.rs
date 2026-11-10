use super::super::test_support::MemoryWorkspaceStateRepository;
use super::*;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::domain::workspace_state::value_objects::pane_layout::{
    PaneLayout, PaneTab, PaneTabKind,
};
use crate::domain::workspace_tree::{WorkspaceTree, WorkspaceTreeNode};

#[test]
fn test_タブ保存エラー_workflowの分類と技術的失敗を保持する() {
    // Given / When / Then
    assert_eq!(
        conflict_progress(&WorkspaceStateError::VersionConflict),
        Some(AttemptProgress::Reload)
    );
    assert_eq!(conflict_progress(&WorkspaceStateError::InvalidLayout), None);
    assert!(matches!(
        workflow_error(WorkspaceStateError::VersionConflict),
        WorkflowError::Conflict(_)
    ));
    assert!(matches!(
        workflow_error(WorkspaceStateError::InvalidLayout),
        WorkflowError::CorruptStoredState(_)
    ));
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "save failure".into(),
        };
        assert_eq!(
            conflict_progress(&WorkspaceStateError::Technical(failure.clone())),
            None
        );
        assert_eq!(
            workflow_error(WorkspaceStateError::Technical(failure.clone())),
            WorkflowError::Technical(failure)
        );
    }
}

struct Trees {
    tree: Option<WorkspaceTree>,
    calls: parking_lot::Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl WorkspaceTreeRepository for Trees {
    async fn load_all_trees(
        &self,
    ) -> Result<(Vec<WorkspaceTree>, Vec<(String, WorkflowError)>), WorkflowError> {
        Ok((self.tree.clone().into_iter().collect(), Vec::new()))
    }

    async fn load_execution_tree(
        &self,
        workspace: &WorkspaceIdentity,
        execution: &str,
    ) -> Result<Option<WorkspaceTree>, WorkflowError> {
        self.calls
            .lock()
            .push((workspace.as_str().into(), execution.into()));
        Ok(self.tree.clone())
    }

    async fn load_trees(
        &self,
        _: &[WorkspaceIdentity],
    ) -> Vec<Result<WorkspaceTree, WorkflowError>> {
        panic!("unexpected workspace read")
    }

    async fn load_node_by_session_id(
        &self,
        _: &WorkspaceIdentity,
        _: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError> {
        panic!("unexpected session read")
    }

    async fn load_node(
        &self,
        _: &WorkspaceIdentity,
        _: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError> {
        panic!("unexpected node read")
    }
}

fn tab(id: &str, session: Option<&str>) -> PaneTab {
    PaneTab {
        id: id.into(),
        kind: PaneTabKind::Session,
        node_id: None,
        agent_session_id: session.map(Into::into),
        file_path: None,
        file_mode: None,
        file_base: None,
    }
}

fn execution_tree() -> WorkspaceTree {
    use crate::domain::workflow::{
        AgentSessionActivity, NodeCompletionSignalState, NodeProcessPresence,
    };
    use crate::domain::workspace_tree::{
        WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTree,
        WorkspaceTreeNode,
    };
    let root = WorkspaceTreeNode {
        worktree: None,
        id: "execution".into(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::Workflow,
        title: "workflow".into(),
        status: WorkspaceNodeStatus::Running,
        process_presence: NodeProcessPresence::Unknown,
        status_classification: WorkspaceNodeStatusClassification::Idle,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: None,
        error_reason: None,
        updated_at_bits: 1.0_f64.to_bits(),
        execution_id: Some("execution".into()),
        node_execution_id: None,
        node_name: None,
        attempt: None,
        retry_predecessor_id: None,
        past_attempt_ids: vec![],
        is_retry_history: false,
        completion_signals: NodeCompletionSignalState::Pending,
        has_artifact: false,
        session_id: None,
        model: None,
        provider: None,
        can_rename: false,
        can_approve: false,
        can_retry: false,
        can_resume_session: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
        command_result: None,
        dynamic_fanout: false,
    };
    let mut nodes = vec![root.clone()];
    for (index, session) in [Some("one"), None, Some("two")].into_iter().enumerate() {
        let id = format!("node-{index}");
        nodes.push(WorkspaceTreeNode {
            id: id.clone(),
            parent_id: Some(root.id.clone()),
            sibling_order: index as u64,
            kind: WorkspaceNodeKind::WorkflowSession,
            node_execution_id: Some(id.clone()),
            node_name: Some(id),
            attempt: Some(1),
            session_id: session.map(Into::into),
            activity: Some(AgentSessionActivity::Working),
            ..root.clone()
        });
    }
    WorkspaceTree::restore("workspace", nodes).unwrap()
}

#[tokio::test]
async fn test_実行木のarchiveとrestore_対象sessionの配置を保存して通知し選択を保つ() {
    // Given
    let repository = Arc::new(MemoryWorkspaceStateRepository::default());
    let trees = Arc::new(Trees {
        tree: Some(execution_tree()),
        calls: Default::default(),
    });
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let usecase = SessionTabsUsecase::new(repository.clone(), trees.clone(), publisher);
    let mut state = WorkspaceState::default();
    let unrelated = tab("selected", Some("other"));
    let initial = PaneLayout::pane(
        "pane".into(),
        vec![
            unrelated.clone(),
            tab("one-tab", Some("one")),
            tab("two-tab", Some("two")),
        ],
        Some("selected".into()),
    )
    .unwrap();
    state.panes = Some(initial);
    repository
        .save(
            "workspace",
            state.clone(),
            crate::domain::local_event::StreamVersion::zero(),
        )
        .await
        .unwrap();
    // When
    usecase
        .archive_execution("workspace", "execution")
        .await
        .unwrap();
    // Then
    let (_, archived) = repository.read("workspace").await.unwrap();
    state.panes = Some(
        PaneLayout::pane(
            "pane".into(),
            vec![unrelated.clone()],
            Some("selected".into()),
        )
        .unwrap(),
    );
    assert_eq!(archived, Some(state.clone()));
    let expected = StateChangeSource::WorkspaceState("workspace".into());
    assert_eq!(changes.try_recv().unwrap(), expected);
    // When
    usecase
        .restore_execution("workspace", "execution")
        .await
        .unwrap();
    // Then
    let (_, restored) = repository.read("workspace").await.unwrap();
    state.panes = Some(
        PaneLayout::pane(
            "pane".into(),
            vec![
                unrelated,
                tab("session-one", Some("one")),
                tab("session-two", Some("two")),
            ],
            Some("selected".into()),
        )
        .unwrap(),
    );
    assert_eq!(restored, Some(state.clone()));
    assert_eq!(changes.try_recv().unwrap(), expected);
    assert!(changes.try_recv().is_err());
    // When
    usecase
        .restore_execution("workspace", "execution")
        .await
        .unwrap();
    // Then
    let (version, unchanged) = repository.read("workspace").await.unwrap();
    assert_eq!(version.value(), 3);
    assert_eq!(unchanged, Some(state));
    assert!(changes.try_recv().is_err());
    assert_eq!(
        *trees.calls.lock(),
        vec![("workspace".into(), "execution".into()); 3]
    );
}

#[tokio::test]
async fn test_実行木のタブ更新_木が見つからなければ保存も通知もしない() {
    // Given
    let repository = Arc::new(MemoryWorkspaceStateRepository::default());
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let usecase = SessionTabsUsecase::new(
        repository.clone(),
        Arc::new(Trees {
            tree: None,
            calls: Default::default(),
        }),
        publisher,
    );
    // When / Then
    assert_eq!(
        usecase.archive_execution("workspace", "missing").await,
        Err(WorkflowError::NotFound("missing".into()))
    );
    assert_eq!(
        usecase.restore_execution("workspace", "missing").await,
        Err(WorkflowError::NotFound("missing".into()))
    );
    assert_eq!(repository.read("workspace").await.unwrap().1, None);
    assert!(changes.try_recv().is_err());
}
