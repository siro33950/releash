use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tempfile::{tempdir, TempDir};

use super::launch_spec::ProviderLaunchSpecError;
use super::{
    parse_provider_payload, LocalProviderLifecycleCredentialGateway,
    LocalProviderLifecycleEventRepository, ProviderLaunchContext, ProviderLaunchSpec,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::{
    ProviderLaunchOptions, ProviderSessionLaunch, ProviderSessionLaunchError,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation, StreamId,
    UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSignalKind,
    ProviderLifecycleSlotId,
};
use crate::domain::workflow::{AgentSessionActivity, SessionPermission};
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

fn scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-1").unwrap()
}

fn context() -> ProviderLaunchContext {
    ProviderLaunchContext::new(slot_id(), "binding-1", "capability-1", scope()).unwrap()
}

fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("slot-1").unwrap()
}

#[test]
fn test_provider起動設定_development_profileでは両providerにreleash_devを使う() {
    let plugin_directory = tempdir().unwrap();
    let claude = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash-dev",
        Some(plugin_directory.path()),
    )
    .unwrap();
    let codex =
        ProviderLaunchSpec::for_provider(ProviderKind::Codex, context(), "releash-dev", None)
            .unwrap();

    let claude_hooks = claude
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new("hooks/hooks.json"))
        .unwrap();
    assert!(std::str::from_utf8(claude_hooks.contents())
        .unwrap()
        .contains("releash-dev hook receive --provider claude"));
    assert!(codex
        .arguments()
        .iter()
        .any(|argument| { argument.contains("releash-dev hook receive --provider codex") }));
}

#[test]
fn test_provider信号変換_claude_payloadを正確なdomain_signalへ変換する() {
    let session_start = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{
            "session_id":"claude-session-1",
            "transcript_path":"/provider/claude/transcript.jsonl",
            "cwd":"/workspace",
            "hook_event_name":"SessionStart",
            "source":"startup"
        }"#,
    )
    .unwrap();
    assert_eq!(session_start.binding_id(), "binding-1");
    assert_eq!(session_start.provider(), ProviderKind::Claude);
    assert_eq!(session_start.scope(), &scope());
    assert_eq!(
        session_start.into_kind(),
        ProviderLifecycleSignalKind::SessionStarted {
            provider_session_id: "claude-session-1".to_string(),
            transcript_ref: Some("/provider/claude/transcript.jsonl".to_string()),
        }
    );

    let stop = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{
            "session_id":"claude-session-1",
            "transcript_path":"/provider/claude/transcript.jsonl",
            "cwd":"/workspace",
            "hook_event_name":"Stop",
            "stop_hook_active":false,
            "last_assistant_message":"done"
        }"#,
    )
    .unwrap();
    assert_eq!(
        stop.into_kind(),
        ProviderLifecycleSignalKind::StopObserved {
            provider_session_id: "claude-session-1".to_string(),
            transcript_ref: Some("/provider/claude/transcript.jsonl".to_string()),
        }
    );
}

#[test]
fn test_provider信号変換_claude_subagent_payloadをroot_signalとして変換しない() {
    let error = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{
            "session_id":"claude-session-1",
            "transcript_path":"/provider/claude/transcript.jsonl",
            "cwd":"/workspace",
            "hook_event_name":"Stop",
            "agent_id":"agent-child-1",
            "agent_type":"Explore"
        }"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Provider lifecycle payload belongs to a subagent"
    );
}

#[test]
fn test_provider信号変換_両providerの共通eventを同じ活動状態へ変換する() {
    let cases = [
        ("UserPromptSubmit", None, AgentSessionActivity::Working),
        ("PreToolUse", Some("Bash"), AgentSessionActivity::Working),
        ("PostToolUse", Some("Bash"), AgentSessionActivity::Working),
        (
            "PermissionRequest",
            Some("Bash"),
            AgentSessionActivity::AwaitingAnswer,
        ),
    ];

    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        for (event, tool_name, expected) in cases {
            let payload = serde_json::json!({
                "session_id": "provider-session-1",
                "transcript_path": "/provider/transcript.jsonl",
                "hook_event_name": event,
                "tool_name": tool_name,
                "prompt": "secret prompt",
                "tool_input": {"secret": true},
                "tool_response": "secret output"
            });

            let signal = parse_provider_payload(
                provider,
                "binding-1",
                scope(),
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();

            assert_eq!(
                signal.into_kind(),
                ProviderLifecycleSignalKind::ActivityObserved {
                    provider_session_id: "provider-session-1".to_string(),
                    transcript_ref: Some("/provider/transcript.jsonl".to_string()),
                    activity: expected,
                }
            );
        }
    }
}

