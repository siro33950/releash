use super::agent_session::{
    AgentSessionArchiveError, AgentSessionAssociationError, AgentSessionCreationError,
    AgentSessionInitialInstructionError, AgentSessionRecoveryError,
    AgentSessionRemovalAuthorization, AgentSessionRemovalError, AgentSessionTreeParentError,
};
use super::{
    AgentSession, AgentSessionArchiveOutcome, AgentSessionInitialInstructionOutcome,
    AgentSessionLifecycle, AgentSessionLifecycleEvent, AgentSessionMutationOutcome,
    AgentSessionOpenAction, AgentSessionOperations, AgentSessionProcessExitOutcome,
    AgentSessionRecoveryResult, AgentSessionTreeParent, ManagedPtyPresence,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_agent_session生成_identityとproviderとtree_parentを固定してopenになる() {
    let workspace = WorkspaceIdentity::new("/repo");

    let session = AgentSession::create(
        "agent-session-1",
        workspace.clone(),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    assert_eq!(session.id(), "agent-session-1");
    assert_eq!(session.workspace(), &workspace);
    assert_eq!(session.worktree_path(), "/repo/.worktrees/feature");
    assert_eq!(session.provider(), ProviderKind::Claude);
    assert_eq!(session.tree_parent(), None);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
    assert_eq!(
        session.terminal_surface_owner(),
        TerminalSurfaceOwner::session(workspace, "agent-session-1").unwrap()
    );
}

#[test]
fn test_agent_session生成_永続化するcreated_eventを発生させる() {
    let workspace = WorkspaceIdentity::new("/repo");
    let session = AgentSession::create(
        "agent-session-1",
        workspace.clone(),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::Created {
            id: "agent-session-1".to_string(),
            workspace,
            worktree_path: "/repo/.worktrees/feature".to_string(),
            provider: ProviderKind::Claude,
            tree_parent: None,
        }]
    );
}

#[test]
fn test_agent_session_event_永続化へ渡したeventを未commit一覧から除く() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let events = session.take_uncommitted_events();

    assert_eq!(events.len(), 1);
    assert!(session.uncommitted_events().is_empty());
}

#[test]
fn test_agent_session生成_空のidentityを拒否する() {
    let error = AgentSession::create(
        " ",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap_err();

    assert_eq!(error, AgentSessionCreationError::Identity);
}

#[test]
fn test_agent_session生成_空のworkspaceを拒否する() {
    let error = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new(" "),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap_err();

    assert_eq!(error, AgentSessionCreationError::Workspace);
}

#[test]
fn test_agent_session生成_空のworktreeを拒否する() {
    let error = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        " ",
        ProviderKind::Claude,
        None,
    )
    .unwrap_err();

    assert_eq!(error, AgentSessionCreationError::Worktree);
}

#[test]
fn test_agent_session_restore_失敗時はarchivedを維持する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();

    let outcome = session
        .complete_restore(AgentSessionRecoveryResult::Failed)
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::AlreadyApplied);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Archived);
}

#[test]
fn test_agent_session_restore_成功時はopenへ遷移する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();

    let outcome = session
        .complete_restore(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::Applied);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_session_restore_成功時にopen_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();
    session.take_uncommitted_events();

    session
        .complete_restore(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Open,
            last_exit_abnormal: false,
        }]
    );
}

#[test]
fn test_agent_session_restore_archived以外では拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let error = session
        .complete_restore(AgentSessionRecoveryResult::Succeeded)
        .unwrap_err();

    assert_eq!(error, AgentSessionRecoveryError::NotArchived);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_session_resume_失敗時はpausedを維持する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.observe_provider_process_exit(Some(0));

    let outcome = session
        .complete_resume(AgentSessionRecoveryResult::Failed)
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::AlreadyApplied);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Paused);
}

#[test]
fn test_agent_session_resume_成功時はopenへ遷移する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.observe_provider_process_exit(Some(0));

    let outcome = session
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::Applied);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_session_resume_成功時にopen_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.observe_provider_process_exit(Some(0));
    session.take_uncommitted_events();

    session
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Open,
            last_exit_abnormal: false,
        }]
    );
}

#[test]
fn test_agent_session_resume_paused以外では拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let error = session
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap_err();

    assert_eq!(error, AgentSessionRecoveryError::NotPaused);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_sessionアーカイブ_親なしでprovider_session_id既知ならarchivedになる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let outcome = session.archive().unwrap();

    assert_eq!(outcome, AgentSessionArchiveOutcome::Archived);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Archived);
}

#[test]
fn test_agent_sessionアーカイブ_永続化するarchived_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.take_uncommitted_events();

    session.archive().unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Archived,
            last_exit_abnormal: false,
        }]
    );
}

