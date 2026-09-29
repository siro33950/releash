use super::*;

fn node(kind: WorkspaceNodeKind, status: WorkspaceNodeStatus) -> WorkspaceTreeNode {
    WorkspaceTreeNode {
        process_presence: NodeProcessPresence::Unknown,
        worktree: None,
        id: "node".into(),
        parent_id: Some("workflow".into()),
        sibling_order: 0,
        kind,
        title: "node".into(),
        status,
        status_classification: WorkspaceNodeStatusClassification::Idle,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: None,
        error_reason: None,
        updated_at_bits: 1.0_f64.to_bits(),
        execution_id: Some("workflow".into()),
        node_execution_id: Some("node-execution".into()),
        node_name: Some("node".into()),
        attempt: Some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: NodeCompletionSignalState::Pending,
        has_artifact: false,
        session_id: (kind == WorkspaceNodeKind::WorkflowSession).then(|| "session".into()),
        can_rename: false,
        can_approve: false,
        can_retry: false,
        can_resume_session: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

#[test]
fn test_表示状態分類_紐づく前のsessionは青() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.session_id = None;
    session.process_presence = NodeProcessPresence::ConfirmedAbsent;
    session.activity = Some(AgentSessionActivity::AwaitingAnswer);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, C::Active);
}

#[test]
fn test_表示状態分類_実行中の紐づき済みsessionがworkingなら青() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.process_presence = NodeProcessPresence::Live;
    session.activity = Some(AgentSessionActivity::Working);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_実行中の紐づき済みsessionがawaiting_answerなら黄() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.process_presence = NodeProcessPresence::Live;
    session.activity = Some(AgentSessionActivity::AwaitingAnswer);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_完了済みsessionもagent状態を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.process_presence = NodeProcessPresence::Live;
    // When
    let cases = [
        ("working", AgentSessionActivity::Working, C::Active),
        (
            "awaiting answer",
            AgentSessionActivity::AwaitingAnswer,
            C::Attention,
        ),
        (
            "stopped",
            AgentSessionActivity::AwaitingInstruction,
            C::Idle,
        ),
    ];
    // Then
    for (name, activity, expected) in cases {
        session.activity = Some(activity);
        assert_eq!(session.classify_status([]), expected, "{name}");
    }
}

#[test]
fn test_表示状態分類_完了済みsessionの記録上workingでもプロセス消失なら緑() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.activity = Some(AgentSessionActivity::Working);
    session.process_presence = NodeProcessPresence::ConfirmedAbsent;
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
}

#[test]
fn test_表示状態分類_実行中sessionの記録上workingでもプロセス消失なら黄() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.activity = Some(AgentSessionActivity::Working);
    session.process_presence = NodeProcessPresence::ConfirmedAbsent;
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_中断済みsessionがstopして子がなければ緑() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Aborted,
    );
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    session.process_presence = NodeProcessPresence::Live;
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
}

#[test]
fn test_表示状態分類_実行中sessionがstopしてsubmitがなければ黄() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    session.completion_signals = NodeCompletionSignalState::StopReceived;
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_承認待ちは黄() {
    // Given
    let session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Waiting,
    );
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_裏の失敗は黄() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.background_failure = true;
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_delegate親は子の色を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.delegate_waits_for_child = true;
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    // When
    let cases = [
        ("working child", C::Active),
        ("answer waiting child", C::Attention),
    ];
    // Then
    for (name, child) in cases {
        assert_eq!(session.classify_status([child]), child, "{name}");
    }
}

#[test]
fn test_表示状態分類_実行中commandは青() {
    // Given
    let command = node(
        WorkspaceNodeKind::WorkflowCommand,
        WorkspaceNodeStatus::Running,
    );
    // When
    let status = command.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_実行中commandのプロセス消失は黄() {
    // Given
    let mut command = node(
        WorkspaceNodeKind::WorkflowCommand,
        WorkspaceNodeStatus::Running,
    );
    command.process_presence = NodeProcessPresence::ConfirmedAbsent;
    // When
    let status = command.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_完了済みcommandは緑() {
    // Given
    let command = node(
        WorkspaceNodeKind::WorkflowCommand,
        WorkspaceNodeStatus::Completed,
    );
    // When
    let status = command.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
}

#[test]
fn test_表示状態分類_sequenceとfanoutは最も重い子を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let kinds = [
        ("sequence", WorkspaceNodeKind::Sequence),
        ("fanout", WorkspaceNodeKind::Fanout),
    ];
    let children = [
        ("blue only", vec![C::Active], C::Active),
        ("green only", vec![C::Idle], C::Idle),
        (
            "includes yellow",
            vec![C::Active, C::Attention],
            C::Attention,
        ),
    ];
    for (kind_name, kind) in kinds {
        let branch = node(kind, WorkspaceNodeStatus::Completed);
        for (case_name, child_statuses, expected) in &children {
            // When
            let status = branch.classify_status(child_statuses.iter().copied());
            // Then
            assert_eq!(status, *expected, "{kind_name}: {case_name}");
        }
    }
}

#[test]
fn test_表示状態分類_公開値は黄青緑の3種類() {
    // Given
    let cases = [
        (
            "active",
            WorkspaceNodeStatusClassification::Active,
            "active",
        ),
        (
            "attention",
            WorkspaceNodeStatusClassification::Attention,
            "attention",
        ),
        ("idle", WorkspaceNodeStatusClassification::Idle, "idle"),
    ];
    // When
    let values = cases
        .map(|(name, classification, expected)| (name, classification.as_public_str(), expected));
    // Then
    for (name, actual, expected) in values {
        assert_eq!(actual, expected, "{name}");
    }
}
