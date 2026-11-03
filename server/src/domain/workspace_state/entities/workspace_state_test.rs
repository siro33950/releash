use super::*;
use crate::domain::workspace_state::value_objects::pane_layout::PaneLayout;

#[test]
fn test_配置の保存_省略だけ前の配置を保持し明示した空の配置で更新する() {
    // Given
    let original = WorkspaceState {
        panes: Some(PaneLayout::pane("original".into(), vec![], None).unwrap()),
        version: 1,
        tabs: Some(WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        }),
    };
    assert_eq!(original.clone().initialize_panes(), original);
    let mut update = original.clone();
    update.panes = None;
    update.layout.as_mut().unwrap().left_nav_collapsed = true;
    // When / Then
    let saved = update
        .clone()
        .preserve_omitted_fields(Some(original.clone()));
    assert_eq!(saved.panes, original.panes);
    assert!(saved.layout.as_ref().unwrap().left_nav_collapsed);
    assert!(update.preserve_omitted_fields(None).panes.is_none());
    let omitted = WorkspaceState {
        panes: None,
        tabs: None,
        layout: None,
        version: 7,
    };
    let merged = omitted
        .clone()
        .preserve_omitted_fields(Some(original.clone()));
    assert_eq!(merged.tabs, original.tabs);
    assert_eq!(merged.layout, original.layout);
    assert_eq!(merged.panes, original.panes);
    assert_eq!(merged.version, 7);
    assert_eq!(omitted.clone().preserve_omitted_fields(None), omitted);
    let empty = PaneLayout::pane("empty".into(), vec![], None).unwrap();
    let mut update = original.clone();
    update.panes = Some(empty.clone());
    assert_eq!(
        update.preserve_omitted_fields(Some(original)).panes,
        Some(empty)
    );
}

use crate::domain::workspace_state::value_objects::pane_layout::{
    PaneLayoutView, PaneTab, PaneTabKind, SplitAxis,
};

fn tab(id: &str, kind: PaneTabKind, node: Option<&str>, session: Option<&str>) -> PaneTab {
    PaneTab {
        id: id.into(),
        kind,
        node_id: node.map(Into::into),
        agent_session_id: session.map(Into::into),
        file_path: None,
        file_mode: None,
        file_base: None,
    }
}

fn pane(id: &str, tabs: Vec<PaneTab>, active: Option<&str>) -> PaneLayout {
    PaneLayout::pane(id.into(), tabs, active.map(Into::into)).unwrap()
}

fn pane_tabs(layout: &PaneLayout) -> (&[PaneTab], Option<&str>) {
    match layout.view() {
        PaneLayoutView::Pane {
            tabs, active_tab, ..
        } => (tabs, active_tab),
        _ => panic!("expected pane"),
    }
}

#[test]
fn test_sessionタブ追加_別paneのpendingへ束ね選択と順序を保持する() {
    // Given
    let first = pane(
        "first",
        vec![tab("command", PaneTabKind::Command, Some("node"), None)],
        Some("command"),
    );
    let pending = tab("pending", PaneTabKind::Session, Some("node"), None);
    let second = pane(
        "second",
        vec![
            pending.clone(),
            tab("terminal", PaneTabKind::Terminal, None, None),
        ],
        Some("pending"),
    );
    let mut state = WorkspaceState::default();
    state.panes = Some(
        PaneLayout::split(
            "split".into(),
            SplitAxis::Horizontal,
            0.5,
            first.clone(),
            second,
        )
        .unwrap(),
    );
    // When
    state.add_session_tab("session", "node");
    state.add_session_tab("session", "node");
    // Then
    match state.panes.as_ref().unwrap().view() {
        PaneLayoutView::Split {
            first: actual,
            second,
            ..
        } => {
            assert_eq!(actual, &first);
            let (tabs, active) = pane_tabs(second);
            assert_eq!(active, Some("pending"));
            assert_eq!(tabs.len(), 2);
            assert_eq!(
                tabs[0],
                PaneTab {
                    node_id: None,
                    agent_session_id: Some("session".into()),
                    ..pending
                }
            );
            assert_eq!(tabs[1].id, "terminal");
        }
        _ => panic!("expected split"),
    }
}

#[test]
fn test_sessionタブ追加_既存sessionが別paneにあればpendingも変更しない() {
    // Given
    let mut state = WorkspaceState::default();
    state.panes = Some(
        PaneLayout::split(
            "split".into(),
            SplitAxis::Vertical,
            0.4,
            pane(
                "first",
                vec![tab("pending", PaneTabKind::Session, Some("node"), None)],
                None,
            ),
            pane(
                "second",
                vec![tab("existing", PaneTabKind::Session, None, Some("session"))],
                Some("existing"),
            ),
        )
        .unwrap(),
    );
    let previous = state.clone();
    // When
    state.add_session_tab("session", "node");
    // Then
    assert_eq!(state, previous);
}