#[test]
fn test_agent_sessionアーカイブ_archivedへの再要求は冪等になる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();

    let outcome = session.archive().unwrap();

    assert_eq!(outcome, AgentSessionArchiveOutcome::AlreadyArchived);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Archived);
}

#[test]
fn test_agent_sessionアーカイブ_pausedからarchivedへ遷移する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.observe_provider_process_exit(Some(0));

    let outcome = session.archive().unwrap();

    assert_eq!(outcome, AgentSessionArchiveOutcome::Archived);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Archived);
}

#[test]
fn test_agent_sessionアーカイブ_provider_session_id不明ならdelete確認を要求する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let outcome = session.archive().unwrap();

    assert_eq!(
        outcome,
        AgentSessionArchiveOutcome::DeleteConfirmationRequired
    );
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_sessionアーカイブ縮退_provider_session_id不明なら確認後deleteを許可する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let authorization = session.authorize_archive_fallback_delete().unwrap();

    assert_eq!(
        authorization,
        AgentSessionRemovalAuthorization::ArchiveFallbackDelete
    );
}

#[test]
fn test_agent_sessionアーカイブ縮退_provider_session_id既知なら拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let error = session.authorize_archive_fallback_delete().unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::ProviderSessionKnown);
}

#[test]
fn test_agent_sessionアーカイブ縮退_親を持つsessionでは拒否する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();

    let error = session.authorize_archive_fallback_delete().unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::WorkflowOwned);
}

#[test]
fn test_agent_sessionアーカイブ_親を持つsessionでは拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let error = session.archive().unwrap_err();

    assert_eq!(error, AgentSessionArchiveError::WorkflowOwned);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_session削除_archivedの親なしsessionだけを許可する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();

    let authorization = session.authorize_delete().unwrap();

    assert_eq!(
        authorization,
        AgentSessionRemovalAuthorization::ExplicitDelete
    );
}

#[test]
fn test_agent_session削除_openでは拒否する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let error = session.authorize_delete().unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::NotArchived);
}

#[test]
fn test_agent_session削除_親を持つsessionでは拒否する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();

    let error = session.authorize_delete().unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::WorkflowOwned);
}

#[test]
fn test_agent_session_workflow添付前rollback_親を持つsessionだけを許可する() {
    let workflow_session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();
    let parentless_session = AgentSession::create(
        "agent-session-2",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    assert_eq!(
        workflow_session
            .authorize_workflow_launch_rollback()
            .unwrap(),
        AgentSessionRemovalAuthorization::WorkflowLaunchRollback
    );
    assert_eq!(
        parentless_session
            .authorize_workflow_launch_rollback()
            .unwrap_err(),
        AgentSessionRemovalError::WithoutTreeParent
    );
}

#[test]
fn test_agent_session_gc_openでprovider_idなしpty不在確定なら許可する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let authorization = session
        .authorize_gc(ManagedPtyPresence::ConfirmedAbsent)
        .unwrap();

    assert_eq!(
        authorization,
        AgentSessionRemovalAuthorization::GarbageCollection
    );
}

#[test]
fn test_agent_session_gc_pty生死不明では拒否する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let error = session
        .authorize_gc(ManagedPtyPresence::Unknown)
        .unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::PtyNotConfirmedAbsent);
}

#[test]
fn test_agent_session_gc_live_ptyがあれば拒否する() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let error = session.authorize_gc(ManagedPtyPresence::Live).unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::PtyNotConfirmedAbsent);
}

#[test]
fn test_agent_session_gc_provider_session_id既知なら拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let error = session
        .authorize_gc(ManagedPtyPresence::ConfirmedAbsent)
        .unwrap_err();

    assert_eq!(error, AgentSessionRemovalError::ProviderSessionKnown);
}

#[test]
fn test_agent_session生成_実行木上の親を固定する() {
    let tree_parent =
        AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap();

    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(tree_parent.clone()),
    )
    .unwrap();

    assert_eq!(session.tree_parent(), Some(&tree_parent));
    assert_eq!(
        session.tree_parent().unwrap().tree_id,
        "workflow-execution-1"
    );
    assert_eq!(
        session.tree_parent().unwrap().node_execution_id,
        "node-execution-1"
    );
}

#[test]
fn test_agent_session生成_空のtree_idを拒否する() {
    let error = AgentSessionTreeParent::new(" ", "node-execution-1").unwrap_err();

    assert_eq!(error, AgentSessionTreeParentError::EmptyTreeId);
}

#[test]
fn test_agent_session生成_空のnode_execution_idを拒否する() {
    let error = AgentSessionTreeParent::new("workflow-execution-1", " ").unwrap_err();

    assert_eq!(error, AgentSessionTreeParentError::EmptyNodeExecutionId);
}

#[test]
fn test_agent_session_initial_instruction_親を持つsessionで最初のdispatchを受理する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();

    let outcome = session.admit_initial_instruction().unwrap();

    assert_eq!(outcome, AgentSessionInitialInstructionOutcome::Admitted);
}

