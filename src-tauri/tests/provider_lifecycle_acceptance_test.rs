#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use agent_tui_fixture::{
    run_fixture, FixtureLifecycleCommand, FixtureLifecycleEmission, FixturePlan, FixtureRun,
    FixtureRunOptions,
};
use releash_lib::provider_lifecycle_acceptance::{
    AcceptanceFact, AcceptanceFactKind, AcceptanceIngressResult, AcceptanceLaunch,
    AcceptanceLedgerEventCounts, AcceptanceProvider, AcceptanceScope, AcceptanceUnavailableReason,
    ProviderLifecycleAcceptanceHost,
};

const CLI_PATH: &str = env!("CARGO_BIN_EXE_releash");
const TRANSCRIPT_BODY_MARKER: &str = "provider-conversation-body-must-not-be-persisted";

#[test]
fn test_providerライフサイクルcharacterization_installed_cli実行gateが存在する() {
    assert!(Path::new("tests/provider_lifecycle_characterization_test.rs").is_file());
}

fn provider_name(provider: AcceptanceProvider) -> &'static str {
    match provider {
        AcceptanceProvider::Claude => "claude",
        AcceptanceProvider::Codex => "codex",
    }
}

fn scope(provider: AcceptanceProvider, scenario: &str, attempt: u32) -> AcceptanceScope {
    AcceptanceScope::new(
        format!("agent-{}-{scenario}", provider_name(provider)),
        format!("workflow-{}-{scenario}", provider_name(provider)),
        format!("node-{}-{scenario}", provider_name(provider)),
        attempt,
    )
}

fn payload(event: &str, session_id: &str, transcript_ref: Option<&str>) -> String {
    serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_ref,
        "cwd": "/acceptance/workspace",
        "hook_event_name": event,
        "last_assistant_message": TRANSCRIPT_BODY_MARKER,
    })
    .to_string()
}

fn stop_failure_payload(session_id: &str, transcript_ref: &str) -> String {
    serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_ref,
        "cwd": "/acceptance/workspace",
        "hook_event_name": "StopFailure",
        "error": "provider stop failed",
        "error_details": "diagnostic only",
        "last_assistant_message": TRANSCRIPT_BODY_MARKER,
    })
    .to_string()
}

fn command(
    provider: AcceptanceProvider,
    launch: &AcceptanceLaunch,
    data_dir: &Path,
) -> FixtureLifecycleCommand {
    let mut environment = launch.environment.clone();
    environment.push((
        "PATH".to_string(),
        data_dir.join("bin").to_string_lossy().into_owned(),
    ));
    environment.push((
        "RELEASH_DATA_DIR".to_string(),
        data_dir.to_string_lossy().into_owned(),
    ));
    let generated_command = generated_hook_command(provider, launch);
    let mut parts = generated_command.split_ascii_whitespace();
    let executable = parts
        .next()
        .expect("generated Hook command must contain an executable")
        .to_string();
    install_cli_alias(data_dir, &executable);
    FixtureLifecycleCommand {
        executable,
        arguments: parts.map(str::to_string).collect(),
        environment,
    }
}