#[test]
fn test_sessionタブ追加_末尾に追加し全paneの選択と既存タブを保持する() {
    // Given
    let original = tab("session-new", PaneTabKind::Terminal, None, None);
    let second = pane("second", vec![], None);
    let mut state = WorkspaceState::default();
    state.panes = Some(
        PaneLayout::split(
            "split".into(),
            SplitAxis::Horizontal,
            0.5,
            pane("first", vec![original.clone()], Some("session-new")),
            second.clone(),
        )
        .unwrap(),
    );
    // When
    state.add_session_tab("new", "node");
    state.add_session_tab("new", "node");
    // Then
    match state.panes.as_ref().unwrap().view() {
        PaneLayoutView::Split {
            first,
            second: actual,
            ..
        } => {
            assert_eq!(actual, &second);
            let (tabs, active) = pane_tabs(first);
            assert_eq!(active, Some("session-new"));
            assert_eq!(tabs.len(), 2);
            assert_eq!(tabs[0], original);
            assert_ne!(tabs[0].id, tabs[1].id);
            assert_eq!(tabs[1].kind, PaneTabKind::Session);
            assert_eq!(tabs[1].agent_session_id.as_deref(), Some("new"));
            assert_eq!(tabs[1].node_id, None);
        }
        _ => panic!("expected split"),
    }
}

#[test]
fn test_sessionタブ削除_全paneから消し選択中なら隣へ移す() {
    // Given / When / Then
    for (ids, active, expected) in [
        (vec!["left", "gone", "right"], Some("gone"), Some("right")),
        (vec!["left", "gone"], Some("gone"), Some("left")),
        (vec!["gone"], Some("gone"), None),
        (
            vec!["left", "gone", "gone-again", "right"],
            Some("gone"),
            Some("right"),
        ),
        (vec!["gone", "right"], Some("right"), Some("right")),
        (vec!["gone", "right"], None, None),
    ] {
        let mut state = WorkspaceState::default();
        state.panes = Some(
            PaneLayout::split(
                "split".into(),
                SplitAxis::Horizontal,
                0.5,
                pane(
                    "first",
                    ids.iter()
                        .map(|id| {
                            tab(
                                id,
                                PaneTabKind::Session,
                                None,
                                Some(if id.starts_with("gone") {
                                    "session"
                                } else {
                                    id
                                }),
                            )
                        })
                        .collect(),
                    active,
                ),
                pane(
                    "second",
                    vec![tab(
                        "duplicate",
                        PaneTabKind::Session,
                        None,
                        Some("session"),
                    )],
                    Some("duplicate"),
                ),
            )
            .unwrap(),
        );
        state.remove_session_tab("session");
        match state.panes.as_ref().unwrap().view() {
            PaneLayoutView::Split { first, second, .. } => {
                let (tabs, active) = pane_tabs(first);
                assert_eq!(active, expected);
                assert_eq!(
                    tabs.iter().map(|tab| tab.id.as_str()).collect::<Vec<_>>(),
                    ids.into_iter()
                        .filter(|id| !id.starts_with("gone"))
                        .collect::<Vec<_>>()
                );
                assert_eq!(pane_tabs(second), (&[][..], None));
            }
            _ => panic!("expected split"),
        }
        let previous = state.clone();
        state.remove_session_tab("missing");
        assert_eq!(state, previous);
    }
}

#[test]
fn test_配置初期化_未保存だけ空paneを作り設定を保持する() {
    // Given
    let mut state = WorkspaceState::default();
    assert_eq!(pane_tabs(state.panes.as_ref().unwrap()), (&[][..], None));
    assert!(state.tabs.is_none());
    assert!(state.layout.is_none());
    state.panes = None;
    state.version = 9;
    // When
    let initialized = state.initialize_panes();
    // Then
    assert_eq!(initialized.version, 9);
    assert_eq!(initialized.clone().initialize_panes(), initialized);
    let mut missing = initialized.clone();
    missing.panes = None;
    missing.remove_session_tab("missing");
    assert!(missing.panes.is_none());
    missing.add_session_tab("session", "node");
    let (tabs, active) = pane_tabs(missing.panes.as_ref().unwrap());
    assert_eq!(tabs.len(), 1);
    assert_eq!(active, None);
}

#[test]
fn test_実行木のタブ増減_sessionを持つnodeだけ追加削除する() {
    use crate::domain::workflow::{
        AgentSessionActivity, NodeCompletionSignalState, NodeProcessPresence,
    };
    use crate::domain::workspace_tree::{
        WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTree,
        WorkspaceTreeNode,
    };
    // Given
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
        execution_parent: None,
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
    let tree = WorkspaceTree::restore("workspace", nodes).unwrap();
    let mut state = WorkspaceState::default();
    let unrelated = tab("unrelated", PaneTabKind::Session, None, Some("other"));
    state.panes = Some(pane(
        "pane",
        vec![
            unrelated.clone(),
            tab("pending", PaneTabKind::Session, Some("node-0"), None),
        ],
        Some("unrelated"),
    ));
    // When
    state.add_execution_tabs(&tree);
    state.add_execution_tabs(&tree);
    // Then
    let (tabs, active) = pane_tabs(state.panes.as_ref().unwrap());
    assert_eq!(active, Some("unrelated"));
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[1].id, "pending");
    assert_eq!(tabs[1].agent_session_id.as_deref(), Some("one"));
    assert_eq!(tabs[2].agent_session_id.as_deref(), Some("two"));
    // When
    state.remove_execution_tabs(&tree);
    // Then
    assert_eq!(
        pane_tabs(state.panes.as_ref().unwrap()),
        (&[unrelated][..], Some("unrelated"))
    );
    let original = state.clone();
    state.add_execution_tabs(&WorkspaceTree::empty("empty"));
    state.remove_execution_tabs(&WorkspaceTree::empty("empty"));
    assert_eq!(state, original);
}
