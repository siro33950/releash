use super::layout::{
    attachment_file_in_dir, attachments_dir_in_dir, content_hash, index_file_in_dir,
    legacy_meta_file, message_file_in_dir, meta_file_in_dir, private_context_file_in_dir,
    session_dir, session_file, sessions_dir, tool_output_file_in_dir, tool_outputs_dir_in_dir,
    write_json_pretty_atomic,
};
use super::*;
use crate::domain::agent_session::services::MAX_IMAGE_BYTES;
use crate::domain::agent_session::ContextSourceKind;
use crate::usecase::agent_session::context_meta::{ContextEpochMeta, ContextSourceRevisionMeta};
use crate::usecase::agent_session::session::{
    AttachmentRef, ChatMessage, ContextCarryState, MessagePart, MessageRole, SessionState,
    SESSION_BODY_FORMAT_VERSION, TOOL_OUTPUT_PREVIEW_BYTES,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

const UUID1: &str = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
const UUID2: &str = "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e";
const UUID3: &str = "c3d4e5f6-a7b8-4c9d-ae0f-1a2b3c4d5e6f";

fn make_session_store() -> crate::usecase::agent_session::session::SessionStore {
    crate::test_support::build_session_store()
}

fn make_session(id: &str, worktree: &str) -> ChatSession {
    ChatSession {
        id: id.to_string(),
        worktree_path: worktree.to_string(),
        messages: vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "Hello".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1000.0,
            mentions: None,
        }],
        state: SessionState::Active,
        created_at: 1000.0,
        updated_at: 1000.0,
        agent_session_id: None,
        context_carry: None,
        permission_mode: "edit".to_string(),
        plan_mode: false,
        permission_profile_id: None,
        selected_model: None,
        backend_id: Some("claude".to_string()),
        workflow_step_session: false,
        workflow_step_context: None,
        context_epoch: None,
    }
}

fn context_epoch_meta_with_payload(payload: &str) -> ContextEpochMeta {
    ContextEpochMeta {
        epoch_id: 1,
        backend_id: Some("claude".to_string()),
        model_id: Some("sonnet".to_string()),
        worktree_path: "/repo".to_string(),
        source_revisions: vec![ContextSourceRevisionMeta {
            kind: "repo_summary".to_string(),
            revision: 2,
            fingerprint: Some("repo-fingerprint".to_string()),
            payload: Some(payload.to_string()),
        }],
    }
}

fn message(id: &str, content: &str, timestamp: f64) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: MessageRole::Human,
        content: content.to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp,
        mentions: None,
    }
}

fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47];
    bytes.extend_from_slice(payload);
    bytes
}

fn attachment_blob_count(dir: &std::path::Path) -> usize {
    let attachments_dir = attachments_dir_in_dir(dir);
    if !attachments_dir.exists() {
        return 0;
    }
    std::fs::read_dir(attachments_dir).unwrap().count()
}

fn tool_output_blob_count(dir: &std::path::Path) -> usize {
    let tool_outputs_dir = tool_outputs_dir_in_dir(dir);
    if !tool_outputs_dir.exists() {
        return 0;
    }
    std::fs::read_dir(tool_outputs_dir).unwrap().count()
}

#[test]
fn save_and_load_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, UUID1);
    assert_eq!(loaded.messages.len(), 1);
}

#[test]
fn save_session_writes_split_layout() {
    let tmp = TempDir::new().unwrap();
    let store = make_session_store();
    let session = make_session(UUID1, "/repo");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert!(meta_file_in_dir(&dir).exists());
    assert!(index_file_in_dir(&dir).exists());
    assert!(message_file_in_dir(&dir, 1).exists());
    assert!(!session_file(tmp.path(), UUID1).unwrap().exists());

    let meta: SessionMeta =
        serde_json::from_str(&std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap()).unwrap();
    assert_eq!(meta.id, UUID1);
    assert_eq!(meta.message_count, 1);
    assert_eq!(meta.first_message_preview, "Hello");

    let saved_message = std::fs::read_to_string(message_file_in_dir(&dir, 1)).unwrap();
    let expected = serde_json::to_string_pretty(&session.messages[0]).unwrap();
    assert_eq!(saved_message, expected);
}

#[test]
fn session_meta_keeps_workflow_instructions_in_private_context_not_meta() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    store
        .update_session_meta(tmp.path(), UUID1, &mut |meta| {
            meta.workflow_instructions = vec!["private workflow instruction".to_string()];
            Ok(())
        })
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let meta_json = std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap();
    let private_json = std::fs::read_to_string(private_context_file_in_dir(&dir)).unwrap();
    let loaded_meta = store
        .get_session_meta(tmp.path(), UUID1)
        .unwrap()
        .expect("loaded meta");

    assert!(!meta_json.contains("workflowInstruction"));
    assert!(!meta_json.contains("workflowInstructions"));
    assert!(private_json.contains("workflowInstructions"));
    assert_eq!(
        loaded_meta.workflow_instructions,
        vec!["private workflow instruction".to_string()]
    );
}

#[test]
fn save_session_keeps_context_epoch_payload_in_private_context_not_meta() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.context_epoch = Some(context_epoch_meta_with_payload("repo payload"));

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let meta_json = std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap();
    let private_json = std::fs::read_to_string(private_context_file_in_dir(&dir)).unwrap();
    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .expect("loaded session");

    assert!(meta_json.contains("contextEpoch"));
    assert!(!meta_json.contains("repo payload"));
    assert!(private_json.contains("contextEpochPayloads"));
    assert!(private_json.contains("repo payload"));
    assert_eq!(
        loaded
            .context_epoch
            .as_ref()
            .and_then(|meta| meta.payload_for(ContextSourceKind::RepoSummary)),
        Some("repo payload")
    );
}

#[test]
fn legacy_backend_system_prompt_payload_hydrates_backend_model_identity() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.context_epoch = Some(ContextEpochMeta {
        epoch_id: 1,
        backend_id: Some("claude".to_string()),
        model_id: Some("sonnet".to_string()),
        worktree_path: "/repo".to_string(),
        source_revisions: vec![ContextSourceRevisionMeta {
            kind: "backend_system_prompt".to_string(),
            revision: 2,
            fingerprint: Some("identity-fingerprint".to_string()),
            payload: Some("backend_id: claude\nmodel_id: sonnet".to_string()),
        }],
    });

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let loaded = FileSessionStorage::default()
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .expect("loaded session");
    let context_epoch = loaded.context_epoch.expect("context epoch");

    assert_eq!(
        context_epoch.payload_for(ContextSourceKind::BackendModelIdentity),
        Some("backend_id: claude\nmodel_id: sonnet")
    );
    assert_eq!(
        context_epoch.fingerprint_for(ContextSourceKind::BackendModelIdentity),
        Some("identity-fingerprint")
    );
}

#[test]
fn append_agent_read_paths_updates_private_context_cache_not_meta() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let agent_message = ChatMessage {
        id: "agent-1".to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "src/local/file.rs"}),
            id: "tool-1".to_string(),
            parent_tool_use_id: None,
        }]),
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    };
    store
        .append_message(tmp.path(), UUID1, &agent_message)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let meta_json = std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap();
    let private_json = std::fs::read_to_string(private_context_file_in_dir(&dir)).unwrap();
    let fresh_store = FileSessionStorage::default();
    let meta = fresh_store
        .get_session_meta(tmp.path(), UUID1)
        .unwrap()
        .expect("meta");

    assert!(!meta_json.contains("agentReadPaths"));
    assert!(private_json.contains("agentReadPaths"));
    assert_eq!(
        meta.agent_read_paths,
        Some(vec![std::path::PathBuf::from("src/local/file.rs")])
    );
}

#[test]
fn meta_rmw_update_keeps_context_epoch_payload_cache_for_later_message_updates() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    store
        .update_session_meta(tmp.path(), UUID1, &mut |meta| {
            meta.context_epoch = Some(context_epoch_meta_with_payload("repo payload"));
            Ok(())
        })
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let meta_json = std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap();
    let private_json = std::fs::read_to_string(private_context_file_in_dir(&dir)).unwrap();
    assert!(!meta_json.contains("repo payload"));
    assert!(private_json.contains("repo payload"));

    let fresh_store = FileSessionStorage::default();
    fresh_store
        .append_message(tmp.path(), UUID1, &message("m2", "second", 1001.0))
        .unwrap();
    let meta = fresh_store
        .get_session_meta(tmp.path(), UUID1)
        .unwrap()
        .expect("meta");

    assert_eq!(
        meta.context_epoch
            .as_ref()
            .and_then(|meta| meta.payload_for(ContextSourceKind::RepoSummary)),
        Some("repo payload")
    );
}