#[test]
fn test_provider信号変換_質問系pre_tool_useを正規化して回答待ちへ変換する() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        for tool_name in [
            "AskUserQuestion",
            "ask-user_question",
            "request_user_input",
            "REQUEST.USER-INPUT",
        ] {
            let payload = serde_json::json!({
                "session_id": "provider-session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": tool_name
            });

            let signal = parse_provider_payload(
                provider,
                "binding-1",
                scope(),
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();

            assert!(matches!(
                signal.into_kind(),
                ProviderLifecycleSignalKind::ActivityObserved {
                    activity: AgentSessionActivity::AwaitingAnswer,
                    ..
                }
            ));
        }
    }
}

#[test]
fn test_provider信号変換_tool名なしのpre_tool_useをworkingとして扱う() {
    let signal = parse_provider_payload(
        ProviderKind::Codex,
        "binding-1",
        scope(),
        br#"{"session_id":"provider-session-1","hook_event_name":"PreToolUse"}"#,
    )
    .unwrap();

    assert!(matches!(
        signal.into_kind(),
        ProviderLifecycleSignalKind::ActivityObserved {
            activity: AgentSessionActivity::Working,
            ..
        }
    ));
}

#[test]
fn test_provider信号変換_claudeの追加eventでもsubagentを除外する() {
    let error = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{
            "session_id":"claude-session-1",
            "hook_event_name":"PreToolUse",
            "tool_name":"Bash",
            "agent_id":"agent-child-1"
        }"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Provider lifecycle payload belongs to a subagent"
    );
}

#[test]
fn test_provider信号変換_claude_stop_failureを診断にしてstopへ変換しない() {
    let signal = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{
            "session_id":"claude-session-1",
            "transcript_path":null,
            "cwd":"/workspace",
            "hook_event_name":"StopFailure",
            "error":"rate_limit",
            "error_details":"429 Too Many Requests",
            "last_assistant_message":"API Error: Rate limit reached"
        }"#,
    )
    .unwrap();

    assert_eq!(
        signal.into_kind(),
        ProviderLifecycleSignalKind::StopFailed {
            provider_session_id: "claude-session-1".to_string(),
            transcript_ref: None,
            reason: "rate_limit: 429 Too Many Requests".to_string(),
        }
    );
}

#[test]
fn test_provider信号変換_codexのnullable_transcriptを変換し未知eventを拒否する() {
    let session_start = parse_provider_payload(
        ProviderKind::Codex,
        "binding-1",
        scope(),
        br#"{
            "session_id":"codex-session-1",
            "transcript_path":null,
            "cwd":"/workspace",
            "hook_event_name":"SessionStart",
            "model":"gpt-5",
            "source":"startup"
        }"#,
    )
    .unwrap();
    assert_eq!(
        session_start.into_kind(),
        ProviderLifecycleSignalKind::SessionStarted {
            provider_session_id: "codex-session-1".to_string(),
            transcript_ref: None,
        }
    );

    let stop = parse_provider_payload(
        ProviderKind::Codex,
        "binding-1",
        scope(),
        br#"{
            "session_id":"codex-session-1",
            "transcript_path":"/provider/codex/rollout.jsonl",
            "cwd":"/workspace",
            "hook_event_name":"Stop",
            "model":"gpt-5",
            "turn_id":"turn-1"
        }"#,
    )
    .unwrap();
    assert_eq!(
        stop.into_kind(),
        ProviderLifecycleSignalKind::StopObserved {
            provider_session_id: "codex-session-1".to_string(),
            transcript_ref: Some("/provider/codex/rollout.jsonl".to_string()),
        }
    );

    let unknown = parse_provider_payload(
        ProviderKind::Codex,
        "binding-1",
        scope(),
        br#"{"session_id":"codex-session-1","hook_event_name":"StopFailure"}"#,
    )
    .unwrap_err();
    assert_eq!(
        unknown.to_string(),
        "unsupported Provider lifecycle event: StopFailure"
    );
}

