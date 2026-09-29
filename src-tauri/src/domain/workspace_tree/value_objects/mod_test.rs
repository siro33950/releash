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
fn test_表示状態分類_完了済みworkflow_sessionもagent状態を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    // When / Then
    for (activity, presence, expected) in [
        (
            AgentSessionActivity::Working,
            NodeProcessPresence::Live,
            C::Active,
        ),
        (
            AgentSessionActivity::AwaitingAnswer,
            NodeProcessPresence::Live,
            C::Attention,
        ),
        (
            AgentSessionActivity::AwaitingInstruction,
            NodeProcessPresence::Live,
            C::Idle,
        ),
        (
            AgentSessionActivity::Working,
            NodeProcessPresence::ConfirmedAbsent,
            C::Idle,
        ),
        (
            AgentSessionActivity::AwaitingAnswer,
            NodeProcessPresence::ConfirmedAbsent,
            C::Idle,
        ),
    ] {
        session.activity = Some(activity);
        session.process_presence = presence;
        assert_eq!(
            session.classify_status([]),
            expected,
            "{activity:?}, {presence:?}"
        );
    }
}

#[test]
fn test_表示状態分類_稼働中のsessionはstopとプロセス消失で黄() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    // When / Then
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    session.completion_signals = NodeCompletionSignalState::StopReceived;
    assert_eq!(session.classify_status([]), C::Attention);
    session.activity = Some(AgentSessionActivity::Working);
    session.process_presence = NodeProcessPresence::ConfirmedAbsent;
    assert_eq!(session.classify_status([]), C::Attention);
}

#[test]
fn test_表示状態分類_承認待ちと失敗は黄() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Waiting,
    );
    session.activity = Some(AgentSessionActivity::Working);
    // When / Then
    assert_eq!(session.classify_status([]), C::Attention);
    session.status = WorkspaceNodeStatus::Completed;
    session.background_failure = true;
    assert_eq!(session.classify_status([]), C::Attention);
}

#[test]
fn test_表示状態分類_delegate親は最重の子を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.delegate_waits_for_child = true;
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    // When / Then
    assert_eq!(session.classify_status([C::Active]), C::Active);
    assert_eq!(session.classify_status([C::Attention]), C::Attention);
}

#[test]
fn test_表示状態分類_commandと親は最重の子を反映する() {
    use WorkspaceNodeStatusClassification as C;
    // Given
    let mut command = node(
        WorkspaceNodeKind::WorkflowCommand,
        WorkspaceNodeStatus::Running,
    );
    // When / Then
    assert_eq!(command.classify_status([]), C::Active);
    command.process_presence = NodeProcessPresence::ConfirmedAbsent;
    assert_eq!(command.classify_status([]), C::Attention);
    command.status = WorkspaceNodeStatus::Completed;
    assert_eq!(command.classify_status([]), C::Idle);
    for kind in [WorkspaceNodeKind::Sequence, WorkspaceNodeKind::Fanout] {
        let branch = node(kind, WorkspaceNodeStatus::Completed);
        assert_eq!(
            branch.classify_status([C::Active, C::Attention]),
            C::Attention
        );
        assert_eq!(branch.classify_status([C::Active]), C::Active);
        assert_eq!(branch.classify_status([C::Idle]), C::Idle);
    }
}

#[test]
fn test_表示状態分類_公開値は黄青緑の3種類() {
    // Given / When / Then
    assert_eq!(
        WorkspaceNodeStatusClassification::Active.as_public_str(),
        "active"
    );
    assert_eq!(
        WorkspaceNodeStatusClassification::Attention.as_public_str(),
        "attention"
    );
    assert_eq!(
        WorkspaceNodeStatusClassification::Idle.as_public_str(),
        "idle"
    );
}