fn install_cli_alias(data_dir: &Path, alias: &str) {
    assert!(
        matches!(alias, "releash" | "releash-dev"),
        "generated Hook executable must be a managed Releash alias: {alias}"
    );
    let bin_directory = data_dir.join("bin");
    std::fs::create_dir_all(&bin_directory).expect("create acceptance alias directory");
    let wrapper = bin_directory.join(alias);
    let script = format!("#!/bin/sh\nexec {} \"$@\"\n", shell_quote(CLI_PATH));
    std::fs::write(&wrapper, script).expect("write acceptance alias wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&wrapper)
            .expect("stat acceptance alias wrapper")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions)
            .expect("make acceptance alias wrapper executable");
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn generated_hook_command(provider: AcceptanceProvider, launch: &AcceptanceLaunch) -> String {
    match provider {
        AcceptanceProvider::Claude => {
            let hooks = launch
                .files
                .iter()
                .find(|file| file.relative_path == Path::new("hooks/hooks.json"))
                .expect("Claude launch must contain hooks/hooks.json");
            let hooks: serde_json::Value =
                serde_json::from_slice(&hooks.contents).expect("Claude Hook JSON must be valid");
            hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .expect("Claude SessionStart Hook command must be a string")
                .to_string()
        }
        AcceptanceProvider::Codex => {
            let config = launch
                .arguments
                .iter()
                .find(|argument| argument.starts_with("hooks.SessionStart="))
                .expect("Codex launch must contain SessionStart Hook configuration");
            let marker = "command=\"";
            let start = config
                .find(marker)
                .map(|index| index + marker.len())
                .expect("Codex SessionStart Hook command must be present");
            let end = config[start..]
                .find('"')
                .map(|index| start + index)
                .expect("Codex SessionStart Hook command must be terminated");
            config[start..end].to_string()
        }
    }
}

fn replace_environment(command: &mut FixtureLifecycleCommand, key: &str, value: &str) {
    let (_, existing) = command
        .environment
        .iter_mut()
        .find(|(candidate, _)| candidate == key)
        .unwrap_or_else(|| panic!("missing launch environment: {key}"));
    *existing = value.to_string();
}

fn replace_provider_argument(command: &mut FixtureLifecycleCommand, provider: &str) {
    let provider_index = command
        .arguments
        .iter()
        .position(|argument| argument == "--provider")
        .expect("generated Hook command must contain --provider");
    command.arguments[provider_index + 1] = provider.to_string();
}

fn run_product_fixture(
    label: &str,
    lifecycle_command: FixtureLifecycleCommand,
    lifecycle: Vec<FixtureLifecycleEmission>,
) -> agent_tui_fixture::FixtureRun {
    let mut plan = FixturePlan::new(label, lifecycle);
    plan.lifecycle_command = Some(lifecycle_command);
    run_fixture(plan, FixtureRunOptions::default())
}

fn raw(payload: String) -> FixtureLifecycleEmission {
    FixtureLifecycleEmission::raw(&payload)
}

fn delayed_raw(payload: String, delay_before_ms: u64) -> FixtureLifecycleEmission {
    let mut emission = raw(payload);
    emission.delay_before_ms = delay_before_ms;
    emission
}

fn has_session(facts: &[AcceptanceFact], session_id: &str, transcript_ref: Option<&str>) -> bool {
    facts.iter().any(|fact| {
        matches!(
            &fact.kind,
            AcceptanceFactKind::SessionAssociated {
                provider_session_id,
                transcript_ref: stored,
                ..
            } if provider_session_id == session_id && stored.as_deref() == transcript_ref
        )
    })
}

fn stop_count(facts: &[AcceptanceFact]) -> usize {
    facts
        .iter()
        .filter(|fact| matches!(fact.kind, AcceptanceFactKind::StopObserved { .. }))
        .count()
}

fn session_count(facts: &[AcceptanceFact]) -> usize {
    facts
        .iter()
        .filter(|fact| matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. }))
        .count()
}

fn unavailable_count(facts: &[AcceptanceFact], reason: &str) -> usize {
    facts
        .iter()
        .filter(|fact| {
            matches!(
                &fact.kind,
                AcceptanceFactKind::LifecycleUnavailable {
                    reason: stored_reason,
                    ..
                } if stored_reason == reason
            )
        })
        .count()
}

fn accepted_lifecycle_count(facts: &[AcceptanceFact]) -> usize {
    facts
        .iter()
        .filter(|fact| {
            matches!(
                fact.kind,
                AcceptanceFactKind::SessionAssociated { .. }
                    | AcceptanceFactKind::TranscriptAssociated { .. }
                    | AcceptanceFactKind::StopObserved { .. }
                    | AcceptanceFactKind::StopFailed { .. }
            )
        })
        .count()
}

