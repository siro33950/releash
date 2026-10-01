use std::sync::Arc;

use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::agent_session::ProviderAgentTerminalGateway;
use crate::domain::agent_session::ProviderAgentTerminalSpawnError;
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
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::default(),
    );
    if let Some(surface) = surface {
        gateway.insert_surface(surface);
    }
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway,
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
    )
}

#[test]
fn test_provider_agent_terminal_spawn_error_残る分類とpayloadを保持する() {
    let cases = [
        (
            crate::usecase::terminal_surface::error::UsecaseError::OwnerConflict,
            ProviderAgentTerminalSpawnError::OwnerConflict,
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "openpty failed".to_string(),
                },
            ),
            ProviderAgentTerminalSpawnError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: "openpty failed".to_string(),
            }),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "checkpoint failed".to_string(),
                },
            ),
            ProviderAgentTerminalSpawnError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: "checkpoint failed".to_string(),
            }),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::InvalidOperation(
                "runtime is shutting down".to_string(),
            ),
            ProviderAgentTerminalSpawnError::InvalidOperation("runtime is shutting down".into()),
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(super::map_spawn_error(source), expected);
    }
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
fn test_terminal失敗_全ての性質とメッセージを操作と起動で保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    use crate::usecase::terminal_surface::error::UsecaseError;
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        // Given
        let failure = TechnicalFailure {
            nature,
            message: "terminal failure".into(),
        };
        // When
        let operation = super::map_terminal_error(UsecaseError::Technical(failure.clone()));
        let spawn = super::map_spawn_error(UsecaseError::Technical(failure.clone()));
        // Then
        assert_eq!(
            operation,
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                failure.clone()
            )
        );
        assert_eq!(spawn, ProviderAgentTerminalSpawnError::Technical(failure));
    }
}