#[test]
fn append_message_writes_only_new_chunk_and_updates_meta() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let first_chunk_before = std::fs::read_to_string(message_file_in_dir(&dir, 1)).unwrap();

    let message = ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "second".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    };

    store.append_message(tmp.path(), UUID1, &message).unwrap();

    assert_eq!(
        std::fs::read_to_string(message_file_in_dir(&dir, 1)).unwrap(),
        first_chunk_before
    );
    let second_chunk = store
        .read_message_file(&message_file_in_dir(&dir, 2))
        .unwrap();
    assert_eq!(second_chunk.id, "m2");
    let index = store.read_index_from_dir(&dir).unwrap();
    assert_eq!(
        index
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    let meta = store.get_session_meta(tmp.path(), UUID1).unwrap().unwrap();
    assert_eq!(meta.message_count, 2);
    assert_eq!(meta.first_message_preview, "Hello");
}

#[test]
fn save_session_externalizes_image_attachments() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    let image_bytes = png_bytes(b"image-bytes");
    session.messages[0].parts = Some(vec![MessagePart::Image {
        data: BASE64_STANDARD.encode(&image_bytes),
        media_type: "image/png".to_string(),
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let saved_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    let parts = saved_message.parts.unwrap();
    let attachment = match &parts[0] {
        MessagePart::ImageRef { attachment } => attachment,
        other => panic!("expected image ref, got {other:?}"),
    };
    assert!(attachment_file_in_dir(&dir, &attachment.id).exists());
    let index = store.read_index_from_dir(&dir).unwrap();
    assert_eq!(index[0].attachment_refs, vec![attachment.clone()]);

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();
    assert!(matches!(
        &page.messages[0].parts.as_ref().unwrap()[0],
        MessagePart::ImageRef { .. }
    ));

    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert!(matches!(
        &loaded.messages[0].parts.as_ref().unwrap()[0],
        MessagePart::Image { data, media_type }
            if data == &BASE64_STANDARD.encode(&image_bytes) && media_type == "image/png"
    ));

    let attachment_data = store
        .get_session_attachment(tmp.path(), UUID1, &attachment.id)
        .unwrap()
        .unwrap();
    assert_eq!(attachment_data.data, BASE64_STANDARD.encode(&image_bytes));
    assert_eq!(attachment_data.media_type, "image/png");
}

#[test]
fn save_session_externalizes_large_tool_output_and_pages_preview_ref() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let full_output = format!("{}USER_SECRET_TAIL", "line\n".repeat(1001));
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: full_output.clone(),
        is_error: true,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(tool_output_blob_count(&dir), 1);
    let saved_json = std::fs::read_to_string(message_file_in_dir(&dir, 1)).unwrap();
    assert!(!saved_json.contains("USER_SECRET_TAIL"));
    let saved_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    let part = &saved_message.parts.as_ref().unwrap()[0];
    let MessagePart::ToolResult {
        content,
        is_error,
        content_ref,
        summary,
        ..
    } = part
    else {
        panic!("expected tool result");
    };
    assert!(*is_error);
    assert!(!content.contains("USER_SECRET_TAIL"));
    let content_ref = content_ref.as_ref().expect("large output should have ref");
    let summary = summary.as_ref().expect("large output should have summary");
    assert_eq!(content_ref.byte_size, full_output.len() as u64);
    assert_eq!(summary.byte_size, full_output.len() as u64);
    assert!(summary.truncated);
    let index = store.read_index_from_dir(&dir).unwrap();
    assert_eq!(index[0].tool_output_refs, vec![content_ref.clone()]);
    let Some(crate::usecase::agent_session::session::ActivityEntry::ToolResult {
        content: activity_content,
        content_ref: activity_content_ref,
        summary: activity_summary,
        ..
    }) = saved_message
        .activities
        .as_ref()
        .and_then(|activities| activities.first())
    else {
        panic!("expected legacy tool result activity");
    };
    assert!(!activity_content.contains("USER_SECRET_TAIL"));
    assert_eq!(
        activity_content_ref
            .as_ref()
            .map(|content_ref| content_ref.id.as_str()),
        Some(content_ref.id.as_str())
    );
    assert!(activity_summary
        .as_ref()
        .is_some_and(|summary| summary.truncated));

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();
    let page_json = serde_json::to_string(&page).unwrap();
    assert!(!page_json.contains("USER_SECRET_TAIL"));
    let page_part = &page.messages[0].parts.as_ref().unwrap()[0];
    let MessagePart::ToolResult {
        content: page_content,
        content_ref: page_content_ref,
        ..
    } = page_part
    else {
        panic!("expected paged tool result");
    };
    assert!(page_content.len() <= TOOL_OUTPUT_PREVIEW_BYTES);
    assert_eq!(
        page_content_ref
            .as_ref()
            .map(|content_ref| content_ref.id.as_str()),
        Some(content_ref.id.as_str())
    );

    let restored = store
        .get_session_tool_output(tmp.path(), UUID1, &content_ref.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.content, full_output);
}

#[test]
fn get_session_page_returns_preview_ref_after_tool_output_blob_removed() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let full_output = format!("{}USER_SECRET_TAIL", "line\n".repeat(1001));
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: full_output.clone(),
        is_error: true,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let saved_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    let MessagePart::ToolResult {
        content_ref: Some(content_ref),
        ..
    } = &saved_message.parts.as_ref().unwrap()[0]
    else {
        panic!("expected externalized tool result");
    };
    let content_ref = content_ref.clone();
    std::fs::remove_dir_all(tool_outputs_dir_in_dir(&dir)).unwrap();

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();
    let page_json = serde_json::to_string(&page).unwrap();
    assert!(!page_json.contains("USER_SECRET_TAIL"));
    let MessagePart::ToolResult {
        content: page_content,
        content_ref: page_content_ref,
        ..
    } = &page.messages[0].parts.as_ref().unwrap()[0]
    else {
        panic!("expected paged tool result");
    };
    assert!(page_content.len() <= TOOL_OUTPUT_PREVIEW_BYTES);
    assert_eq!(
        page_content_ref
            .as_ref()
            .map(|content_ref| content_ref.id.as_str()),
        Some(content_ref.id.as_str())
    );
    assert!(store
        .get_session_tool_output(tmp.path(), UUID1, &content_ref.id)
        .unwrap()
        .is_none());
}

#[test]
fn get_session_tool_output_returns_none_for_missing_index_ref_without_rebuild() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let full_output = "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1);
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: full_output.clone(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let mut index = store.read_index_from_dir(&dir).unwrap();
    let content_ref = index[0].tool_output_refs[0].clone();
    index[0].tool_output_refs.clear();
    write_json_pretty_atomic(&index_file_in_dir(&dir), &index, "session index").unwrap();

    store.reset_message_read_count();
    let restored = store
        .get_session_tool_output(tmp.path(), UUID1, &content_ref.id)
        .unwrap();

    assert!(restored.is_none());
    assert_eq!(
        store.message_read_count(),
        0,
        "unreferenced index entries must not trigger full rebuild"
    );
    let unrepaired_index = store.read_index_from_dir(&dir).unwrap();
    assert!(unrepaired_index[0].tool_output_refs.is_empty());
    assert!(tool_output_file_in_dir(&dir, &content_ref.id).exists());
}