#[test]
fn test_provider信号変換_不正または不完全なpayloadをraw_input非表示で拒否する() {
    let malformed = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{"secret":"must-not-appear""#,
    )
    .unwrap_err();
    assert_eq!(
        malformed.to_string(),
        "Provider lifecycle payload is invalid"
    );
    assert!(!malformed.to_string().contains("must-not-appear"));

    let incomplete = parse_provider_payload(
        ProviderKind::Claude,
        "binding-1",
        scope(),
        br#"{"hook_event_name":"SessionStart"}"#,
    )
    .unwrap_err();
    assert_eq!(
        incomplete.to_string(),
        "Provider lifecycle payload is invalid"
    );
}

#[test]
fn test_provider起動設定_claudeはsession_pluginを使いuser_settingsを変更しない() {
    let directory = tempdir().unwrap();
    let settings_path = directory.path().join("settings.json");
    let original = br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user-hook"}]}]}}"#;
    fs::write(&settings_path, original).unwrap();
    let plugin_directory = directory.path().join("launch-plugin");

    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(&plugin_directory),
    )
    .unwrap();

    assert_eq!(
        spec.arguments(),
        &[
            "--plugin-dir".to_string(),
            plugin_directory.to_string_lossy().into_owned(),
        ]
    );
    assert!(!spec.arguments().iter().any(|value| value == "--settings"));
    assert_eq!(fs::read(&settings_path).unwrap(), original);
    assert!(!spec.requires_hook_trust());

    let manifest = spec
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new(".claude-plugin/plugin.json"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(manifest.contents()).unwrap(),
        serde_json::json!({
            "name":"releash-provider-lifecycle",
            "version":"1.0.0",
            "description":"Releash Provider lifecycle integration",
            "author":{"name":"Releash"}
        })
    );
    let hooks = spec
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new("hooks/hooks.json"))
        .unwrap();
    let hooks = serde_json::from_slice::<serde_json::Value>(hooks.contents()).unwrap();
    for event in [
        "SessionStart",
        "Stop",
        "StopFailure",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
    ] {
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"],
            "releash hook receive --provider claude"
        );
    }
}

#[test]
fn test_provider起動設定_codexはprocess_configを使いhook_trustを要求する() {
    let spec =
        ProviderLaunchSpec::for_provider(ProviderKind::Codex, context(), "releash", None).unwrap();

    assert_eq!(
        spec.arguments(),
        &[
            "-c".to_string(),
            "hooks.SessionStart=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.Stop=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.UserPromptSubmit=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PreToolUse=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PostToolUse=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PermissionRequest=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
        ]
    );
    assert!(spec.requires_hook_trust());
    assert!(spec.files().is_empty());
    assert!(!spec
        .arguments()
        .iter()
        .any(|argument| argument == "--dangerously-bypass-hook-trust"));
}

#[test]
fn test_provider起動設定_claudeのnewとresumeをstructured_root_processへ変換する() {
    let directory = tempdir().unwrap();
    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(directory.path()),
    )
    .unwrap();

    let new = spec
        .terminal_process("/opt/bin/claude", ProviderSessionLaunch::New)
        .unwrap();
    assert_eq!(new.executable(), "/opt/bin/claude");
    assert_eq!(
        new.arguments(),
        &[
            "--plugin-dir".to_string(),
            directory.path().to_string_lossy().into_owned(),
        ]
    );
    assert_eq!(new.environment(), spec.environment());

    let resumed = spec
        .terminal_process(
            "/opt/bin/claude",
            ProviderSessionLaunch::resume("claude-session-1").unwrap(),
        )
        .unwrap();
    assert_eq!(
        resumed.arguments(),
        &[
            "--plugin-dir".to_string(),
            directory.path().to_string_lossy().into_owned(),
            "--resume".to_string(),
            "claude-session-1".to_string(),
        ]
    );
}

#[test]
fn test_provider起動設定_claudeの初回指示を起動時promptとして渡す() {
    let directory = tempdir().unwrap();
    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(directory.path()),
    )
    .unwrap();

    let process = spec
        .terminal_process(
            "/opt/bin/claude",
            ProviderSessionLaunch::new_with_initial_instruction("Implement the workflow node.")
                .unwrap(),
        )
        .unwrap();

    assert_eq!(
        process.arguments().last().map(String::as_str),
        Some("Implement the workflow node.")
    );
}