#[test]
fn test_agent_session_initial_instruction_永続化するadmitted_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();
    session.take_uncommitted_events();

    session.admit_initial_instruction().unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::InitialInstructionAdmitted]
    );
}

#[test]
fn test_agent_session_initial_instruction_同じsessionへの再要求を冪等にする() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        Some(AgentSessionTreeParent::new("workflow-execution-1", "node-execution-1").unwrap()),
    )
    .unwrap();
    session.admit_initial_instruction().unwrap();
    session.take_uncommitted_events();

    let outcome = session.admit_initial_instruction().unwrap();

    assert_eq!(
        outcome,
        AgentSessionInitialInstructionOutcome::AlreadyAdmitted
    );
    assert!(session.uncommitted_events().is_empty());
}

#[test]
fn test_agent_session_initial_instruction_親なしでは拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();

    let error = session.admit_initial_instruction().unwrap_err();

    assert_eq!(
        error,
        AgentSessionInitialInstructionError::WithoutTreeParent
    );
}

#[test]
fn test_agent_session_process終了_provider_session_id既知ならpausedへ遷移する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let outcome = session.observe_provider_process_exit(Some(0));

    assert_eq!(outcome, AgentSessionProcessExitOutcome::Paused);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Paused);
}

#[test]
fn test_agent_session_process終了_永続化するpaused_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.take_uncommitted_events();

    session.observe_provider_process_exit(Some(0));

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Paused,
            last_exit_abnormal: false,
        }]
    );
}

#[test]
fn test_agent_session_process終了_pausedへの重複通知は冪等になる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.observe_provider_process_exit(Some(0));

    let outcome = session.observe_provider_process_exit(Some(0));

    assert_eq!(outcome, AgentSessionProcessExitOutcome::AlreadyPaused);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Paused);
}

#[test]
fn test_agent_session_process終了_archivedへの遅延通知は状態を変えない() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session.archive().unwrap();
    session.take_uncommitted_events();

    let outcome = session.observe_provider_process_exit(Some(0));

    assert_eq!(outcome, AgentSessionProcessExitOutcome::AlreadyArchived);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Archived);
    assert!(session.uncommitted_events().is_empty());
}

#[test]
fn test_agent_session_process終了_provider_session_id不明ならgcを要求する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let outcome = session.observe_provider_process_exit(Some(0));

    assert_eq!(outcome, AgentSessionProcessExitOutcome::GcRequired);
    assert_eq!(session.lifecycle(), AgentSessionLifecycle::Open);
}

#[test]
fn test_agent_session関連付け_異なるprovider_session_idへの差し替えを拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", Some("provider://transcript/1"))
        .unwrap();

    let error = session
        .associate_provider_session("provider-session-2", Some("provider://transcript/2"))
        .unwrap_err();

    assert_eq!(error, AgentSessionAssociationError::ProviderSessionMismatch);
    assert_eq!(session.provider_session_id(), Some("provider-session-1"));
    assert_eq!(session.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_agent_session関連付け_同じprovider_sessionとtranscriptの再送は冪等になる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", Some("provider://transcript/1"))
        .unwrap();
    session.take_uncommitted_events();

    let outcome = session
        .associate_provider_session("provider-session-1", Some("provider://transcript/1"))
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::AlreadyApplied);
    assert!(session.uncommitted_events().is_empty());
}

#[test]
fn test_agent_session関連付け_確定済みtranscript_refの差し替えを拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", Some("provider://transcript/1"))
        .unwrap();

    let error = session
        .associate_provider_session("provider-session-1", Some("provider://transcript/2"))
        .unwrap_err();

    assert_eq!(error, AgentSessionAssociationError::TranscriptMismatch);
    assert_eq!(session.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_agent_session関連付け_provider_session_idとopaque_transcriptだけを保持する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let outcome = session
        .associate_provider_session(
            "provider-session-1",
            Some("provider://opaque/transcript-reference"),
        )
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::Applied);
    assert_eq!(session.provider_session_id(), Some("provider-session-1"));
    assert_eq!(
        session.transcript_ref(),
        Some("provider://opaque/transcript-reference")
    );
}

#[test]
fn test_agent_session関連付け_永続化するassociated_eventを発生させる() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session.take_uncommitted_events();

    session
        .associate_provider_session(
            "provider-session-1",
            Some("provider://opaque/transcript-reference"),
        )
        .unwrap();

    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::ProviderSessionAssociated {
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: Some("provider://opaque/transcript-reference".to_string()),
        }]
    );
}

#[test]
fn test_agent_session関連付け_空のprovider_session_idを拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let error = session.associate_provider_session("  ", None).unwrap_err();

    assert_eq!(error, AgentSessionAssociationError::EmptyProviderSessionId);
    assert_eq!(session.provider_session_id(), None);
}

