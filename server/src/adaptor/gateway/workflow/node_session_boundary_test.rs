use super::*;
use crate::domain::agent_session::ProviderAgentTerminalGatewayError;

#[test]
fn test_session起動停止_分類をruntimeまで保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = activation_error(
            "session",
            AgentSessionLaunchUsecaseError::Technical(stopped.into()),
        );
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
    }
}

#[test]
fn test_session依存先の技術的失敗_性質とメッセージをruntimeまで保持する() {
    use crate::domain::agent_session::{
        ProviderAgentLaunchGatewayError, ProviderAgentTerminalGatewayError,
    };
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    use crate::usecase::agent_session::AgentSessionLifecycleUsecaseError;
    // Given
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "source failure".into(),
        };
        let launch = ProviderAgentLaunchGatewayError::Technical(failure.clone());
        let terminal = ProviderAgentTerminalGatewayError::Technical(failure.clone());
        let spawn = ProviderAgentTerminalGatewayError::Technical(failure.clone());
        // When
        let errors = [
            launch_failure(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLaunchUsecaseError::Launch(launch.clone()),
            ),
            launch_failure(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLaunchUsecaseError::Terminal(terminal.clone()),
            ),
            launch_failure(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLaunchUsecaseError::Terminal(spawn.clone()),
            ),
            lifecycle_error(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLifecycleUsecaseError::Launch(launch),
            ),
            lifecycle_error(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLifecycleUsecaseError::Terminal(terminal),
            ),
            lifecycle_error(
                "launch Workflow AgentSession for NodeExecution 'node-1'".into(),
                AgentSessionLifecycleUsecaseError::Terminal(spawn),
            ),
        ];
        // Then
        for error in errors {
            assert!(
                matches!(error, WorkflowRuntimeError::Technical(actual) if actual.nature == failure.nature && actual.message == format!("launch Workflow AgentSession for NodeExecution 'node-1': {}", failure.message) && actual.message.ends_with(&failure.message))
            );
        }
    }
}

#[test]
fn test_session起動要求_workspaceと隔離cwdをそれぞれの項目に写す() {
    // Given
    let config = WorkflowSessionLaunchConfig {
        provider: ProviderKind::Codex,
        model: Some("model".into()),
        permission: Some(crate::domain::workflow::SessionPermission::Auto),
    };
    // When
    let request = workflow_launch_request(
        "/repo-worktrees/development",
        "/repo-worktrees/.releash-isolated/node-a1",
        config.clone(),
        "execution",
        "node",
        "implement",
    );
    // Then
    assert_eq!(request.workspace.as_str(), "/repo-worktrees/development");
    assert_eq!(
        request.worktree_path,
        "/repo-worktrees/.releash-isolated/node-a1"
    );
    assert_eq!(request.provider, config.provider);
    assert_eq!(request.model, config.model);
    assert_eq!(request.permission, config.permission);
    assert_eq!(request.workflow_execution_id, "execution");
    assert_eq!(request.node_execution_id, "node");
    assert_eq!(request.initial_instruction, "implement");
}

#[test]
fn test_workflow_agent_session_activation_terminal_spawn分類をcontext付きで保持する() {
    let error = activation_error(
        "agent-session-1",
        AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: "openpty failed".to_string(),
            },
        )),
    );

    assert_eq!(
        error.to_string(),
        "activate Workflow AgentSession 'agent-session-1': openpty failed"
    );
}

#[test]
fn test_workflow_agent_session_activation_terminal以外の既存表現を維持する() {
    let error = activation_error(
        "agent-session-1",
        AgentSessionLaunchUsecaseError::Launch(
            crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ),
        ),
    );

    assert_eq!(
        error.to_string(),
        "activate Workflow AgentSession 'agent-session-1': unavailable"
    );
}

#[test]
fn test_workflow_agent_session_activation_owner競合の文言を保持する() {
    // Given / When
    let error = activation_error(
        "agent-session-1",
        AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::OwnerConflict),
    );
    // Then
    assert!(
        matches!(error, WorkflowRuntimeError::AgentSession(ref message) if message == "activate Workflow AgentSession 'agent-session-1': kind=owner_conflict")
    );
}
