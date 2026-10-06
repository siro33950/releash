use super::launch_spec::ProviderLaunchSpecError;
use super::test_helpers::*;
use super::*;
use crate::domain::agent_session::{ProviderSessionLaunch, ProviderSessionLaunchError};
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleSignalKind};
use crate::domain::workflow::AgentSessionActivity;

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