#[test]
fn test_provider起動設定_codexのnewとresumeをstructured_root_processへ変換する() {
    let spec =
        ProviderLaunchSpec::for_provider(ProviderKind::Codex, context(), "releash", None).unwrap();

    let new = spec
        .terminal_process("/opt/bin/codex", ProviderSessionLaunch::New)
        .unwrap();
    assert_eq!(new.executable(), "/opt/bin/codex");
    assert_eq!(new.arguments(), spec.arguments());
    assert_eq!(new.environment(), spec.environment());

    let resumed = spec
        .terminal_process(
            "/opt/bin/codex",
            ProviderSessionLaunch::resume("codex-session-1").unwrap(),
        )
        .unwrap();
    assert_eq!(
        resumed.arguments(),
        &[
            "-c".to_string(),
            "hooks.SessionStart=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.Stop=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.UserPromptSubmit=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PreToolUse=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PostToolUse=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "-c".to_string(),
            "hooks.PermissionRequest=[{hooks=[{type=\"command\",command=\"releash hook receive --provider codex\"}]}]".to_string(),
            "resume".to_string(),
            "codex-session-1".to_string(),
        ]
    );
}

#[test]
fn test_provider起動設定_codexの初回指示を起動時promptとして渡す() {
    let spec =
        ProviderLaunchSpec::for_provider(ProviderKind::Codex, context(), "releash", None).unwrap();

    let process = spec
        .terminal_process(
            "/opt/bin/codex",
            ProviderSessionLaunch::new_with_initial_instruction("Implement the workflow node.")
                .unwrap(),
        )
        .unwrap();

    assert_eq!(
        process.arguments().last().map(String::as_str),
        Some("Implement the workflow node.")
    );
}

