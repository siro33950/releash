use super::*;

#[tokio::test(flavor = "current_thread")]
async fn test_provider_availability_controller_blocking操作中もasync_runtimeを占有しない() {
    let heartbeat = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let heartbeat_task = {
        let heartbeat = heartbeat.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            heartbeat.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };

    run_provider_availability_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(())
    })
    .await
    .unwrap();
    heartbeat_task.await.unwrap();

    assert!(heartbeat.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn test_agent_session_controller_domain結果をwire語彙へ変換する() {
    assert_eq!(
        AgentSessionOpenResponse::from(AgentSessionOpenOutcome::Indeterminate),
        AgentSessionOpenResponse::Indeterminate
    );
    assert_eq!(
        AgentSessionArchiveResponse::from(AgentSessionArchiveOutcome::DeleteConfirmationRequired),
        AgentSessionArchiveResponse::DeleteConfirmationRequired
    );
}

#[test]
fn test_agent_session_controller_provider未選択と未知値を起動前に拒否する() {
    assert_eq!(
        parse_provider("claude", ProviderParseOperation::Start).unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Claude
    );
    assert_eq!(
        parse_provider("codex", ProviderParseOperation::Start).unwrap(),
        crate::domain::provider_lifecycle::ProviderKind::Codex
    );
    let missing =
        serde_json::to_value(parse_provider("", ProviderParseOperation::Start).unwrap_err())
            .unwrap();
    let unknown =
        serde_json::to_value(parse_provider("unknown", ProviderParseOperation::Start).unwrap_err())
            .unwrap();
    assert_eq!(missing["code"], "AGENT_SESSION_INVALID_PROVIDER");
    assert_eq!(unknown["code"], "AGENT_SESSION_INVALID_PROVIDER");
}

#[test]
fn test_agent_session_controller_terminal_spawn詳細を既存の利用者向けerrorへ変換する() {
    // Given
    let internal_worktree_path = "/repo/worktree";

    // When
    let error = launch_error(
        AgentSessionLaunchUsecaseError::TerminalSpawn(
            crate::domain::agent_session::ProviderAgentTerminalSpawnError::PerWorktreeCap {
                worktree_path: internal_worktree_path.to_string(),
            },
        ),
        AgentSessionLaunchOperation::Start,
    );

    // Then
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["code"], "AGENT_SESSION_TERMINAL_UNAVAILABLE");
    assert_eq!(
        value["message"],
        "Releash could not complete the Terminal operation for this AgentSession. Try again."
    );
    assert!(!value.to_string().contains(internal_worktree_path));
}

#[test]
fn test_agent_session_controller_対象21codeを利用者向け英語文言へ変換する() {
    // Given
    let cases = [
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::InvalidInput),
            "PROVIDER_AVAILABILITY_INVALID_EXECUTABLE",
            "Enter a Provider executable command name or path.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::ConfigUnavailable),
            "PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE",
            "Releash could not access the Provider executable setting. Try again.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::RefreshUnavailable),
            "PROVIDER_AVAILABILITY_REFRESH_UNAVAILABLE",
            "Releash could not refresh Provider CLI availability. Try again.",
        ),
        (
            provider_availability_error(ProviderAvailabilityUsecaseError::Corrupt),
            "PROVIDER_AVAILABILITY_CORRUPT",
            "Releash could not read Provider CLI availability. Restart Releash and try again.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::Start).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before starting the AgentSession.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::ProviderUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_PROVIDER_UNAVAILABLE",
            "The selected Provider is unavailable. Check its executable and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not start the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::StorageUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Releash could not access saved AgentSession data. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::LaunchUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Releash could not complete the Provider operation for this AgentSession. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::TerminalUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Releash could not complete the Terminal operation for this AgentSession. Try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Corrupt,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CORRUPT",
            "Releash could not continue because the AgentSession data is invalid.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::NotFound),
            "AGENT_SESSION_NOT_FOUND",
            "The AgentSession is no longer available.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::InvalidOperation),
            "AGENT_SESSION_INVALID_OPERATION",
            "This operation is not available for the AgentSession in its current state. Refresh and try again.",
        ),
        (
            read_error(AgentSessionReadUsecaseError::InvalidRequest),
            "AGENT_SESSION_INVALID_REQUEST",
            "Releash could not load the AgentSession because the request is invalid.",
        ),
        (
            history_error(AgentSessionHistoryQueryError::InvalidRequest),
            "AGENT_SESSION_HISTORY_INVALID_REQUEST",
            "Releash could not load AgentSession history because the request is invalid.",
        ),
        (
            history_error(AgentSessionHistoryQueryError::Unavailable),
            "AGENT_SESSION_HISTORY_UNAVAILABLE",
            "Releash could not load AgentSession history. Try again.",
        ),
        (
            history_error(AgentSessionHistoryQueryError::Corrupt),
            "AGENT_SESSION_HISTORY_CORRUPT",
            "Releash could not load AgentSession history because its saved data is invalid.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::InvalidInput),
            "PROVIDER_HOOK_HEALTH_INVALID_REQUEST",
            "Releash could not load Provider Hook health because the request is invalid.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::StorageUnavailable),
            "PROVIDER_HOOK_HEALTH_STORAGE_UNAVAILABLE",
            "Releash could not load Provider Hook health. Try again.",
        ),
        (
            hook_health_error(ProviderHookHealthUsecaseError::Corrupt),
            "PROVIDER_HOOK_HEALTH_CORRUPT",
            "Releash could not load Provider Hook health because its saved data is invalid.",
        ),
    ];

    // When / Then
    for (error, expected_code, expected_message) in cases {
        assert_coded_error(error, expected_code, expected_message);
    }
}