#[test]
fn get_session_tool_output_returns_none_for_unreferenced_stale_blob() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let full_output = "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1);
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: full_output,
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let index = store.read_index_from_dir(&dir).unwrap();
    let stale_ref = index[0].tool_output_refs[0].clone();
    let mut updated_message = store
        .read_message_file(&message_file_in_dir(&dir, index[0].seq))
        .unwrap();
    updated_message.parts = Some(vec![MessagePart::ToolResult {
        content: "new small output".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);
    write_json_pretty_atomic(
        &message_file_in_dir(&dir, index[0].seq),
        &updated_message,
        "message chunk",
    )
    .unwrap();

    assert_eq!(tool_output_blob_count(&dir), 1);
    assert_eq!(
        store.read_index_from_dir(&dir).unwrap()[0]
            .tool_output_refs
            .len(),
        1
    );
    assert!(store
        .get_session_tool_output(tmp.path(), UUID1, &stale_ref.id)
        .unwrap()
        .is_none());
    let repaired_index = store.read_index_from_dir(&dir).unwrap();
    assert!(repaired_index[0].tool_output_refs.is_empty());
}

#[test]
fn get_session_tool_output_reads_only_referencing_message_chunk_when_index_is_current() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![
        message("m1", "first", 1000.0),
        {
            let mut message = message("m2", "", 1001.0);
            message.parts = Some(vec![MessagePart::ToolResult {
                content: "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }]);
            message
        },
        message("m3", "third", 1002.0),
    ];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let index = store.read_index_from_dir(&dir).unwrap();
    let content_ref = index[1].tool_output_refs[0].clone();

    store.reset_message_read_count();
    let restored = store
        .get_session_tool_output(tmp.path(), UUID1, &content_ref.id)
        .unwrap()
        .unwrap();

    assert_eq!(restored.content.len(), TOOL_OUTPUT_PREVIEW_BYTES + 1);
    assert_eq!(store.message_read_count(), 1);
}

#[test]
fn get_session_tool_output_returns_none_without_rebuild_when_index_has_no_reference() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let index_before = std::fs::read_to_string(index_file_in_dir(&dir)).unwrap();

    store.reset_message_read_count();
    let restored = store
        .get_session_tool_output(tmp.path(), UUID1, &"b".repeat(64))
        .unwrap();

    assert!(restored.is_none());
    assert_eq!(store.message_read_count(), 0);
    assert_eq!(
        std::fs::read_to_string(index_file_in_dir(&dir)).unwrap(),
        index_before
    );
}

#[test]
fn small_tool_output_stays_inline_without_blob() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: "small output".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(tool_output_blob_count(&dir), 0);
    let saved_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    assert!(matches!(
        &saved_message.parts.as_ref().unwrap()[0],
        MessagePart::ToolResult {
            content,
            content_ref: None,
            summary: None,
            ..
        } if content == "small output"
    ));
}

#[test]
fn externalize_tool_output_write_failure_keeps_inline_fallback() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    std::fs::write(tool_outputs_dir_in_dir(tmp.path()), b"not a directory").unwrap();
    let full_output = "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1);
    let message = ChatMessage {
        id: "m-write-fail".to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::ToolResult {
            content: full_output.clone(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        }]),
        streaming_final_seq: 0,
        timestamp: 1.0,
        mentions: None,
    };

    let fallback = store
        .externalize_message_tool_outputs(tmp.path(), message)
        .expect("write failure should preserve message with inline fallback");

    assert!(matches!(
        &fallback.parts.as_ref().unwrap()[0],
        MessagePart::ToolResult {
            content,
            content_ref: None,
            summary: None,
            ..
        } if content == &full_output
    ));
    assert!(matches!(
        fallback.activities.as_ref().and_then(|activities| activities.first()),
        Some(crate::usecase::agent_session::session::ActivityEntry::ToolResult {
            content,
            content_ref: None,
            summary: None,
            ..
        }) if content == &full_output
    ));
}

#[test]
fn externalize_tool_output_write_failure_log_excludes_body_and_skips_telemetry() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    crate::other::telemetry::reset_test_metrics();
    crate::other::telemetry::set_performance_configured(true);
    crate::other::telemetry::set_performance_enabled(true);
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    std::fs::write(tool_outputs_dir_in_dir(tmp.path()), b"not a directory").unwrap();
    let sentinel = "WRITE_FAILURE_SENTINEL";
    let full_output = format!("{}{sentinel}", "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1));
    let log_message = super::tool_output_blob::tool_output_write_failure_log_message(
        "m-write-fail",
        full_output.len(),
        1,
        "not a directory",
    );
    assert!(!log_message.contains(sentinel));
    let message = ChatMessage {
        id: "m-write-fail".to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::ToolResult {
            content: full_output,
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        }]),
        streaming_final_seq: 0,
        timestamp: 1.0,
        mentions: None,
    };

    let _ = store
        .externalize_message_tool_outputs(tmp.path(), message)
        .expect("write failure should preserve inline fallback");

    let records = crate::other::telemetry::test_metric_records();
    assert!(!records
        .iter()
        .any(|record| record.name == "releash.tool_output.truncated_count"));
    assert!(!records
        .iter()
        .any(|record| record.name == "releash.tool_output.full_output_bytes"));
    crate::other::telemetry::reset_test_metrics();
}

#[test]
fn remove_session_deletes_tool_output_blobs() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(tool_output_blob_count(&dir), 1);

    store.remove_session(tmp.path(), UUID1);

    assert!(!dir.exists());
    assert!(store
        .get_session_tool_output(tmp.path(), UUID1, &"a".repeat(64))
        .unwrap()
        .is_none());
}

#[test]
fn externalized_tool_output_records_safe_telemetry_only() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    crate::other::telemetry::reset_test_metrics();
    crate::other::telemetry::set_performance_configured(true);
    crate::other::telemetry::set_performance_enabled(true);
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let secret = "TELEMETRY_SECRET";
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: format!(
            "{}{secret}",
            "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1)
        ),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let records = crate::other::telemetry::test_metric_records();
    assert!(records
        .iter()
        .any(|record| record.name == "releash.tool_output.truncated_count"));
    assert!(records
        .iter()
        .any(|record| record.name == "releash.tool_output.full_output_bytes"));
    for record in records {
        assert!(record
            .attributes
            .iter()
            .all(|(_, value)| !value.contains(secret)));
    }
    crate::other::telemetry::reset_test_metrics();
}

#[test]
fn repeated_tool_output_persist_records_externalized_telemetry_once() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    crate::other::telemetry::reset_test_metrics();
    crate::other::telemetry::set_performance_configured(true);
    crate::other::telemetry::set_performance_enabled(true);
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![message("m1", "", 1000.0)];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let full_output = "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1);
    let parts = [MessagePart::ToolResult {
        content: full_output.clone(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }];

    store
        .persist_message_parts(tmp.path(), UUID1, "m1", &parts, 1, None)
        .unwrap();
    store
        .persist_message_parts(tmp.path(), UUID1, "m1", &parts, 2, None)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(tool_output_blob_count(&dir), 1);
    let records = crate::other::telemetry::test_metric_records();
    let truncated_records = records
        .iter()
        .filter(|record| record.name == "releash.tool_output.truncated_count")
        .collect::<Vec<_>>();
    let byte_records = records
        .iter()
        .filter(|record| record.name == "releash.tool_output.full_output_bytes")
        .collect::<Vec<_>>();
    assert_eq!(truncated_records.len(), 1);
    assert_eq!(byte_records.len(), 1);
    assert_eq!(byte_records[0].value, full_output.len() as f64);
    crate::other::telemetry::reset_test_metrics();
}

#[test]
fn existing_tool_output_content_ref_passes_through_without_blob_write_or_telemetry() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    crate::other::telemetry::reset_test_metrics();
    crate::other::telemetry::set_performance_configured(true);
    crate::other::telemetry::set_performance_enabled(true);
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![message("m1", "", 1000.0)];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let content_ref = crate::usecase::agent_session::session::ToolOutputRef {
        id: "c".repeat(64),
        byte_size: 4096,
    };
    let summary = crate::usecase::agent_session::session::ToolOutputSummary {
        line_count: 12,
        byte_size: 4096,
        is_error: false,
        truncated: true,
    };
    std::fs::create_dir_all(tool_outputs_dir_in_dir(&dir)).unwrap();
    std::fs::write(
        tool_output_file_in_dir(&dir, &content_ref.id),
        b"existing blob",
    )
    .unwrap();
    let part = MessagePart::ToolResult {
        content: "existing preview".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: Some(content_ref.clone()),
        summary: Some(summary.clone()),
    };

    let persisted_parts = store
        .persist_message_parts(
            tmp.path(),
            UUID1,
            "m1",
            std::slice::from_ref(&part),
            2,
            None,
        )
        .unwrap();

    assert_eq!(tool_output_blob_count(&dir), 1);
    assert_eq!(
        std::fs::read_to_string(tool_output_file_in_dir(&dir, &content_ref.id)).unwrap(),
        "existing blob"
    );
    assert_eq!(persisted_parts, vec![part.clone()]);
    let saved_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    assert_eq!(saved_message.parts.as_ref().unwrap(), &vec![part]);
    let records = crate::other::telemetry::test_metric_records();
    assert!(!records
        .iter()
        .any(|record| record.name == "releash.tool_output.truncated_count"));
    assert!(!records
        .iter()
        .any(|record| record.name == "releash.tool_output.full_output_bytes"));
    crate::other::telemetry::reset_test_metrics();
}

#[test]
fn existing_tool_output_content_ref_rejects_invalid_id_on_persist() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![message("m1", "", 1000.0)];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let part = MessagePart::ToolResult {
        content: "preview".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: Some(crate::usecase::agent_session::session::ToolOutputRef {
            id: "../outside".to_string(),
            byte_size: 7,
        }),
        summary: None,
    };

    let err = store
        .persist_message_parts(tmp.path(), UUID1, "m1", &[part], 2, None)
        .unwrap_err();

    assert!(err.contains("Invalid tool output id"), "got: {err}");
}

#[test]
fn save_session_rejects_oversized_image_attachment_before_blob_write() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    let oversized = vec![0; MAX_IMAGE_BYTES + 4];
    session.messages[0].parts = Some(vec![MessagePart::Image {
        data: BASE64_STANDARD.encode(oversized),
        media_type: "image/png".to_string(),
    }]);

    let err = store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap_err();

    assert!(err.contains("Image too large"), "got: {err}");
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(attachment_blob_count(&dir), 0);
}