async fn assert_rejected_without_ledger_change(
    host: &ProviderLifecycleAcceptanceHost,
    data_dir: &Path,
    original_scope: &AcceptanceScope,
    modified_agent_session_id: Option<&str>,
    before: AcceptanceLedgerEventCounts,
    run: &FixtureRun,
    expected_diagnostic: &str,
) {
    assert_eq!(run.exit_code, 0, "Provider process must remain successful");
    assert!(run
        .terminal_output
        .contains("provider-alive-after-lifecycle"));
    assert_eq!(run.lifecycle_commands.len(), 1);
    let command = &run.lifecycle_commands[0];
    assert_eq!(command.exit_code, 0, "Hook CLI must not terminate Provider");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.stdout).unwrap(),
        serde_json::json!({}),
        "Provider-required Hook stdout must remain valid JSON",
    );
    assert_eq!(host.ledger_event_counts().unwrap(), before);
    assert!(
        command.stderr.contains(expected_diagnostic),
        "missing rejection diagnostic {expected_diagnostic:?}: {}",
        command.stderr,
    );
    let original_facts = host.facts(&original_scope.agent_session_id).await.unwrap();
    assert_eq!(accepted_lifecycle_count(&original_facts), 0);
    assert_eq!(
        host.event_counts(&original_scope.agent_session_id)
            .await
            .unwrap()
            .other,
        0,
        "rejection must not append workflow events",
    );
    if let Some(modified_agent_session_id) = modified_agent_session_id {
        let modified_facts = host.facts(modified_agent_session_id).await.unwrap();
        assert_eq!(accepted_lifecycle_count(&modified_facts), 0);
    }
    assert!(!directory_contains(
        data_dir,
        TRANSCRIPT_BODY_MARKER.as_bytes()
    ));
}

fn directory_contains(directory: &Path, marker: &[u8]) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if directory_contains(&path, marker) {
                return true;
            }
        } else if std::fs::read(path).is_ok_and(|contents| {
            contents
                .windows(marker.len())
                .any(|window| window == marker)
        }) {
            return true;
        }
    }
    false
}

async fn prepare(
    host: &ProviderLifecycleAcceptanceHost,
    provider: AcceptanceProvider,
    scope: AcceptanceScope,
    plugin_directory: &Path,
) -> AcceptanceLaunch {
    let plugin = matches!(provider, AcceptanceProvider::Claude).then_some(plugin_directory);
    let launch = host.prepare_launch(provider, scope, plugin).await.unwrap();
    assert_launch_contract(provider, &launch);
    launch
}

async fn prepare_in_slot(
    host: &ProviderLifecycleAcceptanceHost,
    slot_id: &str,
    provider: AcceptanceProvider,
    scope: AcceptanceScope,
    plugin_directory: &Path,
) -> AcceptanceLaunch {
    let plugin = matches!(provider, AcceptanceProvider::Claude).then_some(plugin_directory);
    let launch = host
        .prepare_launch_in_slot(slot_id, provider, scope, plugin)
        .await
        .unwrap();
    assert_launch_contract(provider, &launch);
    launch
}

