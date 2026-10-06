use super::*;
use std::io::Cursor;

#[test]
fn test_hook受信_上限を超えるstdinを拒否する() {
    let payload = vec![b'x'; 65_537];

    let error = receive_from(Cursor::new(payload), HookProvider::Claude).unwrap_err();

    assert_eq!(
        error,
        CliError::InvalidInput(
            "Provider lifecycle payload exceeds the 65536 byte limit".to_string()
        )
    );
}

#[test]
fn test_hook受信_cli入口はpayload不正や配送失敗でも空jsonを返す() {
    assert_eq!(
        complete_receive(Err(CliError::InvalidInput("invalid payload".to_string()))).unwrap(),
        "{}"
    );
    assert_eq!(
        complete_receive(Err(CliError::Other("rejected".to_string()))).unwrap(),
        "{}"
    );
    assert_eq!(complete_receive(Ok("{}".to_string())).unwrap(), "{}");
}

use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};
#[test]
fn test_hook受信_claude_subagent_signalはlocal_apiへ送らず成功として無視する() {
    let _lock = TEST_ENV_LOCK.lock();
    let _slot_id = EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot-1");
    let _binding_id = EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID", "binding-1");
    let _capability =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY", "capability-1");
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        "agent-session-1",
    );
    let payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/subagent-transcript",
        "cwd":"/workspace",
        "hook_event_name":"Stop",
        "agent_id":"agent-child-1",
        "agent_type":"Explore"
    }"#;

    assert_eq!(
        receive_from(Cursor::new(payload), HookProvider::Claude).unwrap(),
        "{}"
    );
}
