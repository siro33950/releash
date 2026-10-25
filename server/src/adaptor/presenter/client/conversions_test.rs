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
    todo!()
}

#[test]
fn test_pane転送_分割と全タブを往復して欠落と未知の種類を拒否する() {
    use crate::domain::workspace_state::value_objects::pane_layout::{
        PaneLayout, PaneTab, PaneTabKind, SplitAxis,
    };
    let pane = PaneLayout::pane(
        "a".into(),
        vec![
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "workflow".into(),
                kind: PaneTabKind::Command,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "file".into(),
                kind: PaneTabKind::File,
                file_path: Some("src/file.rs".into()),
                file_mode: Some(crate::domain::code::value_objects::review::FileMode::Diff),
                file_base: Some(crate::domain::code::ReviewBase::BranchBase),
            },
            PaneTab {
                agent_session_id: Some("agent-session".into()),
                node_id: None,
                id: "session".into(),
                kind: PaneTabKind::Session,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: Some("command-node".into()),
                id: "command".into(),
                kind: PaneTabKind::Command,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
        ],
        Some("file".into()),
    )
    .unwrap();
    for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
        let layout = PaneLayout::split(
            "split".into(),
            axis,
            0.7,
            pane.clone(),
            PaneLayout::pane("b".into(), vec![], None).unwrap(),
        )
        .unwrap();
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
    for kind in [2, 999] {
        if let Some(wire::pane_layout::Node::Pane(pane)) = &mut message.node {
            pane.tabs[0].kind = kind;
        }
        assert!(PaneLayout::try_from(message.clone()).is_err());
    }
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

#[test]
fn test_行の側_proto変換で前後を保持し不明な値を拒否する() {
    use crate::domain::comment::LineSide as ReviewLineSide;
    // Given / When / Then
    for side in [ReviewLineSide::Additions, ReviewLineSide::Deletions] {
        let wire = wire::ReviewLineSideDto::try_from(side).unwrap();
        assert_eq!(ReviewLineSide::try_from(wire).unwrap(), side);
    }
    for value in [None, Some(99)] {
        assert!(ReviewLineSide::try_from(wire::ReviewLineSideDto { value }).is_err());
    }
}

#[test]
fn test_session表示状態_全状態をwireのenumへ変換する() {
    use crate::domain::agent_session::aggregates::AgentSessionAvailability;
    use wire::AgentSessionAvailability as Value;
    // Given / When / Then
    for (availability, value) in [
        (AgentSessionAvailability::Open, Value::Open),
        (AgentSessionAvailability::Paused, Value::Paused),
        (
            AgentSessionAvailability::Indeterminate,
            Value::Indeterminate,
        ),
        (AgentSessionAvailability::Archived, Value::Archived),
    ] {
        assert_eq!(wire::AgentSessionAvailability::from(availability), value);
    }
}

#[test]
fn test_node詳細の転送_分類と実行識別とmodelと過去の試行の値を保持する() {
    use crate::usecase::workflow::*;
    // Given
    let detail = WorkspaceNodeDetailDto {
        classification: "attention".into(),
        node_execution_id: Some("visit".into()),
        node_name: Some("review".into()),
        model: Some("test-model".into()),
        past_attempts: vec![WorkspaceNodeAttemptDto {
            id: "previous".into(),
            title: "Previous review".into(),
            updated_at: 123.5,
            error_reason: Some("Failed review".into()),
        }],
        execution_id: Some("execution".into()),
        can_abort: true,
        process_presence: "confirmed_absent",
        worktree: None,
        id: "current".into(),
        title: "Review".into(),
        status: "waiting".into(),
        submit_received: false,
        stop_received: false,
        waiting_for: None,
        has_artifact: true,
        error_reason: None,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_rename: false,
            can_approve: true,
            can_retry: false,
            can_resume_session: false,
        },
        updated_at: 124.0,
        content: WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: None,
        }),
    };
    // When
    let wire = wire::WorkspaceNodeDetailDto::try_from(detail).unwrap();
    // Then
    assert_eq!(
        wire.classification.unwrap().value,
        Some(wire::workspace_status_classification::Value::Attention as i32)
    );
    assert_eq!(wire.node_execution_id.as_deref(), Some("visit"));
    assert_eq!(wire.node_name.as_deref(), Some("review"));
    assert_eq!(wire.model.as_deref(), Some("test-model"));
    assert_eq!(wire.past_attempts.len(), 1);
    let attempt = &wire.past_attempts[0];
    assert_eq!(attempt.id.as_deref(), Some("previous"));
    assert_eq!(attempt.title.as_deref(), Some("Previous review"));
    assert_eq!(attempt.updated_at, Some(123.5));
    assert_eq!(attempt.error_reason.as_deref(), Some("Failed review"));
}

#[test]
fn test_session詳細の転送_ドメインの状態と操作可否を既存の形式で配る() {
    use crate::domain::workflow::{NodeCompletionSignalState as S, NodeProcessPresence};
    use crate::domain::workspace_tree::{
        WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceNodeStatusClassification,
        WorkspaceTreeNode,
    };
    // Given
    let node = WorkspaceTreeNode {
        id: "node".into(),
        title: "Review".into(),
        parent_id: Some("execution".into()),
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        status: WorkspaceNodeStatus::Waiting,
        process_presence: NodeProcessPresence::ConfirmedAbsent,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: None,
        error_reason: Some("needs attention".into()),
        updated_at_bits: 124.0_f64.to_bits(),
        execution_id: Some("execution".into()),
        node_execution_id: Some("visit".into()),
        node_name: Some("review".into()),
        attempt: Some(2),
        execution_parent: None,
        retry_predecessor_id: None,
        past_attempt_ids: vec!["previous".into()],
        is_retry_history: false,
        completion_signals: S::Pending,
        has_artifact: true,
        session_id: Some("session".into()),
        model: Some("test-model".into()),
        provider: None,
        can_rename: true,
        can_approve: true,
        can_retry: false,
        can_resume_session: true,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
        command_result: None,
        worktree: None,
        dynamic_fanout: false,
    };
    for (signals, submit, stop, waiting) in [
        (S::Pending, false, false, None),
        (S::SubmitReceived, true, false, Some("stop")),
        (S::StopReceived, false, true, Some("submit")),
        (S::Ready, true, true, None),
    ] {
        let mut node = node.clone();
        node.completion_signals = signals;
        // When
        let message = wire::WorkspaceNodeDetailDto::try_from((node, true)).unwrap();
        let actual =
            wire::from_message("releash.client.v1.WorkspaceNodeDetailDto", &message).unwrap();
        // Then
        let mut expected = serde_json::json!({
            "id": "node", "title": "Review", "classification": "attention", "status": "waiting",
            "processPresence": "confirmed_absent", "nodeExecutionId": "visit", "nodeName": "review",
            "model": "test-model", "pastAttempts": [], "executionId": "execution", "canAbort": true,
            "submitReceived": submit, "stopReceived": stop,
            "hasArtifact": true, "errorReason": "needs attention", "updatedAt": 124.0,
            "capabilities": {"canRename": true, "canApprove": true, "canRetry": false, "canResumeSession": true},
            "content": {"kind": "session", "sessionId": "session"}
        });
        if let Some(waiting) = waiting {
            expected["waitingFor"] = waiting.into();
        }
        assert_eq!(actual, expected);
    }
}