#[test]
fn test_agent_session_controller_操作依存codeを操作ごとの固定文言へ変換する() {
    // Given
    let cases = [
        (
            parse_provider("unknown", ProviderParseOperation::ConfigureProvider)
                .unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a valid Provider.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::Start).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before starting the AgentSession.",
        ),
        (
            parse_provider("unknown", ProviderParseOperation::ResumeHistory).unwrap_err(),
            "AGENT_SESSION_INVALID_PROVIDER",
            "Select a Provider before resuming the AgentSession.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not start the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::InvalidInput,
                AgentSessionLaunchOperation::ResumeHistory,
            ),
            "AGENT_SESSION_INVALID_INPUT",
            "Releash could not resume the AgentSession because the request is invalid.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict,
                AgentSessionLaunchOperation::Start,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.",
        ),
        (
            launch_error(
                AgentSessionLaunchUsecaseError::Conflict,
                AgentSessionLaunchOperation::ResumeHistory,
            ),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be resumed because it changed or its Provider session is already in use. Refresh and try again.",
        ),
        (
            lifecycle_error(AgentSessionLifecycleUsecaseError::Conflict),
            "AGENT_SESSION_CONFLICT",
            "The AgentSession could not be updated because it changed or its Provider session is already in use. Refresh and try again.",
        ),
    ];

    // When / Then
    for (error, expected_code, expected_message) in cases {
        assert_coded_error(error, expected_code, expected_message);
    }
}

#[test]
fn test_agent_session_controller_共有codeは全usecase_error経路で同じ文言になる() {
    // Given / When / Then
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::StorageUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::StorageUnavailable),
            read_error(AgentSessionReadUsecaseError::StorageUnavailable),
        ],
        "AGENT_SESSION_STORAGE_UNAVAILABLE",
        "Releash could not access saved AgentSession data. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::LaunchUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::LaunchUnavailable),
        ],
        "AGENT_SESSION_LAUNCH_UNAVAILABLE",
        "Releash could not complete the Provider operation for this AgentSession. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::TerminalUnavailable,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::TerminalUnavailable),
            read_error(AgentSessionReadUsecaseError::TerminalUnavailable),
        ],
        "AGENT_SESSION_TERMINAL_UNAVAILABLE",
        "Releash could not complete the Terminal operation for this AgentSession. Try again.",
    );
    assert_coded_errors(
        [
            launch_error(
                AgentSessionLaunchUsecaseError::Corrupt,
                AgentSessionLaunchOperation::Start,
            ),
            lifecycle_error(AgentSessionLifecycleUsecaseError::Corrupt),
            read_error(AgentSessionReadUsecaseError::Corrupt),
        ],
        "AGENT_SESSION_CORRUPT",
        "Releash could not continue because the AgentSession data is invalid.",
    );
}

fn assert_coded_errors<const N: usize>(
    errors: [AppError; N],
    expected_code: &str,
    expected_message: &str,
) {
    for error in errors {
        assert_coded_error(error, expected_code, expected_message);
    }
}

fn assert_coded_error(error: AppError, expected_code: &str, expected_message: &str) {
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({
            "code": expected_code,
            "message": expected_message,
        })
    );
}