#[test]
fn save_session_rejects_image_attachment_media_type_mismatch_before_blob_write() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::Image {
        data: BASE64_STANDARD.encode(png_bytes(b"declared-as-text")),
        media_type: "text/plain".to_string(),
    }]);

    let err = store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap_err();

    assert!(err.contains("media type mismatch"), "got: {err}");
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(attachment_blob_count(&dir), 0);
}

#[test]
fn save_session_rejects_non_image_attachment_bytes_before_blob_write() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::Image {
        data: BASE64_STANDARD.encode(b"not-an-image"),
        media_type: "image/png".to_string(),
    }]);

    let err = store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap_err();

    assert!(err.contains("Unsupported image format"), "got: {err}");
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert_eq!(attachment_blob_count(&dir), 0);
}

#[test]
fn get_session_attachment_rejects_invalid_id_before_blob_read() {
    let tmp = TempDir::new().unwrap();
    let store = make_session_store();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    for invalid in ["../../etc/passwd", "/etc/passwd", "not-a-hex-id"] {
        let err = store
            .get_session_attachment(tmp.path(), UUID1, invalid)
            .unwrap_err();
        assert!(err.contains("Invalid attachment id"), "got: {err}");
    }
}

#[test]
fn get_session_tool_output_rejects_invalid_id_before_blob_read() {
    let tmp = TempDir::new().unwrap();
    let store = make_session_store();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let invalid_ids = [
        "a".repeat(63),
        "a".repeat(65),
        format!("{}g", "a".repeat(63)),
        "../../tool-output".to_string(),
        "aa/{}".replace("{}", &"a".repeat(62)),
    ];
    for invalid in invalid_ids {
        let err = store
            .get_session_tool_output(tmp.path(), UUID1, &invalid)
            .unwrap_err();
        assert!(err.contains("Invalid tool output id"), "got: {err}");
    }
}

#[test]
fn hydrate_attachment_rejects_path_traversal_image_ref() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();

    let attachment = AttachmentRef {
        id: "../../etc/passwd".to_string(),
        media_type: "image/png".to_string(),
        byte_size: 1,
    };
    let err = store.hydrate_attachment(&dir, &attachment).unwrap_err();
    assert!(err.contains("Invalid attachment id"), "got: {err}");

    let attachment = AttachmentRef {
        id: "/etc/passwd".to_string(),
        media_type: "image/png".to_string(),
        byte_size: 1,
    };
    let err = store.hydrate_attachment(&dir, &attachment).unwrap_err();
    assert!(err.contains("Invalid attachment id"), "got: {err}");
}

#[cfg(unix)]
#[test]
fn hydrate_attachment_rejects_canonical_path_outside_attachments_dir() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let outside = tmp.path().join("outside-attachment");
    std::fs::write(&outside, b"outside").unwrap();
    let attachment_id = "a".repeat(64);
    symlink(&outside, attachment_file_in_dir(&dir, &attachment_id)).unwrap();

    let attachment = AttachmentRef {
        id: attachment_id,
        media_type: "image/png".to_string(),
        byte_size: 7,
    };
    let err = store.hydrate_attachment(&dir, &attachment).unwrap_err();
    assert!(
        err.contains("escaped attachments dir"),
        "outside symlink must be rejected, got: {err}"
    );
}

#[cfg(unix)]
#[test]
fn get_session_tool_output_rejects_canonical_path_outside_tool_outputs_dir() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let content_ref = crate::usecase::agent_session::session::ToolOutputRef {
        id: "d".repeat(64),
        byte_size: 24,
    };
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: "preview".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: Some(content_ref.clone()),
        summary: None,
    }]);
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let outside_secret = "OUTSIDE_TOOL_OUTPUT_SECRET";
    let outside = tmp.path().join("outside-tool-output");
    std::fs::write(&outside, outside_secret).unwrap();
    std::fs::create_dir_all(tool_outputs_dir_in_dir(&dir)).unwrap();
    symlink(&outside, tool_output_file_in_dir(&dir, &content_ref.id)).unwrap();

    let err = store
        .get_session_tool_output(tmp.path(), UUID1, &content_ref.id)
        .unwrap_err();

    assert!(
        err.contains("escaped tool outputs dir"),
        "outside symlink must be rejected, got: {err}"
    );
    assert!(!err.contains(outside_secret));
    let direct_err = store.read_tool_output(&dir, &content_ref.id).unwrap_err();
    assert!(
        direct_err.contains("escaped tool outputs dir"),
        "direct blob read must reject outside symlink, got: {direct_err}"
    );
    assert!(!direct_err.contains(outside_secret));
}

#[test]
fn externalize_rejects_invalid_image_ref_id() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let mut message = message("m2", "", 1001.0);
    message.parts = Some(vec![MessagePart::ImageRef {
        attachment: AttachmentRef {
            id: "../outside".to_string(),
            media_type: "image/png".to_string(),
            byte_size: 1,
        },
    }]);

    let err = store
        .externalize_message_attachments(&dir, &message)
        .unwrap_err();
    assert!(err.contains("Invalid attachment id"), "got: {err}");
}

#[test]
fn stale_index_hash_and_attachment_refs_are_repaired_from_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let stale_index = store.read_index_from_dir(&dir).unwrap();
    assert!(stale_index[0].attachment_refs.is_empty());

    let mut updated_message = store
        .read_message_file(&message_file_in_dir(&dir, 1))
        .unwrap();
    updated_message.content.clear();
    let updated_image_bytes = png_bytes(b"updated-image-bytes");
    updated_message.parts = Some(vec![MessagePart::Image {
        data: BASE64_STANDARD.encode(&updated_image_bytes),
        media_type: "image/png".to_string(),
    }]);
    let (stored_message, attachment_refs) = store
        .externalize_message_attachments(&dir, &updated_message)
        .unwrap();
    write_json_pretty_atomic(
        &message_file_in_dir(&dir, stale_index[0].seq),
        &stored_message,
        "message chunk",
    )
    .unwrap();

    let attachment = attachment_refs
        .first()
        .expect("updated message should have attachment")
        .clone();
    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();
    assert!(matches!(
        &page.messages[0].parts.as_ref().unwrap()[0],
        MessagePart::ImageRef { attachment: page_attachment }
            if page_attachment.id == attachment.id
    ));

    let repaired_index = store.read_index_from_dir(&dir).unwrap();
    assert_eq!(
        repaired_index[0].content_hash,
        content_hash(&stored_message).unwrap()
    );
    assert_eq!(repaired_index[0].attachment_refs, vec![attachment.clone()]);

    let attachment_data = store
        .get_session_attachment(tmp.path(), UUID1, &attachment.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        attachment_data.data,
        BASE64_STANDARD.encode(&updated_image_bytes)
    );
}

#[test]
fn get_session_page_returns_latest_page_and_previous_cursor() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages.clear();
    for i in 0..5 {
        session.messages.push(ChatMessage {
            id: format!("m{i}"),
            role: MessageRole::Human,
            content: format!("message {i}"),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1000.0 + f64::from(i),
            mentions: None,
        });
    }
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let latest = store
        .get_session_page(tmp.path(), UUID1, None, 2)
        .unwrap()
        .unwrap();

    assert_eq!(
        latest
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m3", "m4"]
    );
    assert_eq!(
        latest
            .message_metadata
            .iter()
            .map(|metadata| metadata.message_id.as_str())
            .collect::<Vec<_>>(),
        vec!["m3", "m4"]
    );
    assert!(latest.has_more);
    assert!(latest.next_cursor.is_some());
    assert_eq!(latest.total_count, 5);

    let previous = store
        .get_session_page(tmp.path(), UUID1, latest.next_cursor.clone(), 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        previous
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    assert!(previous.has_more);
    assert!(previous.next_cursor.is_some());

    let first = store
        .get_session_page(tmp.path(), UUID1, previous.next_cursor.clone(), 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m0"]
    );
    assert!(!first.has_more);
    assert_eq!(first.next_cursor, None);

    let repeated = store
        .get_session_page(tmp.path(), UUID1, latest.next_cursor, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        repeated
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
}

#[test]
fn get_session_page_reads_only_requested_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = (0..5)
        .map(|i| {
            message(
                &format!("m{i}"),
                &format!("message {i}"),
                1000.0 + f64::from(i),
            )
        })
        .collect();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    store.reset_message_read_count();
    let page = store
        .get_session_page(tmp.path(), UUID1, None, 2)
        .unwrap()
        .unwrap();

    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m3", "m4"]
    );
    assert_eq!(store.message_read_count(), 2);
}