fn assert_launch_contract(provider: AcceptanceProvider, launch: &AcceptanceLaunch) {
    match provider {
        AcceptanceProvider::Claude => {
            assert!(!launch.requires_hook_trust);
            assert_eq!(launch.files.len(), 2);
            assert!(launch.arguments.iter().any(|arg| arg == "--plugin-dir"));
        }
        AcceptanceProvider::Codex => {
            assert!(launch.requires_hook_trust);
            assert!(launch.files.is_empty());
            assert!(!launch
                .arguments
                .iter()
                .any(|arg| arg.contains("dangerously-bypass-hook-trust")));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_providerライフサイクル受入_両providerがatui_020とatui_021を満たす() {
    for provider in [AcceptanceProvider::Claude, AcceptanceProvider::Codex] {
        let data_dir = tempfile::TempDir::new().unwrap();
        let plugin_directory = tempfile::TempDir::new().unwrap();
        let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
        let name = provider_name(provider);

        let correct_scope = scope(provider, "correct", 1);
        let correct = prepare(
            &host,
            provider,
            correct_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let ledger_before = host.ledger_event_counts().unwrap();
        let workflow_commands_before = host.workflow_runtime_command_count();
        let provider_session = format!("{name}-session-correct");
        let transcript_ref = format!("provider://{name}/transcript-correct");
        let correct_run = run_product_fixture(
            &format!("{name}-correct-visible-terminal"),
            command(provider, &correct, data_dir.path()),
            vec![
                raw(payload(
                    "SessionStart",
                    &provider_session,
                    Some(&transcript_ref),
                )),
                delayed_raw(
                    payload("Stop", &provider_session, Some(&transcript_ref)),
                    80,
                ),
            ],
        );
        assert_eq!(correct_run.exit_code, 0);
        assert!(correct_run
            .terminal_output
            .contains("provider-alive-after-lifecycle"));
        let facts = host.facts(&correct_scope.agent_session_id).await.unwrap();
        assert!(has_session(
            &facts,
            &provider_session,
            Some(&transcript_ref)
        ));
        assert_eq!(session_count(&facts), 1);
        assert_eq!(stop_count(&facts), 1);
        let session_at = facts
            .iter()
            .find(|fact| matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. }))
            .unwrap()
            .occurred_at_ms;
        let stop_at = facts
            .iter()
            .find(|fact| matches!(fact.kind, AcceptanceFactKind::StopObserved { .. }))
            .unwrap()
            .occurred_at_ms;
        assert!(stop_at.saturating_sub(session_at) >= 50);
        let ledger_after = host.ledger_event_counts().unwrap();
        assert_eq!(
            ledger_after.other, ledger_before.other,
            "#1596 must not append workflow events",
        );
        assert_eq!(
            host.workflow_runtime_command_count(),
            workflow_commands_before,
            "#1596 must not invoke workflow runtime commands",
        );
        assert!(!directory_contains(
            data_dir.path(),
            TRANSCRIPT_BODY_MARKER.as_bytes()
        ));

        let duplicate_scope = scope(provider, "duplicate", 1);
        let duplicate = prepare(
            &host,
            provider,
            duplicate_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let duplicate_session = format!("{name}-session-duplicate");
        let duplicate_transcript = format!("provider://{name}/transcript-duplicate");
        run_product_fixture(
            &format!("{name}-duplicate"),
            command(provider, &duplicate, data_dir.path()),
            vec![
                raw(payload(
                    "SessionStart",
                    &duplicate_session,
                    Some(&duplicate_transcript),
                )),
                raw(payload(
                    "SessionStart",
                    &duplicate_session,
                    Some(&duplicate_transcript),
                )),
                raw(payload(
                    "Stop",
                    &duplicate_session,
                    Some(&duplicate_transcript),
                )),
                raw(payload(
                    "Stop",
                    &duplicate_session,
                    Some(&duplicate_transcript),
                )),
            ],
        );
        let facts = host.facts(&duplicate_scope.agent_session_id).await.unwrap();
        assert_eq!(session_count(&facts), 1);
        assert_eq!(stop_count(&facts), 1);

        let missing_start_scope = scope(provider, "missing-start", 1);
        let missing_start = prepare(
            &host,
            provider,
            missing_start_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        run_product_fixture(
            &format!("{name}-missing-start"),
            command(provider, &missing_start, data_dir.path()),
            vec![raw(payload(
                "Stop",
                &format!("{name}-session-missing-start"),
                None,
            ))],
        );
        assert_eq!(
            host.report_unavailable(
                &missing_start,
                AcceptanceUnavailableReason::SessionStartDeadlineExceeded,
            )
            .await
            .unwrap(),
            AcceptanceIngressResult::Applied,
        );
        assert_eq!(
            host.report_unavailable(
                &missing_start,
                AcceptanceUnavailableReason::SessionStartDeadlineExceeded,
            )
            .await
            .unwrap(),
            AcceptanceIngressResult::Duplicate,
        );
        let facts = host
            .facts(&missing_start_scope.agent_session_id)
            .await
            .unwrap();
        assert_eq!(session_count(&facts), 0);
        assert_eq!(stop_count(&facts), 0);
        assert_eq!(
            unavailable_count(&facts, "session_start_deadline_exceeded"),
            1
        );

        let missing_stop_scope = scope(provider, "missing-stop", 1);
        let missing_stop = prepare(
            &host,
            provider,
            missing_stop_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        run_product_fixture(
            &format!("{name}-missing-stop"),
            command(provider, &missing_stop, data_dir.path()),
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-missing-stop"),
                None,
            ))],
        );
        let facts = host
            .facts(&missing_stop_scope.agent_session_id)
            .await
            .unwrap();
        assert_eq!(session_count(&facts), 1);
        assert_eq!(stop_count(&facts), 0);

        for (scenario, environment_key, wrong_value) in [
            (
                "other-agent",
                "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
                "other-agent-session",
            ),
            (
                "other-workflow",
                "RELEASH_PROVIDER_LIFECYCLE_WORKFLOW_EXECUTION_ID",
                "other-workflow-execution",
            ),
            (
                "other-node",
                "RELEASH_PROVIDER_LIFECYCLE_NODE_EXECUTION_ID",
                "other-node-execution",
            ),
        ] {
            let signal_scope = scope(provider, scenario, 2);
            let launch = prepare(
                &host,
                provider,
                signal_scope.clone(),
                plugin_directory.path(),
            )
            .await;
            let mut lifecycle_command = command(provider, &launch, data_dir.path());
            replace_environment(&mut lifecycle_command, environment_key, wrong_value);
            let before = host.ledger_event_counts().unwrap();
            let run = run_product_fixture(
                &format!("{name}-{scenario}"),
                lifecycle_command,
                vec![raw(payload(
                    "SessionStart",
                    &format!("{name}-session-{scenario}"),
                    None,
                ))],
            );
            assert_rejected_without_ledger_change(
                &host,
                data_dir.path(),
                &signal_scope,
                (scenario == "other-agent").then_some("other-agent-session"),
                before,
                &run,
                "scope_mismatch",
            )
            .await;
        }

        let malformed_scope = scope(provider, "malformed", 1);
        let malformed = prepare(
            &host,
            provider,
            malformed_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-malformed"),
            command(provider, &malformed, data_dir.path()),
            vec![FixtureLifecycleEmission::raw("{not-valid-json")],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &malformed_scope,
            None,
            before,
            &run,
            "Provider lifecycle payload is invalid",
        )
        .await;

        let invalid_scope = scope(provider, "invalid-capability", 1);
        let invalid = prepare(
            &host,
            provider,
            invalid_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let mut invalid_command = command(provider, &invalid, data_dir.path());
        replace_environment(
            &mut invalid_command,
            "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY",
            "invalid-capability",
        );
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-invalid-capability"),
            invalid_command,
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-invalid"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &invalid_scope,
            None,
            before,
            &run,
            "invalid_capability",
        )
        .await;

        let stale_scope = scope(provider, "stale-capability", 1);
        let stale_slot = format!("{name}-stale-capability-slot");
        let stale = prepare_in_slot(
            &host,
            &stale_slot,
            provider,
            stale_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let current = prepare_in_slot(
            &host,
            &stale_slot,
            provider,
            stale_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let mut stale_capability_command = command(provider, &current, data_dir.path());
        replace_environment(
            &mut stale_capability_command,
            "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY",
            &stale.capability,
        );
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-stale-capability"),
            stale_capability_command,
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-stale"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &stale_scope,
            None,
            before,
            &run,
            "invalid_capability",
        )
        .await;

        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-expired-binding"),
            command(provider, &stale, data_dir.path()),
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-expired"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &stale_scope,
            None,
            before,
            &run,
            "binding_expired",
        )
        .await;
        let facts = host.facts(&stale_scope.agent_session_id).await.unwrap();
        assert_eq!(accepted_lifecycle_count(&facts), 0);
        assert!(facts.iter().any(|fact| {
            matches!(
                &fact.kind,
                AcceptanceFactKind::BindingExpired { binding_id }
                    if binding_id == &stale.binding_id
            )
        }));

        let retry_slot = format!("{name}-previous-attempt-slot");
        let previous_scope = AcceptanceScope::new(
            format!("agent-{name}-previous-attempt"),
            format!("workflow-{name}-retry"),
            format!("node-{name}-previous-attempt"),
            1,
        );
        let previous = prepare_in_slot(
            &host,
            &retry_slot,
            provider,
            previous_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let current_scope = AcceptanceScope::new(
            format!("agent-{name}-current-attempt"),
            format!("workflow-{name}-retry"),
            format!("node-{name}-current-attempt"),
            2,
        );
        let _current = prepare_in_slot(
            &host,
            &retry_slot,
            provider,
            current_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-previous-attempt"),
            command(provider, &previous, data_dir.path()),
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-previous-attempt"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &previous_scope,
            None,
            before,
            &run,
            "binding_expired",
        )
        .await;
        let previous_facts = host.facts(&previous_scope.agent_session_id).await.unwrap();
        assert!(previous_facts.iter().any(|fact| {
            matches!(
                &fact.kind,
                AcceptanceFactKind::BindingExpired { binding_id }
                    if binding_id == &previous.binding_id
            )
        }));
        let current_facts = host.facts(&current_scope.agent_session_id).await.unwrap();
        assert_eq!(accepted_lifecycle_count(&current_facts), 0);

        let provider_mismatch_scope = scope(provider, "provider-mismatch", 1);
        let provider_mismatch = prepare(
            &host,
            provider,
            provider_mismatch_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let mut provider_mismatch_command = command(provider, &provider_mismatch, data_dir.path());
        replace_provider_argument(
            &mut provider_mismatch_command,
            match provider {
                AcceptanceProvider::Claude => "codex",
                AcceptanceProvider::Codex => "claude",
            },
        );
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-provider-mismatch"),
            provider_mismatch_command,
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-provider-mismatch"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &provider_mismatch_scope,
            None,
            before,
            &run,
            "provider_mismatch",
        )
        .await;

        let missing_discovery_scope = scope(provider, "missing-discovery", 1);
        let missing_discovery = prepare(
            &host,
            provider,
            missing_discovery_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let absent_data_dir = tempfile::TempDir::new().unwrap();
        let before = host.ledger_event_counts().unwrap();
        let run = run_product_fixture(
            &format!("{name}-missing-discovery"),
            command(provider, &missing_discovery, absent_data_dir.path()),
            vec![raw(payload(
                "SessionStart",
                &format!("{name}-session-no-discovery"),
                None,
            ))],
        );
        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &missing_discovery_scope,
            None,
            before,
            &run,
            "Releash アプリの起動が必要",
        )
        .await;

        for (scenario, visible_label, exit_code) in [
            ("process-exit", "ordinary provider output", 23),
            (
                "visible-stop-text",
                "hook_event_name=Stop validated Stop completed",
                0,
            ),
        ] {
            let no_signal_scope = scope(provider, scenario, 1);
            let launch = prepare(
                &host,
                provider,
                no_signal_scope.clone(),
                plugin_directory.path(),
            )
            .await;
            let mut plan = FixturePlan::new(visible_label, vec![]);
            plan.exit_code = exit_code;
            plan.lifecycle_command = Some(command(provider, &launch, data_dir.path()));
            let run = run_fixture(plan, FixtureRunOptions::default());
            assert_eq!(run.exit_code, u32::from(exit_code));
            assert!(run.terminal_output.contains(visible_label));
            let facts = host.facts(&no_signal_scope.agent_session_id).await.unwrap();
            assert_eq!(session_count(&facts), 0);
            assert_eq!(stop_count(&facts), 0);
        }

        if matches!(provider, AcceptanceProvider::Claude) {
            let failure_scope = scope(provider, "stop-failure", 1);
            let failure = prepare(
                &host,
                provider,
                failure_scope.clone(),
                plugin_directory.path(),
            )
            .await;
            let failure_session = "claude-session-stop-failure";
            let failure_transcript = "provider://claude/stop-failure";
            run_product_fixture(
                "claude-stop-failure",
                command(provider, &failure, data_dir.path()),
                vec![
                    raw(payload(
                        "SessionStart",
                        failure_session,
                        Some(failure_transcript),
                    )),
                    raw(stop_failure_payload(failure_session, failure_transcript)),
                ],
            );
            let facts = host.facts(&failure_scope.agent_session_id).await.unwrap();
            assert_eq!(stop_count(&facts), 0);
            assert!(facts
                .iter()
                .any(|fact| matches!(fact.kind, AcceptanceFactKind::StopFailed { .. })));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_providerライフサイクル受入_stale_discoveryから古い接続先へ送信しない() {
    for provider in [AcceptanceProvider::Claude, AcceptanceProvider::Codex] {
        let data_dir = tempfile::TempDir::new().unwrap();
        let plugin_directory = tempfile::TempDir::new().unwrap();
        let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
        let provider_scope = scope(provider, "stale-discovery", 1);
        let launch = prepare(
            &host,
            provider,
            provider_scope.clone(),
            plugin_directory.path(),
        )
        .await;
        let stale_target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stale_port = stale_target.local_addr().unwrap().port();
        let stale_request = std::thread::spawn(move || {
            let (mut stream, _) = stale_target.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 8_192];
            if let Ok(read) = stream.read(&mut buffer) {
                request.extend_from_slice(&buffer[..read]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        let discovery_path = data_dir.path().join("local-api.json");
        let mut stale_discovery: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&discovery_path).unwrap()).unwrap();
        stale_discovery["port"] = serde_json::json!(stale_port);
        stale_discovery["token"] = serde_json::json!("stale-bearer-token");
        std::fs::write(
            &discovery_path,
            serde_json::to_vec(&stale_discovery).unwrap(),
        )
        .unwrap();
        let before = host.ledger_event_counts().unwrap();

        let run = run_product_fixture(
            &format!("{}-stale-discovery", provider_name(provider)),
            command(provider, &launch, data_dir.path()),
            vec![raw(payload(
                "SessionStart",
                &format!("{}-stale-session", provider_name(provider)),
                Some("provider://stale/transcript"),
            ))],
        );

        assert_rejected_without_ledger_change(
            &host,
            data_dir.path(),
            &provider_scope,
            None,
            before,
            &run,
            "不正または古い",
        )
        .await;
        let stale_request = stale_request.join().unwrap();
        assert!(!stale_request
            .to_ascii_lowercase()
            .contains("authorization:"));
        assert!(!stale_request.contains(&launch.capability));
    }
}

#[test]
fn test_providerライフサイクル受信_product_acceptance専用routerを持たない() {
    let router_source = include_str!("../src/adaptor/controller/api/mod.rs");

    assert!(!router_source.contains("build_provider_lifecycle_router"));
    assert!(router_source
        .contains("application_router.merge(provider_lifecycle::router(provider_lifecycle))"));
}