#[test]
fn test_provider起動設定_permissionの4値をprovider別引数列へ写像する() {
    let cases: [(ProviderKind, SessionPermission, &[&str]); 8] = [
        (
            ProviderKind::Claude,
            SessionPermission::Manual,
            &["--permission-mode", "default"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::Auto,
            &["--permission-mode", "auto"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::Bypass,
            &["--permission-mode", "bypassPermissions"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::ReadOnly,
            &["--permission-mode", "plan"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Manual,
            &[
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
            ],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Auto,
            &["--approve-for-me"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Bypass,
            &["--dangerously-bypass-approvals-and-sandbox"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::ReadOnly,
            &["--sandbox", "read-only", "--ask-for-approval", "never"],
        ),
    ];

    for (provider, permission, expected_permission_arguments) in cases {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        let launch = ProviderSessionLaunch::new_with_initial_instruction("complete-action")
            .unwrap()
            .with_options(ProviderLaunchOptions::new(
                Some("model-x".to_string()),
                Some(permission),
            ));

        let process = spec.terminal_process("provider", launch).unwrap();
        let expected_suffix = ["--model", "model-x"]
            .into_iter()
            .chain(expected_permission_arguments.iter().copied())
            .chain(["complete-action"])
            .map(str::to_string)
            .collect::<Vec<_>>();

        assert!(
            process.arguments().ends_with(&expected_suffix),
            "{provider:?} {permission}: {:?}",
            process.arguments()
        );
    }
}

#[test]
fn test_provider起動設定_permission省略時は権限引数なしでmodelを無変換に保つ() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        let launch = ProviderSessionLaunch::new_with_initial_instruction("complete-action")
            .unwrap()
            .with_options(ProviderLaunchOptions::new(
                Some("provider-model-value".to_string()),
                None,
            ));

        let process = spec.terminal_process("provider", launch).unwrap();

        assert!(process.arguments().ends_with(&[
            "--model".to_string(),
            "provider-model-value".to_string(),
            "complete-action".to_string(),
        ]));
        assert!(process.arguments().iter().all(|argument| !matches!(
            argument.as_str(),
            "--permission-mode"
                | "--sandbox"
                | "--ask-for-approval"
                | "--approve-for-me"
                | "--dangerously-bypass-approvals-and-sandbox"
        )));
    }
}

#[test]
fn test_provider起動設定_resumeの空session_idを拒否する() {
    assert_eq!(
        ProviderSessionLaunch::resume(" ").unwrap_err(),
        ProviderSessionLaunchError::ProviderSessionIdMissing
    );
}

#[test]
fn test_provider起動設定_空の初回指示を拒否する() {
    assert_eq!(
        ProviderSessionLaunch::new_with_initial_instruction(" ").unwrap_err(),
        ProviderSessionLaunchError::InitialInstructionMissing
    );
}

#[test]
fn test_provider起動設定_launch_contextをchild_environmentだけに保持する() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        assert_eq!(
            spec.environment(),
            &[
                (
                    "RELEASH_PROVIDER_LIFECYCLE_SLOT_ID".to_string(),
                    "slot-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID".to_string(),
                    "binding-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY".to_string(),
                    "capability-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID".to_string(),
                    "agent-1".to_string(),
                ),
            ]
        );

        let visible_configuration = spec
            .arguments()
            .iter()
            .map(String::as_str)
            .chain(
                spec.files()
                    .iter()
                    .map(|file| std::str::from_utf8(file.contents()).unwrap()),
            )
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!visible_configuration.contains("binding-1"));
        assert!(!visible_configuration.contains("slot-1"));
        assert!(!visible_configuration.contains("capability-1"));
        assert!(!visible_configuration.contains("agent-1"));
        assert!(!visible_configuration.contains("workflow-1"));
        assert!(!visible_configuration.contains("node-1"));
    }
}

#[test]
fn test_provider起動設定_binding入力の欠落を拒否する() {
    assert_eq!(
        ProviderLaunchContext::new(slot_id(), "", "capability-1", scope()).unwrap_err(),
        ProviderLaunchSpecError::EmptyField("binding_id")
    );
    assert_eq!(
        ProviderLaunchContext::new(slot_id(), "binding-1", "", scope()).unwrap_err(),
        ProviderLaunchSpecError::EmptyField("capability")
    );
    assert_eq!(
        ProviderLaunchSpec::for_provider(ProviderKind::Claude, context(), "releash", None)
            .unwrap_err(),
        ProviderLaunchSpecError::ClaudePluginDirectoryRequired
    );
    assert_eq!(
        ProviderLaunchSpec::for_provider(
            ProviderKind::Codex,
            context(),
            "user-controlled-command",
            None,
        )
        .unwrap_err(),
        ProviderLaunchSpecError::UnsupportedCliAlias
    );
}

fn persistence_scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-1").unwrap()
}

fn setup_persistence_usecase() -> (TempDir, Arc<LocalEventStore>, ProviderLifecycleUsecase) {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let events = LocalProviderLifecycleEventRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    (directory, store, usecase)
}

async fn provider_event_count(store: &LocalEventStore, agent_session_id: &str) -> usize {
    store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::provider_lifecycle(agent_session_id).unwrap(),
            after: None,
            limit: 64,
        })
        .await
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                &event.event,
                crate::domain::local_event::LoadedDomainEvent::Known(inner)
                    if matches!(inner.as_ref(), LocalDomainEvent::ProviderLifecycle(_))
            )
        })
        .count()
}

struct ResolveFailureOnceRepository {
    inner: Arc<LocalEventStore>,
    fail_resolve_once: AtomicBool,
}

struct ResolveFailureRepository {
    inner: Arc<LocalEventStore>,
    fail_resolve: AtomicBool,
}

impl ResolveFailureRepository {
    fn new(inner: Arc<LocalEventStore>) -> Self {
        Self {
            inner,
            fail_resolve: AtomicBool::new(false),
        }
    }