#[test]
fn append_message_does_not_read_existing_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![
        message("m1", "first", 1000.0),
        message("m2", "second", 1001.0),
    ];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    store.reset_message_read_count();
    store
        .append_message(tmp.path(), UUID1, &message("m3", "third", 1002.0))
        .unwrap();

    assert_eq!(store.message_read_count(), 0);
}

#[test]
fn persist_message_parts_missing_target_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let err = store
        .persist_message_parts(
            tmp.path(),
            UUID1,
            "missing-message",
            &[MessagePart::Text {
                content: "updated".to_string(),
                parent_tool_use_id: None,
            }],
            0,
            Some(2000.0),
        )
        .unwrap_err();

    assert!(err.contains(UUID1), "session id missing from error: {err}");
    assert!(
        err.contains("missing-message"),
        "message id missing from error: {err}"
    );
}

#[test]
fn persist_message_parts_updates_only_target_chunk_index_and_meta() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![
        message("m1", "first", 1000.0),
        message("m2", "second", 1001.0),
        message("m3", "third", 1002.0),
    ];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let chunk1 = message_file_in_dir(&dir, 1);
    let chunk2 = message_file_in_dir(&dir, 2);
    let chunk3 = message_file_in_dir(&dir, 3);
    let chunk1_before = std::fs::read_to_string(&chunk1).unwrap();
    let chunk2_before = std::fs::read_to_string(&chunk2).unwrap();
    let chunk3_before = std::fs::read_to_string(&chunk3).unwrap();
    let index_before = std::fs::read_to_string(index_file_in_dir(&dir)).unwrap();
    let meta_before = std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    store.reset_message_read_count();
    let persisted_parts = store
        .persist_message_parts(
            tmp.path(),
            UUID1,
            "m2",
            &[MessagePart::Text {
                content: "updated second".to_string(),
                parent_tool_use_id: None,
            }],
            7,
            Some(2002.0),
        )
        .unwrap();

    assert_eq!(
        persisted_parts,
        vec![MessagePart::Text {
            content: "updated second".to_string(),
            parent_tool_use_id: None,
        }]
    );
    assert_eq!(store.message_read_count(), 1);
    assert_eq!(std::fs::read_to_string(&chunk1).unwrap(), chunk1_before);
    assert_eq!(std::fs::read_to_string(&chunk3).unwrap(), chunk3_before);
    assert_ne!(std::fs::read_to_string(&chunk2).unwrap(), chunk2_before);
    assert_ne!(
        std::fs::read_to_string(index_file_in_dir(&dir)).unwrap(),
        index_before
    );
    assert_ne!(
        std::fs::read_to_string(meta_file_in_dir(&dir)).unwrap(),
        meta_before
    );

    let meta = store.get_session_meta(tmp.path(), UUID1).unwrap().unwrap();
    assert_eq!(meta.message_count, 3);
    assert_eq!(meta.first_message_preview, "first");
    assert_eq!(meta.updated_at, 2002.0);

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "updated second", "third"]
    );
    assert_eq!(page.messages[1].streaming_final_seq, 7);
    assert_eq!(page.total_count, 3);
}

#[test]
fn persist_message_parts_returns_externalized_tool_output_parts() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages = vec![message("m1", "", 1000.0)];
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let full_output = format!(
        "{}PERSIST_RETURN_SECRET_TAIL",
        "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1)
    );

    let persisted_parts = store
        .persist_message_parts(
            tmp.path(),
            UUID1,
            "m1",
            &[MessagePart::ToolResult {
                content: full_output.clone(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }],
            4,
            None,
        )
        .unwrap();

    let MessagePart::ToolResult {
        content,
        content_ref,
        summary,
        ..
    } = &persisted_parts[0]
    else {
        panic!("expected tool result");
    };
    assert!(!content.contains("PERSIST_RETURN_SECRET_TAIL"));
    assert!(content_ref.is_some());
    assert_eq!(
        summary.as_ref().map(|summary| summary.byte_size),
        Some(full_output.len() as u64)
    );
}

#[test]
fn list_sessions_uses_meta_without_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    std::fs::remove_file(message_file_in_dir(&dir, 1)).unwrap();

    let store2 = make_session_store();
    let summaries = store2.list_sessions(tmp.path(), "/repo").unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, UUID1);
    assert_eq!(summaries[0].first_message, "Hello");
    assert_eq!(summaries[0].message_count, 1);
}

#[test]
fn list_worktree_sessions_returns_shells_without_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    std::fs::write(message_file_in_dir(&dir, 1), "{not valid json").unwrap();

    let sessions = make_session_store()
        .list_worktree_sessions(tmp.path(), "/repo")
        .unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, UUID1);
    assert!(sessions[0].messages.is_empty());
}

#[test]
fn get_session_page_repairs_missing_index_entries() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages.push(ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    });
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    std::fs::remove_file(message_file_in_dir(&dir, 1)).unwrap();

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 1)
        .unwrap()
        .unwrap();

    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, "m2");
    assert!(!page.has_more);
    assert_eq!(page.total_count, 1);
    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(summaries[0].message_count, 1);
    assert_eq!(summaries[0].first_message, "Response");
}

#[test]
fn get_session_page_repairs_orphan_message_chunks_missing_from_index() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let orphan = ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    };
    write_json_pretty_atomic(&message_file_in_dir(&dir, 2), &orphan, "message chunk").unwrap();

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();

    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    assert_eq!(page.total_count, 2);
    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(summaries[0].message_count, 2);
    assert_eq!(summaries[0].first_message, "Hello");
}

#[test]
fn concurrent_append_and_page_repair_preserve_index_and_meta() {
    let tmp = TempDir::new().unwrap();
    let app_data_dir = tmp.path().to_path_buf();
    let store = Arc::new(FileSessionStorage::default());
    let mut session = make_session(UUID1, "/repo");
    session.messages = (0..3)
        .map(|i| {
            message(
                &format!("m{i}"),
                &format!("message {i}"),
                1000.0 + f64::from(i),
            )
        })
        .collect();
    store
        .save_full_session_for_migration_or_restore(&app_data_dir, &session)
        .unwrap();
    let dir = session_dir(&app_data_dir, UUID1).unwrap();
    let mut stale_index = store.read_index_from_dir(&dir).unwrap();
    stale_index.pop();
    write_json_pretty_atomic(&index_file_in_dir(&dir), &stale_index, "session index").unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let page_store = Arc::clone(&store);
    let page_dir = app_data_dir.clone();
    let page_barrier = Arc::clone(&barrier);
    let page_thread = std::thread::spawn(move || {
        page_barrier.wait();
        page_store
            .get_session_page(&page_dir, UUID1, None, 2)
            .unwrap()
            .unwrap();
    });

    let append_store = Arc::clone(&store);
    let append_dir = app_data_dir.clone();
    let append_barrier = Arc::clone(&barrier);
    let append_thread = std::thread::spawn(move || {
        append_barrier.wait();
        append_store
            .append_message(&append_dir, UUID1, &message("m3", "message 3", 1003.0))
            .unwrap();
    });

    barrier.wait();
    page_thread.join().unwrap();
    append_thread.join().unwrap();

    let index = store.read_index_from_dir(&dir).unwrap();
    assert_eq!(
        index
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m0", "m1", "m2", "m3"]
    );
    let meta = store
        .get_session_meta(&app_data_dir, UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(meta.message_count, 4);
    assert_eq!(meta.updated_at, 1003.0);
    let loaded = store
        .load_full_session_for_restore(&app_data_dir, UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m0", "m1", "m2", "m3"]
    );
}

#[test]
fn load_full_session_for_restore_repairs_orphan_message_chunks_missing_from_index() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let dir = session_dir(tmp.path(), UUID1).unwrap();
    let orphan = ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    };
    write_json_pretty_atomic(&message_file_in_dir(&dir, 2), &orphan, "message chunk").unwrap();

    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();

    assert_eq!(
        loaded
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    let meta = store.get_session_meta(tmp.path(), UUID1).unwrap().unwrap();
    assert_eq!(meta.message_count, 2);
}

#[test]
fn metadata_only_save_does_not_rewrite_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let chunk = message_file_in_dir(&session_dir(tmp.path(), UUID1).unwrap(), 1);
    let before = std::fs::metadata(&chunk).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    make_session_store()
        .update_plan_mode(tmp.path(), UUID1, true)
        .unwrap();

    let after = std::fs::metadata(&chunk).unwrap().modified().unwrap();
    assert_eq!(after, before);
}

#[test]
fn metadata_only_update_does_not_read_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let chunk = message_file_in_dir(&session_dir(tmp.path(), UUID1).unwrap(), 1);
    std::fs::write(&chunk, "{not valid json").unwrap();

    let session_store = make_session_store();
    session_store
        .update_plan_mode(tmp.path(), UUID1, true)
        .unwrap();

    let meta = session_store
        .get_session_meta(tmp.path(), UUID1)
        .unwrap()
        .expect("meta remains readable");
    assert!(meta.plan_mode);
}

