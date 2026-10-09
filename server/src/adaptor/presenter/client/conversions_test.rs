use super::*;

#[test]
fn test_workflow設定入力_転送値の真偽と欠落を入力データへ写す() {
    // Given
    let values = [(Some(true), true), (Some(false), false), (None, false)];

    // When
    for (wire_value, expected) in values {
        let input =
            crate::usecase::app_config::WorkflowConfigInput::try_from(wire::WorkflowSection {
                approval_auto_approve: wire_value.map(|value| value.try_into().unwrap()),
            })
            .unwrap();
        // Then
        assert_eq!(input.approval_auto_approve, expected);
    }
}

#[test]
fn test_review転送_解決済みthreadの全項目を既存のjsonの形で保持する() {
    use crate::domain::comment::{
        ReviewActor, ReviewComment, ReviewResolveInfo, ReviewTarget, ReviewThread,
        ReviewThreadState,
    };
    // Given
    let actor =
        ReviewActor::provider_agent("codex".into(), Some("session".into())).redacted_for_public();
    let thread = ReviewThread {
        id: "thread".into(),
        worktree_name: "/repo".into(),
        author: actor.clone(),
        target: ReviewTarget {
            file_path: Some("src/main.rs".into()),
            line_number: Some(3),
            end_line: Some(5),
        },
        state: ReviewThreadState::Resolved,
        comments: vec![ReviewComment {
            id: "comment".into(),
            thread_id: "thread".into(),
            author: actor.clone(),
            content: "content".into(),
            created_at: 1.0,
        }],
        resolve: Some(ReviewResolveInfo {
            actor,
            outcome: "fixed".into(),
            summary: "summary".into(),
            resolved_at: 2.0,
        }),
        created_at: 1.0,
        updated_at: 2.0,
        version: 3,
        can_resolve: false,
    };
    // When
    let message = wire::ReviewThreadDto::try_from(thread).unwrap();
    let json = wire::from_message("releash.client.v1.ReviewThreadDto", &message).unwrap();
    // Then
    let actor = serde_json::json!({"kind":"agent", "backendId":"codex", "model":null, "displayName":"codex"});
    assert_eq!(
        json,
        serde_json::json!({
            "id":"thread", "worktreeName":"/repo", "author":actor,
            "target":{"filePath":"src/main.rs", "lineNumber":3, "endLine":5}, "state":"resolved",
            "comments":[{"id":"comment", "threadId":"thread", "author":actor, "content":"content", "createdAt":1.0}],
            "resolve":{"actor":actor, "outcome":"fixed", "summary":"summary", "resolvedAt":2.0},
            "createdAt":1.0, "updatedAt":2.0, "version":3, "canResolve":false,
        })
    );
}

#[test]
fn test_pane転送_分割と全タブを往復して欠落と未知の種類を拒否する() {
    use crate::domain::workspace_state::value_objects::pane_layout::{
        PaneLayout, PaneTab, PaneTabKind, SplitAxis,
    };
    let pane = PaneLayout::Pane {
        id: "a".into(),
        tabs: vec![
            PaneTab {
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
            },
            PaneTab {
                id: "workflow".into(),
                kind: PaneTabKind::Workflow,
            },
            PaneTab {
                id: "file".into(),
                kind: PaneTabKind::File,
            },
        ],
        active_tab: Some("file".into()),
    };
    for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
        let layout = PaneLayout::Split {
            id: "split".into(),
            axis,
            ratio: 0.7,
            first: Box::new(pane.clone()),
            second: Box::new(PaneLayout::Pane {
                id: "b".into(),
                tabs: vec![],
                active_tab: None,
            }),
        };
        let message = wire::PaneLayout::try_from(layout.clone()).unwrap();
        assert_eq!(PaneLayout::try_from(message).unwrap(), layout);
    }
    let mut state = crate::domain::workspace_state::WorkspaceState {
        version: 1,
        panes: None,
        tabs: crate::domain::workspace_state::value_objects::WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: crate::domain::workspace_state::value_objects::WorkspaceLayoutState {
            center_tab: "agent".into(),
            active_view: "git".into(),
            left_nav_collapsed: true,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    state.panes = Some(pane.clone());
    let message = wire::WorkspaceStateDto::try_from(state.clone()).unwrap();
    assert!(message.pane_layout.is_some());
    assert_eq!(
        crate::domain::workspace_state::WorkspaceState::try_from(message).unwrap(),
        state
    );
    assert!(PaneLayout::try_from(wire::PaneLayout::default()).is_err());
    let mut message = wire::PaneLayout::try_from(pane).unwrap();
    if let Some(wire::pane_layout::Node::Pane(pane)) = &mut message.node {
        pane.tabs[0].kind = 999;
    }
    assert!(PaneLayout::try_from(message).is_err());
    for axis in [0, 999, wire::SplitAxis::Horizontal as i32] {
        assert!(PaneLayout::try_from(wire::PaneLayout {
            node: Some(wire::pane_layout::Node::Split(Box::new(wire::PaneSplit {
                axis,
                ..Default::default()
            })))
        })
        .is_err());
    }
}
