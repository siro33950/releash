use std::sync::Arc;

use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::agent_session::ProviderAgentTerminalGateway;
use crate::domain::agent_session::ProviderAgentTerminalGatewayError;
use crate::domain::agent_session::ProviderAgentTerminalInputGateway;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::domain::terminal_surface::{
    TerminalProcessState, TerminalSurfaceCheckpoint, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;

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
    application_with_gateway(gateway)
}

fn application_with_gateway(
    gateway: Arc<dyn TerminalSurfaceGateway + Send + Sync>,
) -> TerminalSurfaceApplication {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway,
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
        crate::test_support::terminal_config(""),
    )
}

#[test]
fn test_指示入力_複数行を包んで一回の書き込みで確定する() {
    // Given
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let gateway = Arc::new(FakePtyGateway::new());
    let application = application_with_gateway(gateway.clone());

    // When
    application
        .write_instruction(&owner, "  修正してください\n次の行\r\n末尾の空白  ")
        .unwrap();

    // Then
    assert_eq!(
        *gateway.writes.lock(),
        vec![(
            owner.stable_key(),
            "\u{1b}[200~  修正してください\n次の行\n末尾の空白  \u{1b}[201~\r".to_string()
        )]
    );
}

#[test]
fn test_指示入力_本文の制御文字を除き改行とtabを保持する() {
    // Given
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let gateway = Arc::new(FakePtyGateway::new());
    let application = application_with_gateway(gateway.clone());
    let instruction = "本文\u{1b}[201~\r操作\u{0}\u{7}\u{8}\u{7f}\u{85}\u{9b}\n\t続き";

    // When
    application.write_instruction(&owner, instruction).unwrap();

    // Then
    let writes = gateway.writes.lock();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, owner.stable_key());
    let content = writes[0]
        .1
        .strip_prefix("\u{1b}[200~")
        .unwrap()
        .strip_suffix("\u{1b}[201~\r")
        .unwrap();
    assert_eq!(content, "本文[201~操作\n\t続き");
    assert!(content
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\t')));
}

#[test]
fn test_指示入力_末尾の改行を除いて確定用の改行を一つ付ける() {
    for instruction in [
        "修正してください\r",
        "修正してください\n",
        "修正してください\r\n",
        "修正してください\r\n\n\r",
    ] {
        // Given
        let owner =
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1")
                .unwrap();
        let gateway = Arc::new(FakePtyGateway::new());
        let application = application_with_gateway(gateway.clone());

        // When
        application.write_instruction(&owner, instruction).unwrap();

        // Then
        assert_eq!(
            *gateway.writes.lock(),
            vec![(
                owner.stable_key(),
                "\u{1b}[200~修正してください\u{1b}[201~\r".to_string()
            )],
            "instruction: {instruction:?}"
        );
    }
}

#[test]
fn test_指示入力_書き込み失敗をgatewayのエラーとして返す() {
    // Given
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let application = application_with(None);

    // When
    let result = application.write_instruction(&owner, "修正してください");

    // Then
    assert_eq!(
        result,
        Err(ProviderAgentTerminalGatewayError::NotFound(format!(
            "Terminal Surface not found for owner {}",
            owner.stable_key()
        )))
    );
}

#[test]
fn test_provider_agent_terminal_spawn_error_残る分類とpayloadを保持する() {
    let cases = [
        (
            crate::usecase::terminal_surface::error::UsecaseError::AppConfig(
                crate::domain::app_config::AppConfigError::Repository("config unavailable".into()),
            ),
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "config unavailable".into(),
                },
            ),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::AppConfig(
                crate::domain::app_config::AppConfigError::InvalidInput("invalid config".into()),
            ),
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "invalid config".into(),
                },
            ),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::OwnerConflict,
            ProviderAgentTerminalGatewayError::OwnerConflict,
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "openpty failed".to_string(),
                },
            ),
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "openpty failed".to_string(),
                },
            ),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "checkpoint failed".to_string(),
                },
            ),
            ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "checkpoint failed".to_string(),
                },
            ),
        ),
        (
            crate::usecase::terminal_surface::error::UsecaseError::InvalidOperation(
                "runtime is shutting down".to_string(),
            ),
            ProviderAgentTerminalGatewayError::InvalidOperation("runtime is shutting down".into()),
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(super::map_terminal_error(source), expected);
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
        let spawn = super::map_terminal_error(UsecaseError::Technical(failure.clone()));
        // Then
        assert_eq!(
            operation,
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                failure.clone()
            )
        );
        assert_eq!(spawn, ProviderAgentTerminalGatewayError::Technical(failure));
    }
}
