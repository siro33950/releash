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
        execution_parent: None,
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: NodeCompletionSignalState::Pending,
        has_artifact: false,
        session_id: (kind == WorkspaceNodeKind::WorkflowSession).then(|| "session".into()),
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
fn test_表示状態分類_完了済みsessionのworkingは青() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.process_presence = NodeProcessPresence::Live;
    session.activity = Some(AgentSessionActivity::Working);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_完了済みsessionの回答待ちは黄() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.process_presence = NodeProcessPresence::Live;
    session.activity = Some(AgentSessionActivity::AwaitingAnswer);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_完了済みsessionのstopは緑() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.process_presence = NodeProcessPresence::Live;
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    // When
    let status = session.classify_status([]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
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
fn test_表示状態分類_完了済みsessionの記録上awaiting_answerでもプロセス消失なら緑() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    );
    session.activity = Some(AgentSessionActivity::AwaitingAnswer);
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
fn test_表示状態分類_delegate親は動作中の子を青として反映する() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.delegate_waits_for_child = true;
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    // When
    let status = session.classify_status([WorkspaceNodeStatusClassification::Active]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_delegate親は回答待ちの子を黄として反映する() {
    // Given
    let mut session = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Running,
    );
    session.delegate_waits_for_child = true;
    session.activity = Some(AgentSessionActivity::AwaitingInstruction);
    // When
    let status = session.classify_status([WorkspaceNodeStatusClassification::Attention]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
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
fn test_表示状態分類_sequenceの青い子は青として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Sequence, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([WorkspaceNodeStatusClassification::Active]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_sequenceの緑の子は緑として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Sequence, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([WorkspaceNodeStatusClassification::Idle]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
}

#[test]
fn test_表示状態分類_sequenceの子に黄があれば黄として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Sequence, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([
        WorkspaceNodeStatusClassification::Active,
        WorkspaceNodeStatusClassification::Attention,
    ]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_fanoutの青い子は青として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Fanout, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([WorkspaceNodeStatusClassification::Active]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Active);
}

#[test]
fn test_表示状態分類_fanoutの緑の子は緑として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Fanout, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([WorkspaceNodeStatusClassification::Idle]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Idle);
}

#[test]
fn test_表示状態分類_fanoutの子に黄があれば黄として反映する() {
    // Given
    let branch = node(WorkspaceNodeKind::Fanout, WorkspaceNodeStatus::Completed);
    // When
    let status = branch.classify_status([
        WorkspaceNodeStatusClassification::Active,
        WorkspaceNodeStatusClassification::Attention,
    ]);
    // Then
    assert_eq!(status, WorkspaceNodeStatusClassification::Attention);
}

#[test]
fn test_表示状態分類_青の公開値はactive() {
    // Given
    let classification = WorkspaceNodeStatusClassification::Active;
    // When
    let value = classification.as_public_str();
    // Then
    assert_eq!(value, "active");
}

#[test]
fn test_表示状態分類_黄の公開値はattention() {
    // Given
    let classification = WorkspaceNodeStatusClassification::Attention;
    // When
    let value = classification.as_public_str();
    // Then
    assert_eq!(value, "attention");
}

#[test]
fn test_表示状態分類_緑の公開値はidle() {
    // Given
    let classification = WorkspaceNodeStatusClassification::Idle;
    // When
    let value = classification.as_public_str();
    // Then
    assert_eq!(value, "idle");
}

#[test]
fn test_command失敗理由_非ゼロ終了を示し記録された理由を優先する() {
    // Given
    let mut command = node(
        WorkspaceNodeKind::WorkflowCommand,
        WorkspaceNodeStatus::Completed,
    );
    command.command_result = Some(WorkspaceCommandResult {
        exit_code: 0,
        duration: 1,
        stdout: "out".into(),
        stderr: "err".into(),
    });
    // When / Then
    assert_eq!(command.failure_reason(), None);
    command.command_result.as_mut().unwrap().exit_code = 2;
    assert_eq!(
        command.failure_reason().as_deref(),
        Some("Command exited with code 2")
    );
    command.error_reason = Some("process failed".into());
    assert_eq!(command.failure_reason().as_deref(), Some("process failed"));
}

#[test]
fn test_部分木の状態分類_子の状態と失敗を反映し過去の試行を含めない() {
    // Given
    let mut parent = node(
        WorkspaceNodeKind::WorkflowSession,
        WorkspaceNodeStatus::Completed,
    )
    .classification_input();
    parent.id = "parent".into();
    parent.activity = Some(AgentSessionActivity::AwaitingInstruction);
    parent.has_session = true;
    let mut child = parent.clone();
    child.id = "child".into();
    child.parent_id = Some("parent".into());
    // When / Then
    for (activity, expected) in [
        (
            AgentSessionActivity::Working,
            WorkspaceNodeStatusClassification::Active,
        ),
        (
            AgentSessionActivity::AwaitingAnswer,
            WorkspaceNodeStatusClassification::Attention,
        ),
    ] {
        child.activity = Some(activity);
        assert_eq!(
            WorkspaceNodeClassification::aggregate(&[parent.clone(), child.clone()], "parent"),
            expected
        );
    }
    // When
    child.activity = Some(AgentSessionActivity::AwaitingInstruction);
    child.background_failure = true;
    // Then
    assert_eq!(
        WorkspaceNodeClassification::aggregate(&[parent.clone(), child.clone()], "parent"),
        WorkspaceNodeStatusClassification::Attention
    );
    // When
    child.hidden = true;
    // Then
    assert_eq!(
        WorkspaceNodeClassification::aggregate(&[parent, child], "parent"),
        WorkspaceNodeStatusClassification::Idle
    );
}

#[test]
fn test_node更新時刻_開始と成果と承認待ちと終端に共通の時刻規則を使う() {
    use crate::domain::workflow::NodeExecutionStatus as S;
    for (status, completed, artifact, expected) in [
        (S::Running, None, false, 1.0),
        (S::Running, None, true, 5.0),
        (S::WaitingApproval, None, false, 5.0),
        (S::Succeeded, Some(3.0), true, 3.0),
        (S::Succeeded, None, false, 5.0),
        (S::Aborted, Some(4.0), false, 4.0),
    ] {
        assert_eq!(
            super::node_updated_at(1.0, completed, status, artifact, 5.0),
            expected
        );
    }
}

#[test]
fn test_背景失敗の観測_自身と実行の理由と分類を詳細入力と同じ操作で反映する() {
    // Given
    for target in ["node", "node-execution", "workflow", "unrelated"] {
        let mut listed = node(
            WorkspaceNodeKind::WorkflowSession,
            WorkspaceNodeStatus::Completed,
        );
        listed.error_reason = Some("existing".into());
        let mut input = listed.classification_input();
        let mut reason = listed.error_reason.clone();
        // When
        if let Some(observed) = input.observe_background_failure(target, "background") {
            reason = Some(observed.into());
        }
        if input.observes(target) {
            listed.observe_background_failure("background");
        }
        // Then
        assert_eq!(input.background_failure, listed.background_failure);
        assert_eq!(input.classify([]), listed.classify_status([]));
        assert_eq!(reason, listed.error_reason);
        assert_eq!(
            reason.as_deref(),
            Some(if target == "unrelated" {
                "existing"
            } else {
                "background"
            })
        );
    }
}
