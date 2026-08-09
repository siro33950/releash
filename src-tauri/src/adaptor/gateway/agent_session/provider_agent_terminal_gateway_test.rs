use std::sync::Arc;

use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::agent_session::ProviderAgentTerminalGateway;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::domain::terminal_surface::{
    TerminalProcessState, TerminalSurfaceCheckpoint, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

fn surface_for(
    owner: &TerminalSurfaceOwner,
    registered_owner: TerminalSurfaceOwner,
    process_state: TerminalProcessState,
) -> TerminalSurface {
    TerminalSurface {
        session_key: owner.stable_key(),
        owner: registered_owner,
        worktree_path: Some("/repo".to_string()),
        label: None,
        runtime_generation: 1.into(),
        process_state,
        checkpoint: TerminalSurfaceCheckpoint::empty(80, 24),
        latest_sequence: 0,
        last_output_at: None,
    }
}

fn application_with(surface: Option<TerminalSurface>) -> TerminalSurfaceApplication {
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
    );
    if let Some(surface) = surface {
        gateway.insert_surface(surface);
    }
    TerminalSurfaceApplication::new(gateway, Arc::new(TerminalSurfaceEventHub::new()))
}

#[test]
fn test_provider_agent_terminal_presence_未登録は不在確定を返す() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let application = application_with(None);

    assert_eq!(
        application.presence(&owner),
        Ok(ManagedPtyPresence::ConfirmedAbsent)
    );
}

#[test]
fn test_provider_agent_terminal_presence_owner不整合は不在確定にしない() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let registered_owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/other-repo"), "agent-session-1")
            .unwrap();
    let application = application_with(Some(surface_for(
        &owner,
        registered_owner,
        TerminalProcessState::Running,
    )));

    assert_eq!(
        application.presence(&owner),
        Ok(ManagedPtyPresence::Unknown)
    );
}

#[test]
fn test_provider_agent_terminal_presence_稼働中は生存を返し終了後は不在確定を返す() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let running = application_with(Some(surface_for(
        &owner,
        owner.clone(),
        TerminalProcessState::Running,
    )));
    assert_eq!(running.presence(&owner), Ok(ManagedPtyPresence::Live));

    let exited = application_with(Some(surface_for(
        &owner,
        owner.clone(),
        TerminalProcessState::Exited { exit_code: Some(0) },
    )));
    assert_eq!(
        exited.presence(&owner),
        Ok(ManagedPtyPresence::ConfirmedAbsent)
    );
}

#[test]
fn test_provider_agent_terminal_activity_直近出力ありのsurfaceをrunningに分類する() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let mut surface = surface_for(&owner, owner.clone(), TerminalProcessState::Running);
    surface.last_output_at = Some(std::time::Instant::now());
    let application = application_with(Some(surface));

    assert_eq!(
        crate::domain::agent_session::ProviderAgentTerminalObservationGateway::session_activity(
            &application,
            &owner
        ),
        crate::domain::terminal_surface::TerminalActivity::Running
    );
}

#[test]
fn test_provider_agent_terminal_activity_出力recencyがないsurfaceをidleに分類する() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let application = application_with(Some(surface_for(
        &owner,
        owner.clone(),
        TerminalProcessState::Running,
    )));

    assert_eq!(
        crate::domain::agent_session::ProviderAgentTerminalObservationGateway::session_activity(
            &application,
            &owner
        ),
        crate::domain::terminal_surface::TerminalActivity::Idle
    );
}

#[test]
fn test_provider_agent_terminal_activity_終了済みsurfaceは出力直後でもidleに分類する() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let mut surface = surface_for(
        &owner,
        owner.clone(),
        TerminalProcessState::Exited { exit_code: Some(1) },
    );
    surface.last_output_at = Some(std::time::Instant::now());
    let application = application_with(Some(surface));

    assert_eq!(
        crate::domain::agent_session::ProviderAgentTerminalObservationGateway::session_activity(
            &application,
            &owner
        ),
        crate::domain::terminal_surface::TerminalActivity::Idle
    );
}

#[test]
fn test_provider_agent_terminal_exit_code_終了済みsurfaceのexit_codeを返す() {
    use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;

    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let application = application_with(Some(surface_for(
        &owner,
        owner.clone(),
        TerminalProcessState::Exited {
            exit_code: Some(137),
        },
    )));

    assert_eq!(application.session_exit_code(&owner), Some(137));
    assert_eq!(
        application.exited_session_owners(),
        vec![(1, owner.clone(), Some(137))]
    );
}

#[test]
fn test_provider_agent_terminal_worktree解決_session所有surfaceだけを対象にする() {
    use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;

    let session_owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let session_backed = application_with(Some(surface_for(
        &session_owner,
        session_owner.clone(),
        TerminalProcessState::Running,
    )));
    assert_eq!(
        session_backed.session_worktree_path(&session_owner.stable_key()),
        Some("/repo".to_string())
    );

    let workspace_owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let workspace_backed = application_with(Some(surface_for(
        &workspace_owner,
        workspace_owner.clone(),
        TerminalProcessState::Running,
    )));
    assert_eq!(
        workspace_backed.session_worktree_path(&workspace_owner.stable_key()),
        None
    );
}