#[test]
fn resume_metadata_updates_do_not_read_message_chunks() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let chunk = message_file_in_dir(&session_dir(tmp.path(), UUID1).unwrap(), 1);
    std::fs::write(&chunk, "{not valid json").unwrap();

    let session_store = make_session_store();
    let meta = session_store
        .update_agent_session_id_if_changed(tmp.path(), UUID1, Some("sdk-session".to_string()))
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent_session_id.as_deref(), Some("sdk-session"));

    let meta = session_store
        .update_context_carry_if_changed(tmp.path(), UUID1, Some(ContextCarryState::Resumed))
        .unwrap()
        .unwrap();
    assert_eq!(meta.context_carry, Some(ContextCarryState::Resumed));

    let meta = session_store
        .update_resume_metadata_if_changed(tmp.path(), UUID1, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent_session_id, None);
    assert_eq!(meta.context_carry, None);
}

#[cfg(unix)]
#[test]
fn fork_session_hardlinks_message_chunks() {
    use std::os::unix::fs::MetadataExt;

    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let forked = make_session_store()
        .fork_session(tmp.path(), UUID1)
        .unwrap();
    let parent_chunk = message_file_in_dir(&session_dir(tmp.path(), UUID1).unwrap(), 1);
    let fork_chunk = message_file_in_dir(&session_dir(tmp.path(), &forked.id).unwrap(), 1);

    assert_eq!(
        std::fs::metadata(parent_chunk).unwrap().ino(),
        std::fs::metadata(fork_chunk).unwrap().ino()
    );
}

#[test]
fn fork_session_copies_tool_output_blobs() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    let full_output = "x".repeat(TOOL_OUTPUT_PREVIEW_BYTES + 1);
    session.messages[0].content.clear();
    session.messages[0].parts = Some(vec![MessagePart::ToolResult {
        content: full_output.clone(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }]);
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let session_store = make_session_store();
    let forked = session_store.fork_session(tmp.path(), UUID1).unwrap();
    let page = session_store
        .get_session_page(tmp.path(), &forked.id, None, 10)
        .unwrap()
        .unwrap();
    let MessagePart::ToolResult {
        content_ref: Some(content_ref),
        ..
    } = &page.messages[0].parts.as_ref().unwrap()[0]
    else {
        panic!("expected forked tool result ref");
    };

    let restored = session_store
        .get_session_tool_output(tmp.path(), &forked.id, &content_ref.id)
        .unwrap()
        .unwrap();

    assert_eq!(restored.content, full_output);
}

#[test]
fn get_session_page_and_restore_ignore_legacy_flat_json() {
    let tmp = TempDir::new().unwrap();
    let session = make_session(UUID1, "/repo");
    write_session_json(
        tmp.path(),
        UUID1,
        &serde_json::to_string_pretty(&session).unwrap(),
    );
    let store = FileSessionStorage::default();

    let page = store.get_session_page(tmp.path(), UUID1, None, 10).unwrap();
    let restored = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap();

    assert!(page.is_none());
    assert!(restored.is_none());
    assert!(!session_dir(tmp.path(), UUID1).unwrap().exists());
    assert!(session_file(tmp.path(), UUID1).unwrap().exists());
}

#[test]
fn list_sessions_ignores_legacy_flat_json_and_sidecar() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &serde_json::to_string_pretty(&make_session(UUID1, "/repo")).unwrap(),
    );
    let sidecar = SessionMeta {
        id: UUID1.to_string(),
        worktree_path: "/repo".to_string(),
        state: SessionState::Active,
        created_at: 1000.0,
        updated_at: 1001.0,
        agent_session_id: Some("agent-session".to_string()),
        context_carry: Some(ContextCarryState::Resumed),
        permission_mode: "edit".to_string(),
        plan_mode: false,
        selected_model: Some("gpt-5".to_string()),
        permission_profile_id: None,
        backend_id: "codex".to_string(),
        workflow_step_session: false,
        workflow_step_context: None,
        workflow_instructions: Vec::new(),
        agent_read_paths: None,
        context_epoch: None,
        first_message_preview: "Hello legacy".to_string(),
        message_count: 1,
        body_format_version: SESSION_BODY_FORMAT_VERSION,
    };
    write_json_pretty_atomic(
        &legacy_meta_file(tmp.path(), UUID1).unwrap(),
        &sidecar,
        "legacy session meta",
    )
    .unwrap();

    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();

    assert!(summaries.is_empty());
    assert!(legacy_meta_file(tmp.path(), UUID1).unwrap().exists());
}

#[test]
fn get_session_review_context_ignores_legacy_flat_json_and_sidecar() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &serde_json::to_string_pretty(&make_session(UUID1, "/repo")).unwrap(),
    );
    let sidecar = legacy_meta_file(tmp.path(), UUID1).unwrap();
    std::fs::write(&sidecar, "{not-json").unwrap();

    let context = FileSessionStorage::default()
        .get_session_review_context(tmp.path(), UUID1)
        .unwrap();

    assert!(context.is_none());
    assert!(sidecar.exists());
}

#[test]
fn get_session_review_context_reads_only_target_split_meta_without_cache_warmup() {
    let tmp = TempDir::new().unwrap();
    let store_for_save = FileSessionStorage::default();
    store_for_save
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let invalid_dir = session_dir(tmp.path(), UUID2).unwrap();
    std::fs::create_dir_all(&invalid_dir).unwrap();
    std::fs::write(meta_file_in_dir(&invalid_dir), "{not-json").unwrap();

    let store = FileSessionStorage::default();
    store.reset_message_read_count();
    let context = store
        .get_session_review_context(tmp.path(), UUID1)
        .unwrap()
        .unwrap();

    assert_eq!(context.id, UUID1);
    assert_eq!(context.worktree_path, "/repo");
    assert!(!store.loaded.load(std::sync::atomic::Ordering::Acquire));
    assert!(store.cache.read().is_empty());
    assert!(store.invalid_sessions.read().is_empty());
    assert_eq!(store.message_read_count(), 0);
}

#[test]
fn save_and_load_session_with_backend_id() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.backend_id = Some("claude".to_string());

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    // Load from a fresh store to verify file persistence
    let store2 = FileSessionStorage::default();
    let loaded = store2
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.backend_id, Some("claude".to_string()));
}

#[test]
fn save_session_with_none_backend_id_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.backend_id = None;
    assert_eq!(session.backend_id, None);

    let error = store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap_err();
    assert!(error.contains("Invalid session data"));
}

#[test]
fn load_session_missing_backend_id_is_isolated_as_invalid() {
    let tmp = TempDir::new().unwrap();
    write_session_meta_json(
        tmp.path(),
        UUID1,
        &format!(
            r#"{{
                "id":"{UUID1}",
                "worktreePath":"/repo",
                "state":"active",
                "createdAt":1000.0,
                "updatedAt":1000.0,
                "permissionMode":"edit",
                "planMode":false,
                "selectedModel":"claude-4-sonnet",
                "firstMessagePreview":"",
                "messageCount":0,
                "bodyFormatVersion":{SESSION_BODY_FORMAT_VERSION}
            }}"#
        ),
    );
    let store = FileSessionStorage::default();

    let error = store.get_session_meta(tmp.path(), UUID1).unwrap_err();

    assert!(error.contains("Invalid session data"));
    assert!(store.invalid_sessions.read().contains_key(UUID1));
}

#[test]
fn list_sessions_filters_by_worktree() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo-a"))
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID2, "/repo-b"))
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID3, "/repo-a"))
        .unwrap();

    let sessions = make_session_store()
        .list_sessions(tmp.path(), "/repo-a")
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|s| s.worktree_path == "/repo-a"));
}

#[test]
fn get_nonexistent_session_returns_none() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let result = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn save_overwrites_existing_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    session.messages.push(ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    });
    session.updated_at = 1001.0;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.messages.len(), 2);
}

#[test]
fn save_session_removes_chunks_for_deleted_messages() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.messages.push(ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    });
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    session.messages.pop();
    session.updated_at = 1002.0;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let dir = session_dir(tmp.path(), UUID1).unwrap();
    assert!(!message_file_in_dir(&dir, 2).exists());
    std::fs::remove_file(index_file_in_dir(&dir)).unwrap();

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();

    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1"]
    );
    assert_eq!(page.total_count, 1);
}

