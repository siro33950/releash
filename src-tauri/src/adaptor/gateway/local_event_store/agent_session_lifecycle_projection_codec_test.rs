use super::{decode_session_projection_record_v1, encode_session_projection_record_v1};
use crate::domain::local_event::{
    ProviderAgentSessionLifecycleRecord, ProviderAgentSessionOriginRecord,
    ProviderAgentSessionProjectionRecord, ProviderAgentSessionProviderRecord,
    ProviderSessionOwnershipProjectionRecord, SessionProjectionRecord,
};
use sha2::{Digest, Sha256};

#[test]
fn test_agent_session_lifecycle_projection_codec_semantic_recordを往復できる() {
    let record =
        SessionProjectionRecord::ProviderAgentSession(ProviderAgentSessionProjectionRecord {
            id: "agent-session-1".to_string(),
            workspace_identity: "/repo".to_string(),
            worktree_path: "/repo/.worktrees/feature".to_string(),
            provider: ProviderAgentSessionProviderRecord::Codex,
            origin: ProviderAgentSessionOriginRecord::Standalone,
            lifecycle: ProviderAgentSessionLifecycleRecord::Paused,
            provider_session_id: Some("provider-session-1".to_string()),
            transcript_ref: Some("provider://transcript/1".to_string()),
            initial_instruction_admitted: false,
            last_exit_abnormal: true,
        });

    let raw = encode_session_projection_record_v1(&record).unwrap();
    let decoded =
        decode_session_projection_record_v1(&raw, "provider-agent-session:agent-session-1")
            .unwrap();

    assert_eq!(decoded, record);
    assert!(!raw.contains("transcript_body"));
    assert!(!raw.contains("conversation"));
    assert!(!raw.contains("secret"));
}

#[test]
fn test_agent_session_lifecycle_projection_codec_last_exit_abnormalなしの旧行をfalseとして復元する()
{
    let legacy = concat!(
        r#"{"schema":"provider_agent_session_projection_v1","id":"agent-session-1","#,
        r#""workspaceIdentity":"/repo","worktreePath":"/repo/.worktrees/feature","#,
        r#""provider":"codex","origin":{"kind":"standalone"},"lifecycle":"paused","#,
        r#""providerSessionId":"provider-session-1","transcriptRef":null,"#,
        r#""initialInstructionAdmitted":false}"#
    );

    let decoded =
        decode_session_projection_record_v1(legacy, "provider-agent-session:agent-session-1")
            .unwrap();

    let SessionProjectionRecord::ProviderAgentSession(projection) = decoded else {
        panic!("provider AgentSession projection expected");
    };
    assert!(!projection.last_exit_abnormal);
}

#[test]
fn test_provider_session_ownership_projection_codec_semantic_recordを往復できる() {
    let record = SessionProjectionRecord::ProviderSessionOwnership(
        ProviderSessionOwnershipProjectionRecord {
            provider: ProviderAgentSessionProviderRecord::Claude,
            provider_session_id: "provider-session-1".to_string(),
            agent_session_id: Some("agent-session-1".to_string()),
        },
    );

    let raw = encode_session_projection_record_v1(&record).unwrap();
    let digest = hex::encode(Sha256::digest(b"provider-session-1"));
    let decoded = decode_session_projection_record_v1(
        &raw,
        &format!("provider-session-ownership:claude:{digest}"),
    )
    .unwrap();

    assert_eq!(decoded, record);
}