    fn set_resolve_failure(&self, fail: bool) {
        self.fail_resolve.store(fail, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for ResolveFailureRepository {
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_mutation_identity_v1(mutation)
    }

    fn canonical_event_batch_identity_v1(
        &self,
        events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_event_batch_identity_v1(events)
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        if self.fail_resolve.load(Ordering::SeqCst) {
            return Err(LocalEventQueryError::Internal {
                correlation_id: "provider-lifecycle-persistent-resolve-failure".to_string(),
            });
        }
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.inner.query(request).await
    }
}

impl ResolveFailureOnceRepository {
    fn new(inner: Arc<LocalEventStore>) -> Self {
        Self {
            inner,
            fail_resolve_once: AtomicBool::new(true),
        }
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for ResolveFailureOnceRepository {
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_mutation_identity_v1(mutation)
    }

    fn canonical_event_batch_identity_v1(
        &self,
        events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_event_batch_identity_v1(events)
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        if self.fail_resolve_once.swap(false, Ordering::SeqCst) {
            return Err(LocalEventQueryError::Internal {
                correlation_id: "provider-lifecycle-resolve-failure".to_string(),
            });
        }
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.inner.query(request).await
    }
}

#[tokio::test]
async fn test_providerライフサイクル永続化_session_start再送とstopをdurableに保存する() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    assert_eq!(armed.provider(), ProviderKind::Claude);
    assert_eq!(armed.scope(), &persistence_scope());
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let started = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), started)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    let duplicate = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), duplicate)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Duplicate
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    let stopped = ProviderLifecycleSignal::stop_observed(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stopped)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 3);
}

#[tokio::test]
async fn test_providerライフサイクル永続化_invalid_capabilityとdomain拒否ではledgerを変更しない() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Codex, persistence_scope())
        .await
        .unwrap();

    let started = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), "wrong-capability", started)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::InvalidCapability)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let stale = ProviderLifecycleSignal::session_started(
        "other-binding",
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stale)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let stop_before_start = ProviderLifecycleSignal::stop_observed(
        armed.binding_id(),
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stop_before_start)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::SessionNotAssociated)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);
}

#[tokio::test]
async fn test_providerライフサイクル再設定_同一slotの旧bindingをdurableに失効させる() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let previous_scope = ProviderLifecycleScope::new("agent-previous").unwrap();
    let current_scope = ProviderLifecycleScope::new("agent-current").unwrap();
    let previous = usecase
        .arm(slot_id(), ProviderKind::Codex, previous_scope.clone())
        .await
        .unwrap();

    let current = usecase
        .arm(slot_id(), ProviderKind::Codex, current_scope)
        .await
        .unwrap();

    assert_ne!(previous.binding_id(), current.binding_id());
    assert_eq!(provider_event_count(&store, "agent-previous").await, 2);
    assert_eq!(provider_event_count(&store, "agent-current").await, 1);
    let stale_signal = ProviderLifecycleSignal::session_started(
        previous.binding_id(),
        ProviderKind::Codex,
        previous_scope,
        "codex-session-stale",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(previous.slot_id(), previous.capability(), stale_signal)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
    assert_eq!(provider_event_count(&store, "agent-previous").await, 2);
    assert_eq!(provider_event_count(&store, "agent-current").await, 1);
}

#[tokio::test]
async fn test_providerライフサイクル再試行_outcome_unknown後もdurable_factを重複させない() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = || {
        ProviderLifecycleSignal::session_started(
            armed.binding_id(),
            ProviderKind::Claude,
            persistence_scope(),
            "claude-session-unknown",
            Some("provider://claude/unknown"),
        )
        .unwrap()
    };
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Duplicate
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}

#[tokio::test]
async fn test_providerライフサイクル再試行_outcome_unknownの照会失敗後も同一commitを確定する() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = Arc::new(ResolveFailureOnceRepository::new(store.clone()));
    let events = LocalProviderLifecycleEventRepository::new(
        repository as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-resolve-retry",
        Some("provider://claude/resolve-retry"),
    )
    .unwrap();
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}

#[tokio::test]
async fn test_providerライフサイクル再試行_outcome_unknownを有限時間で返し再送で同一commitを確定する(
) {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = Arc::new(ResolveFailureRepository::new(store.clone()));
    let events = LocalProviderLifecycleEventRepository::new(
        repository.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = || {
        ProviderLifecycleSignal::session_started(
            armed.binding_id(),
            ProviderKind::Claude,
            persistence_scope(),
            "claude-session-persistent-resolve-failure",
            Some("provider://claude/persistent-resolve-failure"),
        )
        .unwrap()
    };
    repository.set_resolve_failure(true);
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    let first = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        usecase.receive(armed.slot_id(), armed.capability(), session_start()),
    )
    .await
    .expect("persistent resolve failure must not block the Slot indefinitely");
    assert_eq!(
        first,
        Err(crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::StorageUnavailable)
    );

    repository.set_resolve_failure(false);
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}