#[test]
fn list_sessions_sorted_by_updated_at_desc() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    let mut s1 = make_session(UUID1, "/repo");
    s1.updated_at = 1000.0;
    let mut s2 = make_session(UUID2, "/repo");
    s2.updated_at = 2000.0;
    let mut s3 = make_session(UUID3, "/repo");
    s3.updated_at = 1500.0;

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &s1)
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &s2)
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &s3)
        .unwrap();

    let sessions = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(sessions[0].id, UUID2);
    assert_eq!(sessions[1].id, UUID3);
    assert_eq!(sessions[2].id, UUID1);
}

#[test]
fn persistence_across_store_instances() {
    let tmp = TempDir::new().unwrap();
    let store1 = FileSessionStorage::default();
    store1
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let store2 = FileSessionStorage::default();
    let loaded = store2
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap();
    assert!(loaded.is_some());
}

#[test]
fn session_file_validates_uuid() {
    let tmp = TempDir::new().unwrap();
    assert!(session_file(tmp.path(), UUID1).is_ok());
}

#[test]
fn session_file_rejects_non_uuid() {
    let tmp = TempDir::new().unwrap();
    assert!(session_file(tmp.path(), "not-a-uuid").is_err());
}

#[test]
fn session_file_rejects_path_traversal() {
    let tmp = TempDir::new().unwrap();
    assert!(session_file(tmp.path(), "../../../etc/passwd").is_err());
    assert!(session_file(tmp.path(), "..").is_err());
    assert!(session_file(tmp.path(), "foo/bar").is_err());
}

#[test]
fn save_session_rejects_invalid_permission_mode() {
    // Production 保存経路の SessionStore が PermissionMode::parse を通す。
    // 旧語彙・未知語彙・空文字は許可一覧付きエラーで拒否し、cache に残らない。
    let tmp = TempDir::new().unwrap();
    let store = make_session_store();
    let valid = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &valid)
        .unwrap();

    for invalid in [
        "acceptEdits",
        "bypassPermissions",
        "plan",
        "default",
        "unknown",
        "",
    ] {
        let mut bad = make_session(UUID2, "/repo");
        bad.permission_mode = invalid.to_string();
        let err = store
            .save_full_session_for_migration_or_restore(tmp.path(), &bad)
            .unwrap_err();
        assert!(
            err.contains("ask, edit, full"),
            "invalid '{invalid}' must include allowed list, got: {err}"
        );
        // cache に invalid なセッションが残らないこと。
        assert!(store
            .load_full_session_for_restore(tmp.path(), UUID2)
            .unwrap()
            .is_none());
    }
    // 既存の valid セッションは破壊されない。
    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "edit");
}

#[test]
fn save_session_rejects_invalid_id() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session("bad-id", "/repo");
    assert!(store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .is_err());
}

#[test]
fn list_sessions_excludes_closed() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let mut closed = make_session(UUID2, "/repo");
    closed.state = SessionState::Closed;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &closed)
        .unwrap();

    let sessions = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, UUID1);
}

#[test]
fn list_sessions_excludes_archived() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let mut archived = make_session(UUID2, "/repo");
    archived.state = SessionState::Archived;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &archived)
        .unwrap();

    let session_store = make_session_store();
    let sessions = session_store.list_sessions(tmp.path(), "/repo").unwrap();
    let closed = session_store
        .list_closed_sessions(tmp.path(), "/repo")
        .unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, UUID1);
    assert!(closed.is_empty());
}

#[test]
fn archive_session_moves_closed_session_out_of_closed_history() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    let mut closed = make_session(UUID1, "/repo");
    closed.state = SessionState::Closed;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &closed)
        .unwrap();

    let session_store = make_session_store();
    session_store.archive_session(tmp.path(), UUID1).unwrap();

    let saved = session_store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(saved.state, SessionState::Archived);
    assert!(session_store
        .list_closed_sessions(tmp.path(), "/repo")
        .unwrap()
        .is_empty());
}

#[test]
fn archive_open_session_archives_active_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let session_store = make_session_store();
    session_store
        .archive_open_session(tmp.path(), UUID1)
        .unwrap();

    let saved = session_store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(saved.state, SessionState::Archived);
    assert!(session_store
        .list_sessions(tmp.path(), "/repo")
        .unwrap()
        .is_empty());
    assert!(session_store
        .list_closed_sessions(tmp.path(), "/repo")
        .unwrap()
        .is_empty());
}

#[test]
fn archive_open_session_rejects_workflow_step_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.workflow_step_session = true;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let err = make_session_store()
        .archive_open_session(tmp.path(), UUID1)
        .unwrap_err();

    assert_eq!(err, "Workflow step sessions cannot be archived");
}

#[test]
fn set_session_title_overrides_summary_and_can_clear() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let session_store = make_session_store();
    let summary = session_store
        .set_session_title(tmp.path(), UUID1, Some("  Custom   title  "))
        .unwrap();

    assert_eq!(summary.first_message, "Custom title");
    let sessions = session_store.list_sessions(tmp.path(), "/repo").unwrap();
    assert_eq!(sessions[0].first_message, "Custom title");

    let summary = session_store
        .set_session_title(tmp.path(), UUID1, None)
        .unwrap();

    assert_eq!(summary.first_message, "Hello");
    let sessions = session_store.list_sessions(tmp.path(), "/repo").unwrap();
    assert_eq!(sessions[0].first_message, "Hello");
}

#[test]
fn set_session_title_rejects_workflow_step_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.workflow_step_session = true;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let err = make_session_store()
        .set_session_title(tmp.path(), UUID1, Some("Step title"))
        .unwrap_err();

    assert_eq!(err, "Workflow step sessions cannot be renamed");
}

#[test]
fn fork_session_creates_detached_copy() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    let mut session = make_session(UUID1, "/repo");
    session.agent_session_id = Some("agent-session".to_string());
    session.selected_model = Some("claude-opus".to_string());
    session.backend_id = Some("claude".to_string());
    session.messages.push(ChatMessage {
        id: "m2".to_string(),
        role: MessageRole::Agent,
        content: "Response".to_string(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp: 1001.0,
        mentions: None,
    });
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let session_store = make_session_store();
    let forked = session_store.fork_session(tmp.path(), UUID1).unwrap();

    assert_ne!(forked.id, UUID1);
    assert_eq!(forked.worktree_path, "/repo");
    assert_eq!(forked.state, SessionState::Idle);
    assert_eq!(forked.agent_session_id, None);
    assert_eq!(forked.permission_mode, "edit");
    assert_eq!(forked.selected_model, Some("claude-opus".to_string()));
    assert_eq!(forked.backend_id, Some("claude".to_string()));
    assert!(forked.messages.is_empty());
    let page = session_store
        .get_session_page(tmp.path(), &forked.id, None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    assert!(session_store
        .load_full_session_for_restore(tmp.path(), &forked.id)
        .unwrap()
        .is_some());
}

#[test]
fn fork_session_persists_context_epoch_payload_for_fresh_load() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.context_epoch = Some(context_epoch_meta_with_payload("repo payload"));
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let forked = make_session_store()
        .fork_session(tmp.path(), UUID1)
        .unwrap();
    let fork_dir = session_dir(tmp.path(), &forked.id).unwrap();
    assert!(private_context_file_in_dir(&fork_dir).exists());

    let loaded = make_session_store()
        .load_full_session_for_restore(tmp.path(), &forked.id)
        .unwrap()
        .expect("freshly loaded fork");

    assert_eq!(
        loaded
            .context_epoch
            .as_ref()
            .and_then(|meta| meta.payload_for(ContextSourceKind::RepoSummary)),
        Some("repo payload")
    );
}

#[test]
fn fork_session_copies_custom_title() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let session_store = make_session_store();
    session_store
        .set_session_title(tmp.path(), UUID1, Some("Custom title"))
        .unwrap();

    let forked = session_store.fork_session(tmp.path(), UUID1).unwrap();

    assert_eq!(
        session_store
            .session_title(tmp.path(), &forked.id)
            .unwrap()
            .as_deref(),
        Some("Custom title")
    );
}

#[test]
fn fork_session_rejects_workflow_step_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let mut session = make_session(UUID1, "/repo");
    session.workflow_step_session = true;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let err = make_session_store()
        .fork_session(tmp.path(), UUID1)
        .unwrap_err();

    assert_eq!(err, "Workflow step sessions cannot be forked");
}

#[test]
fn update_permission_mode_persists() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    assert_eq!(session.permission_mode, "edit");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let session_store = make_session_store();
    session_store
        .update_permission_mode(tmp.path(), UUID1, "ask")
        .unwrap();

    let loaded = session_store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "ask");
}

#[test]
fn update_plan_mode_persists() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    assert!(!session.plan_mode);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    let session_store = make_session_store();
    session_store
        .update_plan_mode(tmp.path(), UUID1, true)
        .unwrap();

    let loaded = session_store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert!(loaded.plan_mode);
    let summary = loaded.to_summary();
    assert!(summary.plan_mode);
}

