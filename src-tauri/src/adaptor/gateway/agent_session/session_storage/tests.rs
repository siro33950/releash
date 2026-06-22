use super::layout::{
    attachment_file_in_dir, attachments_dir_in_dir, content_hash, index_file_in_dir,
    legacy_meta_file, message_file_in_dir, meta_file_in_dir, session_dir, session_file,
    sessions_dir, write_json_pretty_atomic,
};
use super::*;
use crate::usecase::agent_session::session::image_attachment::MAX_IMAGE_BYTES;
use crate::usecase::agent_session::session::{
    AttachmentRef, ChatMessage, ContextCarryState, MessagePart, MessageRole, SessionState,
    SESSION_BODY_FORMAT_VERSION,
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
        backend_id: None,
        workflow_step_session: false,
        workflow_step_context: None,
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
    store
        .persist_message_parts(
            tmp.path(),
            UUID1,
            "m2",
            &[MessagePart::Text {
                content: "updated second".to_string(),
                parent_tool_use_id: None,
            }],
            Some(2002.0),
        )
        .unwrap();

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
    assert_eq!(page.total_count, 3);
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
fn get_session_page_migrates_legacy_flat_json_without_data_loss() {
    let tmp = TempDir::new().unwrap();
    let session = make_session(UUID1, "/repo");
    write_session_json(
        tmp.path(),
        UUID1,
        &serde_json::to_string_pretty(&session).unwrap(),
    );
    let store = FileSessionStorage::default();

    let page = store
        .get_session_page(tmp.path(), UUID1, None, 10)
        .unwrap()
        .unwrap();

    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].content, "Hello");
    assert!(session_dir(tmp.path(), UUID1).unwrap().exists());
    assert!(!session_file(tmp.path(), UUID1).unwrap().exists());
    let loaded = store
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.messages.len(), session.messages.len());
    assert_eq!(loaded.messages[0].id, session.messages[0].id);
    assert_eq!(loaded.messages[0].content, session.messages[0].content);
}

#[test]
fn list_sessions_scans_legacy_flat_metadata_without_storing_all_messages() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &format!(
            r#"{{
                    "id":"{UUID1}",
                    "worktreePath":"/repo",
                    "messages":[
                        {{"id":"m1","role":"human","content":"Hello legacy","timestamp":1000.0}},
                        {{"id":"m2","role":"agent","content":"Legacy reply","timestamp":1001.0}}
                    ],
                    "state":"active",
                    "createdAt":1000.0,
                    "updatedAt":1001.0,
                    "agentSessionId":"agent-session",
                    "contextCarry":"resumed",
                    "permissionMode":"edit",
                    "backendId":"claude",
                    "workflowStepSession":false
                }}"#
        ),
    );
    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, UUID1);
    assert_eq!(summaries[0].first_message, "Hello legacy");
    assert_eq!(summaries[0].message_count, 2);
    assert_eq!(
        summaries[0].agent_session_id.as_deref(),
        Some("agent-session")
    );
    assert_eq!(summaries[0].context_carry, Some(ContextCarryState::Resumed));
    assert_eq!(summaries[0].backend_id.as_deref(), Some("claude"));
    assert!(legacy_meta_file(tmp.path(), UUID1).unwrap().exists());
}

#[test]
fn list_sessions_legacy_flat_skips_non_preview_message_body() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &format!(
            r#"{{
                    "id":"{UUID1}",
                    "worktreePath":"/repo",
                    "messages":[
                        {{"id":"m1","role":"human","content":"Preview only","timestamp":1000.0}},
                        {{
                            "id":"m2",
                            "role":"agent",
                            "content":"",
                            "parts":[{{"type":"image","data":["not","a","string"],"mediaType":"image/png"}}],
                            "timestamp":1001.0
                        }}
                    ],
                    "state":"active",
                    "createdAt":1000.0,
                    "updatedAt":1001.0,
                    "permissionMode":"edit",
                    "workflowStepSession":false
                }}"#
        ),
    );
    let summaries = make_session_store()
        .list_sessions(tmp.path(), "/repo")
        .unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].first_message, "Preview only");
    assert_eq!(summaries[0].message_count, 2);
}

#[test]
fn list_sessions_uses_legacy_flat_sidecar_when_available() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &format!(
            r#"{{
                    "id":"{UUID1}",
                    "worktreePath":"/repo",
                    "messages":[{{"id":"m1","role":"human","content":"Hello legacy","timestamp":1000.0}}],
                    "state":"active",
                    "createdAt":1000.0,
                    "updatedAt":1001.0,
                    "permissionMode":"edit",
                    "workflowStepSession":false
                }}"#
        ),
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
        selected_model: None,
        permission_profile_id: None,
        backend_id: Some("claude".to_string()),
        workflow_step_session: false,
        workflow_step_context: None,
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

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].first_message, "Hello legacy");
    assert_eq!(summaries[0].message_count, 1);
    assert_eq!(
        summaries[0].agent_session_id.as_deref(),
        Some("agent-session")
    );
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
fn save_and_load_session_with_none_backend_id() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStorage::default();
    let session = make_session(UUID1, "/repo");
    assert_eq!(session.backend_id, None);

    store
        .save_full_session_for_migration_or_restore(tmp.path(), &session)
        .unwrap();

    let store2 = FileSessionStorage::default();
    let loaded = store2
        .load_full_session_for_restore(tmp.path(), UUID1)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.backend_id, None);
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

fn session_json_with_permission(session_id: &str, permission_field: Option<&str>) -> String {
    let permission_segment = match permission_field {
        Some(value) => format!(",\"permissionMode\":\"{value}\""),
        None => String::new(),
    };
    format!(
        r#"{{"id":"{session_id}","worktreePath":"/repo"{permission_segment},"messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"workflowStepSession":false}}"#
    )
}

#[test]
fn ensure_loaded_rejects_missing_permission_mode() {
    let tmp = TempDir::new().unwrap();
    write_session_json(
        tmp.path(),
        UUID1,
        &session_json_with_permission(UUID1, None),
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
        write_session_json(
            tmp.path(),
            UUID1,
            &session_json_with_permission(UUID1, Some(invalid)),
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
    // invalid session（旧語彙を含む生 JSON を直接書き込み）
    write_session_json(
        tmp.path(),
        UUID2,
        &session_json_with_permission(UUID2, Some("acceptEdits")),
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
    write_session_json(
        tmp.path(),
        UUID1,
        &session_json_with_permission(UUID2, Some("acceptEdits")),
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
    write_session_json(
        tmp.path(),
        UUID1,
        &session_json_with_permission(UUID1, Some("acceptEdits")),
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
    for (input, expected) in [
        ("readonly", "ask"),
        ("ask", "ask"),
        ("edit", "edit"),
        ("full", "full"),
    ] {
        let tmp = TempDir::new().unwrap();
        write_session_json(
            tmp.path(),
            UUID1,
            &session_json_with_permission(UUID1, Some(input)),
        );
        let store = FileSessionStorage::default();
        let session = store
            .load_full_session_for_restore(tmp.path(), UUID1)
            .unwrap()
            .expect("session loads with valid permission_mode");
        assert_eq!(session.permission_mode, expected);
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