#[test]
fn test_agent_session関連付け_空のtranscript_refを拒否する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();

    let error = session
        .associate_provider_session("provider-session-1", Some(" "))
        .unwrap_err();

    assert_eq!(
        error,
        AgentSessionAssociationError::EmptyTranscriptReference
    );
    assert_eq!(session.provider_session_id(), None);
    assert_eq!(session.transcript_ref(), None);
}

#[test]
fn test_agent_session_open判断_pty状態とlifecycleから唯一の操作を返す() {
    let mut open = AgentSession::create(
        "agent-session-open",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    assert_eq!(
        open.open_action(ManagedPtyPresence::Live),
        AgentSessionOpenAction::Attach
    );
    assert_eq!(
        open.open_action(ManagedPtyPresence::Unknown),
        AgentSessionOpenAction::Indeterminate
    );
    assert_eq!(
        open.open_action(ManagedPtyPresence::ConfirmedAbsent),
        AgentSessionOpenAction::GarbageCollect
    );

    open.associate_provider_session("provider-session-1", None)
        .unwrap();
    assert_eq!(
        open.open_action(ManagedPtyPresence::ConfirmedAbsent),
        AgentSessionOpenAction::Resume
    );

    open.observe_provider_process_exit(Some(0));
    assert_eq!(
        open.open_action(ManagedPtyPresence::ConfirmedAbsent),
        AgentSessionOpenAction::RemainPaused
    );

    open.complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();
    open.archive().unwrap();
    assert_eq!(
        open.open_action(ManagedPtyPresence::ConfirmedAbsent),
        AgentSessionOpenAction::Restore
    );
}

#[test]
fn test_agent_session操作表示_tree_parentとlifecycleの規則をdomainだけが決める() {
    assert_eq!(
        AgentSessionOperations::for_state(None, AgentSessionLifecycle::Open),
        AgentSessionOperations {
            can_archive: true,
            can_restore: false,
            can_delete: false,
        }
    );
    assert_eq!(
        AgentSessionOperations::for_state(None, AgentSessionLifecycle::Archived),
        AgentSessionOperations {
            can_archive: false,
            can_restore: true,
            can_delete: true,
        }
    );
    assert_eq!(
        AgentSessionOperations::for_state(
            Some(&AgentSessionTreeParent::new("workflow-1", "node-1").unwrap()),
            AgentSessionLifecycle::Open,
        ),
        AgentSessionOperations {
            can_archive: false,
            can_restore: false,
            can_delete: false,
        }
    );
}

#[test]
fn test_agent_session復帰開始_resumeとrestoreの受理状態をdomainが判定する() {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    assert_eq!(
        session.authorize_resume().unwrap_err(),
        AgentSessionRecoveryError::NotPaused
    );
    assert_eq!(
        session.authorize_restore().unwrap_err(),
        AgentSessionRecoveryError::NotArchived
    );

    session.observe_provider_process_exit(Some(0));
    assert!(session.authorize_resume().is_ok());

    session
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();
    session.archive().unwrap();
    assert!(session.authorize_restore().is_ok());
}

fn paused_candidate() -> AgentSession {
    let mut session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        None,
    )
    .unwrap();
    session
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    session
}

#[test]
fn test_agent_session_process終了_非0のexit_codeをlast_exit_abnormalとして記録する() {
    let mut session = paused_candidate();
    session.take_uncommitted_events();

    session.observe_provider_process_exit(Some(1));

    assert!(session.last_exit_abnormal());
    assert_eq!(
        session.uncommitted_events(),
        &[AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Paused,
            last_exit_abnormal: true,
        }]
    );
}

#[test]
fn test_agent_session_process終了_exit_code_0は正常終了として記録する() {
    let mut session = paused_candidate();

    session.observe_provider_process_exit(Some(0));

    assert!(!session.last_exit_abnormal());
}

#[test]
fn test_agent_session_process終了_exit_code不明は異常終了として記録する() {
    let mut session = paused_candidate();

    session.observe_provider_process_exit(None);

    assert!(session.last_exit_abnormal());
}

#[test]
fn test_agent_session_resume成功時にlast_exit_abnormalを解除する() {
    let mut session = paused_candidate();
    session.observe_provider_process_exit(Some(137));
    assert!(session.last_exit_abnormal());

    session
        .complete_resume(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert!(!session.last_exit_abnormal());
}

#[test]
fn test_agent_session_restore成功時にlast_exit_abnormalを解除する() {
    let mut session = paused_candidate();
    session.observe_provider_process_exit(Some(137));
    session.archive().unwrap();
    assert!(session.last_exit_abnormal());

    session
        .complete_restore(AgentSessionRecoveryResult::Succeeded)
        .unwrap();

    assert!(!session.last_exit_abnormal());
}