#[test]
fn update_plan_mode_nonexistent_session_returns_error() {
    let tmp = TempDir::new().unwrap();
    let result = make_session_store().update_plan_mode(tmp.path(), UUID1, true);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Session not found"));
}

#[test]
fn update_permission_mode_nonexistent_session_returns_error() {
    let tmp = TempDir::new().unwrap();
    let result = make_session_store().update_permission_mode(tmp.path(), UUID1, "ask");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Session not found"));
}

#[test]
fn update_permission_mode_rejects_legacy_value() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();
    for legacy in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
        let err = make_session_store()
            .update_permission_mode(tmp.path(), UUID1, legacy)
            .unwrap_err();
        assert!(
            err.contains("ask, edit, full"),
            "legacy '{legacy}' must be rejected with allowed list, got: {err}"
        );
    }
    // Ensure the persisted value was not corrupted by failed attempts
    let loaded = make_session_store()
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "edit");
}

fn write_session_json(dir: &Path, session_id: &str, json: &str) {
    let sessions = sessions_dir(dir);
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join(format!("{session_id}.json"));
    std::fs::write(&file, json).unwrap();
}

fn write_session_meta_json(dir: &Path, session_id: &str, json: &str) {
    let dir = session_dir(dir, session_id).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(meta_file_in_dir(&dir), json).unwrap();
}

fn session_meta_json_with_permission(session_id: &str, permission_field: Option<&str>) -> String {
    let permission_segment = match permission_field {
        Some(value) => format!(",\"permissionMode\":\"{value}\""),
        None => String::new(),
    };
    format!(
        r#"{{"id":"{session_id}","worktreePath":"/repo","state":"active","createdAt":1000.0,"updatedAt":1000.0{permission_segment},"workflowStepSession":false,"firstMessagePreview":"","messageCount":0,"bodyFormatVersion":1,"backendId":"claude"}}"#
    )
}

#[test]
fn ensure_loaded_rejects_missing_permission_mode() {
    let tmp = TempDir::new().unwrap();
    write_session_meta_json(
        tmp.path(),
        UUID1,
        &session_meta_json_with_permission(UUID1, None),
    );
    let store = FileSessionStorage::default();
    let err = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap_err();
    assert!(
        err.contains("ask, edit, full"),
        "missing permissionMode must be rejected with allowed list, got: {err}"
    );
}

#[test]
fn ensure_loaded_rejects_legacy_and_unknown_permission_modes() {
    for invalid in [
        "acceptEdits",
        "bypassPermissions",
        "plan",
        "default",
        "unknown",
        "",
    ] {
        let tmp = TempDir::new().unwrap();
        write_session_meta_json(
            tmp.path(),
            UUID1,
            &session_meta_json_with_permission(UUID1, Some(invalid)),
        );
        let store = FileSessionStorage::default();
        let err = store
            .load_full_session_for_restore(tmp.path(), UUID1)
            .unwrap_err();
        assert!(
            err.contains("ask, edit, full"),
            "invalid permissionMode '{invalid}' must be rejected with allowed list, got: {err}"
        );
    }
}

#[test]
fn invalid_session_is_ignored_by_list_but_rejected_by_targeted_operations() {
    // Spec issues-947: 一覧では invalid session を隔離し、無関係な正常 session を返す。
    // 個別取得や更新では invalid session は汎化済みエラーを返す。
    let tmp = TempDir::new().unwrap();
    // valid session
    let store_for_save = FileSessionStorage::default();
    store_for_save
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    // invalid session（旧語彙を含む meta.json を直接書き込み）
    write_session_meta_json(
        tmp.path(),
        UUID2,
        &session_meta_json_with_permission(UUID2, Some("acceptEdits")),
    );

    let store = make_session_store();

    let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, UUID1);

    let sessions = store.list_worktree_sessions(tmp.path(), "/repo").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, UUID1);

    // valid 単体取得は成功
    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap();
    assert_eq!(loaded.unwrap().id, UUID1);

    // invalid 単体取得は許可一覧付きエラー（生パス・serde メッセージは含まない）
    let err = store
        .load_full_session_for_restore(tmp.path(), UUID2)
        .unwrap_err();
    assert!(err.contains("ask, edit, full"), "got: {err}");
    assert!(
        !err.contains(tmp.path().to_str().unwrap()),
        "path must not leak: {err}"
    );
    assert!(!err.contains(".json"), "filename must not leak: {err}");

    // update_permission_mode も invalid は弾く
    let err = store
        .update_permission_mode(tmp.path(), UUID2, "edit")
        .unwrap_err();
    assert!(err.contains("ask, edit, full"), "got: {err}");
}

#[test]
fn invalid_session_isolation_key_uses_file_session_id_for_permission_errors() {
    let tmp = TempDir::new().unwrap();
    write_session_meta_json(
        tmp.path(),
        UUID1,
        &session_meta_json_with_permission(UUID2, Some("acceptEdits")),
    );
    let store = FileSessionStorage::default();

    let err = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap_err();
    assert!(
        err.contains(UUID1),
        "file id must be the invalid key: {err}"
    );
    let by_json_id = store
        .load_full_session_for_restore(tmp.path(), UUID2)
        .unwrap();
    assert!(by_json_id.is_none());

    let listed = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert!(listed.is_empty());
}

#[test]
fn save_session_removes_stale_invalid_marker_for_same_id() {
    let tmp = TempDir::new().unwrap();
    write_session_meta_json(
        tmp.path(),
        UUID1,
        &session_meta_json_with_permission(UUID1, Some("acceptEdits")),
    );
    let store = FileSessionStorage::default();
    let err = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap_err();
    assert!(err.contains("ask, edit, full"), "got: {err}");

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();

    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "edit");
    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, UUID1);
}

#[test]
fn ensure_loaded_normalizes_legacy_and_accepts_valid_permission_modes() {
    for (input, expected) in [("ask", "ask"), ("edit", "edit"), ("full", "full")] {
        let tmp = TempDir::new().unwrap();
        write_session_meta_json(
            tmp.path(),
            UUID1,
            &session_meta_json_with_permission(UUID1, Some(input)),
        );
        let store = FileSessionStorage::default();
        let meta = store
            .get_session_meta(tmp.path(), UUID1)
            .unwrap()
            .expect("meta loads with valid permission_mode");
        assert_eq!(meta.permission_mode, expected);
    }
}

#[test]
fn state_change_listener_fires_on_close_and_restore() {
    use parking_lot::Mutex as PlMutex;

    let tmp = TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let events: Arc<PlMutex<Vec<(String, String, SessionState)>>> =
        Arc::new(PlMutex::new(Vec::new()));
    let events_for_listener = events.clone();
    store.register_state_change_listener(Arc::new(move |session_id, worktree_path, new_state| {
        events_for_listener.lock().push((
            session_id.to_string(),
            worktree_path.to_string(),
            new_state.clone(),
        ));
    }));

    // タブを閉じる: Active → Closed
    store
        .set_session_state(tmp.path(), UUID1, SessionState::Closed)
        .unwrap();
    // 復帰: Closed → Idle
    store
        .set_session_state(tmp.path(), UUID1, SessionState::Idle)
        .unwrap();

    let captured = events.lock().clone();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].0, UUID1);
    assert_eq!(captured[0].1, "/repo");
    assert_eq!(captured[0].2, SessionState::Closed);
    assert_eq!(captured[1].2, SessionState::Idle);
}

#[test]
fn state_change_listener_does_not_fire_when_state_unchanged() {
    use parking_lot::Mutex as PlMutex;

    let tmp = TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();
    let session = make_session(UUID1, "/repo");
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let count = Arc::new(PlMutex::new(0usize));
    let count_for_listener = count.clone();
    store.register_state_change_listener(Arc::new(move |_, _, _| {
        *count_for_listener.lock() += 1;
    }));

    // 状態は変えずに permission_mode を更新する（Spec issues-947 で抽象3値のみ受理）
    store
        .update_permission_mode(tmp.path(), UUID1, "ask")
        .unwrap();

    assert_eq!(*count.lock(), 0);
}

#[test]
fn list_closed_sessions_returns_only_closed() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &make_session(UUID1, "/repo"))
        .unwrap();
    let mut closed1 = make_session(UUID2, "/repo");
    closed1.state = SessionState::Closed;
    closed1.updated_at = 2000.0;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &closed1)
        .unwrap();
    let mut closed2 = make_session(UUID3, "/repo");
    closed2.state = SessionState::Closed;
    closed2.updated_at = 3000.0;
    store
        .save_full_session_for_migration_or_restore(tmp.path(), &closed2)
        .unwrap();

    let closed = make_session_store()
        .list_closed_sessions(tmp.path(), "/repo")
        .unwrap();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[0].id, UUID3);
    assert_eq!(closed[1].id, UUID2);
}
