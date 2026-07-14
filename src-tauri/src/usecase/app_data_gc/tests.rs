use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::app_data_gc::{GcCategory, RetentionPolicy};
use crate::usecase::agent_session::session::{AttachmentRef, MessagePart, ToolOutputRef};

use super::test_fixtures::*;
use super::{
    run_startup_gc, CurrentSessionState, CurrentWorkflowExecutionState, GcRevalidationReader,
    GcWorktreePath, ProcessRecord, ProcessRecordStatus, RevalidationRead, RuntimeProtection,
};

#[test]
fn deleted_workspace_session_workflow_and_workspace_state_are_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let deleted = tmp.path().join("deleted-worktree");
    let live_session = write_session(tmp.path(), "live-session", live.path(), "idle", NOW);
    let deleted_session = write_session(tmp.path(), "deleted-session", &deleted, "idle", NOW);
    write_workflow_execution(tmp.path(), "deleted-execution", &deleted, "completed");
    write_workflow_execution(tmp.path(), "live-execution", live.path(), "completed");
    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    fs::write(
        tmp.path()
            .join("workspace_state")
            .join(format!("{}.json", workspace_state_storage_key("live"))),
        "{}",
    )
    .unwrap();
    fs::write(
        tmp.path().join("workspace_state").join("deleted.json"),
        "{}",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("review-comments")).unwrap();
    let live_review_key = review_comment_storage_key(&live.path().to_string_lossy());
    fs::write(
        tmp.path()
            .join("review-comments")
            .join(format!("{live_review_key}.events.json")),
        "[]",
    )
    .unwrap();
    fs::write(
        tmp.path()
            .join("review-comments/deleted-review.events.json"),
        "[]",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("agent-worktree-checkpoints")).unwrap();
    fs::write(
        tmp.path()
            .join("agent-worktree-checkpoints")
            .join(format!("{}.json", workspace_state_storage_key("live"))),
        "{}",
    )
    .unwrap();
    fs::write(
        tmp.path().join("agent-worktree-checkpoints/deleted.json"),
        "{}",
    )
    .unwrap();

    run_gc(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(live_session.exists());
    assert!(!deleted_session.exists());
    assert!(!tmp
        .path()
        .join("workflow_executions/deleted-execution.json")
        .exists());
    assert!(!tmp
        .path()
        .join("workflow_execution_logs/deleted-execution.ndjson")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_executions/live-execution.json")
        .exists());
    assert!(tmp
        .path()
        .join("workspace_state")
        .join(format!("{}.json", workspace_state_storage_key("live")))
        .exists());
    assert!(!tmp.path().join("workspace_state/deleted.json").exists());
    assert!(tmp
        .path()
        .join("review-comments")
        .join(format!("{live_review_key}.events.json"))
        .exists());
    assert!(!tmp
        .path()
        .join("review-comments/deleted-review.events.json")
        .exists());
    assert!(tmp
        .path()
        .join("agent-worktree-checkpoints")
        .join(format!("{}.json", workspace_state_storage_key("live")))
        .exists());
    assert!(tmp
        .path()
        .join("agent-worktree-checkpoints/deleted.json")
        .exists());
}

#[cfg(unix)]
#[test]
fn symlinked_live_worktree_keeps_in_use_sessions_and_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let real_worktree = root.path().join("real-worktree");
    let symlink_worktree = root.path().join("symlink-worktree");
    fs::create_dir_all(&real_worktree).unwrap();
    std::os::unix::fs::symlink(&real_worktree, &symlink_worktree).unwrap();
    let idle = write_session(tmp.path(), "idle-session", &symlink_worktree, "idle", NOW);
    let done = write_session(tmp.path(), "done-session", &symlink_worktree, "done", NOW);
    let error = write_session(tmp.path(), "error-session", &symlink_worktree, "error", NOW);
    write_workflow_execution(
        tmp.path(),
        "completed-execution",
        &symlink_worktree,
        "completed",
    );

    run_gc(
        tmp.path(),
        Some(live_set(&[("live", &real_worktree)])),
        vec![],
        NOW,
    );

    assert!(idle.exists());
    assert!(done.exists());
    assert!(error.exists());
    assert!(tmp
        .path()
        .join("workflow_executions/completed-execution.json")
        .exists());
}

#[test]
fn unresolved_worktree_path_keeps_session_and_workflow_conservatively() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("blocked-worktree");
    let session = write_session(tmp.path(), "session", &missing, "idle", NOW);
    write_workflow_execution(tmp.path(), "completed-execution", &missing, "completed");
    let mut request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    for record in &mut request.session_records {
        record.worktree_path = Some(GcWorktreePath::unresolved(missing.to_string_lossy()));
    }
    for execution in &mut request.workflow_executions {
        execution.worktree_path = GcWorktreePath::unresolved(missing.to_string_lossy());
    }

    run_startup_gc(
        request,
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert!(session.exists());
    assert!(tmp
        .path()
        .join("workflow_executions/completed-execution.json")
        .exists());
}

#[test]
fn session_state_rules_keep_archived_until_retention_expires() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let archived_old = write_session(
        tmp.path(),
        "archived-old",
        live.path(),
        "archived",
        NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
    );
    let archived_boundary = write_session(
        tmp.path(),
        "archived-boundary",
        live.path(),
        "archived",
        NOW - RetentionPolicy::default().archived_log_secs as f64,
    );
    let closed_old = write_session(
        tmp.path(),
        "closed-old",
        live.path(),
        "closed",
        NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
    );
    let closed_boundary = write_session(
        tmp.path(),
        "closed-boundary",
        live.path(),
        "closed",
        NOW - RetentionPolicy::default().archived_log_secs as f64,
    );
    let idle_old = write_session(
        tmp.path(),
        "idle-old",
        live.path(),
        "idle",
        NOW - RetentionPolicy::default().archived_log_secs as f64 - 1000.0,
    );

    run_gc(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(!archived_old.exists());
    assert!(archived_boundary.exists());
    assert!(!closed_old.exists());
    assert!(closed_boundary.exists());
    assert!(idle_old.exists());
}

#[test]
fn workflow_archive_retention_and_running_protection() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    write_workflow_execution(tmp.path(), "old-archive", live.path(), "completed");
    write_workflow_execution(tmp.path(), "boundary-archive", live.path(), "completed");
    write_workflow_execution(tmp.path(), "restored-archive", live.path(), "completed");
    write_workflow_execution(tmp.path(), "running-archive", live.path(), "running");
    fs::create_dir_all(tmp.path().join("workflow/old-archive")).unwrap();
    fs::write(tmp.path().join("workflow/old-archive/artifact.json"), "{}").unwrap();
    fs::create_dir_all(tmp.path().join("workflow_pending/pending")).unwrap();
    fs::create_dir_all(tmp.path().join("workflow_pending/processed")).unwrap();
    fs::write(
        tmp.path().join("workflow_pending/pending/old-command.json"),
        serde_json::json!({"id": "cmd-1", "execution_id": "old-archive"}).to_string(),
    )
    .unwrap();
    fs::write(
        tmp.path()
            .join("workflow_pending/processed/boundary-command.json"),
        serde_json::json!({"id": "cmd-2", "executionId": "boundary-archive"}).to_string(),
    )
    .unwrap();
    write_archive_index(
        tmp.path(),
        &[
            (
                "old-archive",
                NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
                None,
            ),
            (
                "boundary-archive",
                NOW - RetentionPolicy::default().archived_log_secs as f64,
                None,
            ),
            (
                "restored-archive",
                NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
                Some(NOW - 1.0),
            ),
            (
                "running-archive",
                NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
                None,
            ),
        ],
    );

    run_gc(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(!tmp
        .path()
        .join("workflow_executions/old-archive.json")
        .exists());
    assert!(!tmp.path().join("workflow/old-archive").exists());
    assert!(!tmp
        .path()
        .join("workflow_pending/pending/old-command.json")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_pending/processed/boundary-command.json")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_executions/boundary-archive.json")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_executions/restored-archive.json")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_executions/running-archive.json")
        .exists());
    let archive_index: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(tmp.path().join("workflow_execution_archives.json")).unwrap(),
    )
    .unwrap();
    assert!(archive_index["executions"].get("old-archive").is_none());
    assert!(archive_index["executions"]
        .get("boundary-archive")
        .is_some());
}

#[test]
fn workflow_archive_record_is_kept_when_execution_deletion_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    write_workflow_execution(tmp.path(), "old-archive", live.path(), "completed");
    write_archive_index(
        tmp.path(),
        &[(
            "old-archive",
            NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
            None,
        )],
    );

    let report = run_startup_gc(
        startup_gc_request(
            tmp.path(),
            Some(full_resolution(live_set(&[("live", live.path())]))),
            RuntimeProtection::default(),
            Vec::new(),
            NOW,
        ),
        &FailingRemoveFs {
            failing_path: tmp.path().join("workflow_executions/old-archive.json"),
        },
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert_eq!(report.errors, 1);
    assert!(tmp
        .path()
        .join("workflow_executions/old-archive.json")
        .exists());
    let archive_index: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(tmp.path().join("workflow_execution_archives.json")).unwrap(),
    )
    .unwrap();
    assert!(archive_index["executions"].get("old-archive").is_some());
}

#[test]
fn sweep_skips_workflow_candidate_outside_app_data_dir() {
    let root = tempfile::tempdir().unwrap();
    let app_data_dir = root.path().join("app-data");
    let live = tempfile::tempdir().unwrap();
    let deleted = root.path().join("deleted-worktree");
    fs::create_dir_all(app_data_dir.join("workflow_executions")).unwrap();
    fs::create_dir_all(root.path().join("outside")).unwrap();
    let outside = root.path().join("outside/target.json");
    fs::write(&outside, "outside").unwrap();
    fs::write(
        app_data_dir.join("workflow_executions/malicious.json"),
        serde_json::json!({
            "executionId": "../../outside/target",
            "status": "completed",
            "worktreePath": deleted.to_string_lossy()
        })
        .to_string(),
    )
    .unwrap();
    write_workflow_execution(&app_data_dir, "normal-execution", &deleted, "completed");

    run_gc(
        &app_data_dir,
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(outside.exists());
    assert!(!app_data_dir
        .join("workflow_executions/normal-execution.json")
        .exists());
}

#[test]
fn cache_uses_strict_seven_day_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("lsp/typescript");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("cache.bin"), "cache").unwrap();
    let mtime = modified_secs(&cache).unwrap();

    run_gc(
        tmp.path(),
        None,
        vec![],
        mtime + RetentionPolicy::default().cache_secs as f64,
    );
    assert!(cache.exists());

    run_gc(
        tmp.path(),
        None,
        vec![],
        mtime + RetentionPolicy::default().cache_secs as f64 + 1.0,
    );
    assert!(!cache.exists());
}

#[test]
fn jdtls_workspaces_are_collected_per_workspace_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let old_workspace = tmp.path().join("lsp/jdtls-workspaces/old-workspace");
    let fresh_workspace = tmp.path().join("lsp/jdtls-workspaces/fresh-workspace");
    fs::create_dir_all(&old_workspace).unwrap();
    fs::create_dir_all(&fresh_workspace).unwrap();
    fs::write(old_workspace.join("state"), "old").unwrap();
    fs::write(fresh_workspace.join("state"), "fresh").unwrap();
    let old_mtime = NOW - RetentionPolicy::default().cache_secs as f64 - 1.0;
    set_mtime(&old_workspace.join("state"), old_mtime);
    set_mtime(&old_workspace, old_mtime);
    set_mtime(&fresh_workspace.join("state"), NOW);
    set_mtime(&fresh_workspace, NOW);

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(!old_workspace.exists());
    assert!(fresh_workspace.exists());
    assert!(tmp.path().join("lsp/jdtls-workspaces").exists());
}

#[test]
fn legacy_comments_are_removed_and_review_comments_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    for dir in ["comments", "diff-comments", "threads", "review-comments"] {
        fs::create_dir_all(tmp.path().join(dir)).unwrap();
        fs::write(tmp.path().join(dir).join("entry.json"), "{}").unwrap();
    }

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(!tmp.path().join("comments").exists());
    assert!(!tmp.path().join("diff-comments").exists());
    assert!(!tmp.path().join("threads").exists());
    assert!(tmp.path().join("review-comments").exists());
}

#[test]
fn orphan_blobs_are_removed_without_touching_messages_or_referenced_blobs() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "active-session", live.path(), "active", NOW);
    write_message(
        &session_dir,
        1,
        vec![
            MessagePart::ToolResult {
                content: String::new(),
                is_error: false,
                tool_use_id: None,
                parent_tool_use_id: None,
                content_ref: Some(ToolOutputRef {
                    id: "kept-tool".to_string(),
                    byte_size: 1,
                }),
                summary: None,
            },
            MessagePart::ImageRef {
                attachment: AttachmentRef {
                    id: "kept-attachment".to_string(),
                    media_type: "image/png".to_string(),
                    byte_size: 1,
                },
            },
        ],
    );
    fs::create_dir_all(session_dir.join("tool_outputs")).unwrap();
    fs::create_dir_all(session_dir.join("attachments")).unwrap();
    fs::write(session_dir.join("tool_outputs/kept-tool"), "keep").unwrap();
    fs::write(session_dir.join("tool_outputs/orphan-tool"), "delete").unwrap();
    fs::write(session_dir.join("attachments/kept-attachment"), "keep").unwrap();
    fs::write(session_dir.join("attachments/orphan-attachment"), "delete").unwrap();

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(session_dir.join("messages/1.json").exists());
    assert!(session_dir.join("tool_outputs/kept-tool").exists());
    assert!(!session_dir.join("tool_outputs/orphan-tool").exists());
    assert!(session_dir.join("attachments/kept-attachment").exists());
    assert!(!session_dir.join("attachments/orphan-attachment").exists());
}

#[test]
fn stale_index_refs_do_not_keep_unreferenced_blobs() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "session", live.path(), "idle", NOW);
    fs::write(
        session_dir.join("index.json"),
        serde_json::json!([{
            "id": "stale",
            "seq": 1,
            "role": "agent",
            "timestamp": NOW,
            "contentHash": "stale",
            "toolOutputRefs": [{"id": "stale-index-tool", "byteSize": 1}],
            "attachmentRefs": [{"id": "stale-index-attachment", "mediaType": "image/png", "byteSize": 1}]
        }])
        .to_string(),
    )
    .unwrap();
    write_message(&session_dir, 1, Vec::new());
    fs::create_dir_all(session_dir.join("tool_outputs")).unwrap();
    fs::create_dir_all(session_dir.join("attachments")).unwrap();
    fs::write(session_dir.join("tool_outputs/stale-index-tool"), "delete").unwrap();
    fs::write(
        session_dir.join("attachments/stale-index-attachment"),
        "delete",
    )
    .unwrap();

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(!session_dir.join("tool_outputs/stale-index-tool").exists());
    assert!(!session_dir
        .join("attachments/stale-index-attachment")
        .exists());
}

#[test]
fn missing_messages_dir_is_treated_as_empty_refs_for_blob_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "session", live.path(), "idle", NOW);
    fs::remove_dir_all(session_dir.join("messages")).unwrap();
    fs::create_dir_all(session_dir.join("tool_outputs")).unwrap();
    fs::write(session_dir.join("tool_outputs/orphan-tool"), "delete").unwrap();

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(!session_dir.join("tool_outputs/orphan-tool").exists());
}

#[test]
fn orphan_blob_cleanup_runs_even_when_session_meta_read_model_is_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "session", live.path(), "idle", NOW);
    fs::create_dir_all(session_dir.join("tool_outputs")).unwrap();
    fs::write(session_dir.join("tool_outputs/orphan-tool"), "delete").unwrap();
    let mut request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    request.session_records.clear();

    run_startup_gc(
        request,
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert!(session_dir.exists());
    assert!(!session_dir.join("tool_outputs/orphan-tool").exists());
}

#[test]
fn unreadable_messages_dir_skips_orphan_blob_cleanup_for_session() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "session", live.path(), "idle", NOW);
    fs::create_dir_all(session_dir.join("tool_outputs")).unwrap();
    fs::write(session_dir.join("tool_outputs/orphan-tool"), "keep").unwrap();

    run_startup_gc(
        startup_gc_request(
            tmp.path(),
            None,
            RuntimeProtection::default(),
            Vec::new(),
            NOW,
        ),
        &FailingReadDirFs {
            failing_dir: session_dir.join("messages"),
        },
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert!(session_dir.join("tool_outputs/orphan-tool").exists());
}

#[test]
fn session_meta_read_error_keeps_whole_session_cleanup_from_deleting_it() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = write_session(tmp.path(), "session", live.path(), "idle", NOW);

    let mut request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    request.session_records.clear();
    run_startup_gc(
        request,
        &FailingReadFileFs {
            failing_file: session_dir.join("meta.json"),
        },
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert!(session_dir.exists());
}

#[test]
fn missing_session_meta_deletes_orphan_session_as_unrecoverable() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session_dir = tmp.path().join("sessions/orphan-session");
    fs::create_dir_all(session_dir.join("messages")).unwrap();

    let report = run_gc(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(!session_dir.exists());
    assert_eq!(
        report.categories[&GcCategory::UnrecoverableSession].deleted,
        1
    );
}

#[test]
fn stale_process_records_are_removed_and_live_unknown_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("agent-processes");
    fs::create_dir_all(&dir).unwrap();
    let stale = dir.join("stale.json");
    let live = dir.join("live.json");
    let unknown = dir.join("unknown.json");
    fs::write(&stale, "stale").unwrap();
    fs::write(&live, "live").unwrap();
    fs::write(&unknown, "unknown").unwrap();

    run_gc(
        tmp.path(),
        None,
        vec![
            ProcessRecord {
                path: stale.clone(),
                session_id: Some("s1".to_string()),
                status: ProcessRecordStatus::Stale,
            },
            ProcessRecord {
                path: live.clone(),
                session_id: Some("s2".to_string()),
                status: ProcessRecordStatus::Live,
            },
            ProcessRecord {
                path: unknown.clone(),
                session_id: Some("s3".to_string()),
                status: ProcessRecordStatus::Unknown,
            },
        ],
        NOW,
    );

    assert!(!stale.exists());
    assert!(live.exists());
    assert!(unknown.exists());
}

#[test]
fn workspace_dependent_rules_skip_when_live_worktrees_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing-worktree");
    let archived = write_session(tmp.path(), "archived", &missing, "archived", NOW);
    write_workflow_execution(tmp.path(), "deleted-execution", &missing, "completed");

    run_gc(tmp.path(), None, vec![], NOW);

    assert!(archived.exists());
    assert!(tmp
        .path()
        .join("workflow_executions/deleted-execution.json")
        .exists());
}

#[test]
fn partial_live_worktree_resolution_keeps_only_unresolved_repo_data() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let failed_repo = tmp.path().join("failed-repo");
    fs::create_dir_all(&failed_repo).unwrap();
    let failed_repo_worktree = failed_repo.join("maybe-live-worktree");
    let deleted_worktree = tmp.path().join("deleted-worktree");

    let deleted_session = write_session(
        tmp.path(),
        "deleted-session",
        &deleted_worktree,
        "idle",
        NOW,
    );
    let unresolved_session = write_session(
        tmp.path(),
        "unresolved-session",
        &failed_repo_worktree,
        "idle",
        NOW,
    );
    let expired_live_session = write_session(
        tmp.path(),
        "expired-live-session",
        live.path(),
        "archived",
        NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
    );
    write_workflow_execution(
        tmp.path(),
        "deleted-execution",
        &deleted_worktree,
        "completed",
    );
    write_workflow_execution(
        tmp.path(),
        "unresolved-execution",
        &failed_repo_worktree,
        "completed",
    );

    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    let unresolved_workspace_key =
        workspace_state_storage_key(&failed_repo_worktree.to_string_lossy());
    fs::write(
        tmp.path()
            .join("workspace_state")
            .join(format!("{unresolved_workspace_key}.json")),
        "{}",
    )
    .unwrap();
    fs::write(
        tmp.path().join("workspace_state/deleted-workspace.json"),
        "{}",
    )
    .unwrap();

    let resolution = partial_resolution(
        live_set(&[("live", live.path())]),
        vec![failed_repo.to_string_lossy().into_owned()],
        HashSet::from([workspace_state_storage_key(&failed_repo.to_string_lossy())]),
    );

    run_gc_with_resolution(
        tmp.path(),
        Some(resolution),
        RuntimeProtection::default(),
        NOW,
    );

    assert!(!deleted_session.exists());
    assert!(unresolved_session.exists());
    assert!(!expired_live_session.exists());
    assert!(!tmp
        .path()
        .join("workflow_executions/deleted-execution.json")
        .exists());
    assert!(tmp
        .path()
        .join("workflow_executions/unresolved-execution.json")
        .exists());
    assert!(tmp
        .path()
        .join("workspace_state")
        .join(format!("{unresolved_workspace_key}.json"))
        .exists());
    assert!(tmp
        .path()
        .join("workspace_state/deleted-workspace.json")
        .exists());
}

#[test]
fn unresolved_repo_keeps_basename_workspace_state_key() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let failed_repo = tmp.path().join("failed-repo");
    fs::create_dir_all(&failed_repo).unwrap();
    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    fs::write(
        tmp.path().join("workspace_state/maybe-live-worktree.json"),
        "{}",
    )
    .unwrap();

    let resolution = partial_resolution(
        live_set(&[("live", live.path())]),
        vec![failed_repo.to_string_lossy().into_owned()],
        HashSet::new(),
    );

    run_gc_with_resolution(
        tmp.path(),
        Some(resolution),
        RuntimeProtection::default(),
        NOW,
    );

    assert!(tmp
        .path()
        .join("workspace_state/maybe-live-worktree.json")
        .exists());
}

#[test]
fn workspace_keyed_cleanup_skips_when_runtime_protection_is_incomplete() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    fs::write(tmp.path().join("workspace_state/deleted.json"), "{}").unwrap();
    fs::create_dir_all(tmp.path().join("review-comments")).unwrap();
    fs::write(tmp.path().join("review-comments/deleted.events.json"), "[]").unwrap();

    run_gc_with_resolution(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default().with_workspace_keyed_protection_complete(false),
        NOW,
    );

    assert!(tmp.path().join("workspace_state/deleted.json").exists());
    assert!(tmp
        .path()
        .join("review-comments/deleted.events.json")
        .exists());
}

#[test]
fn checkpoint_cleanup_is_skipped_without_verified_worktree_mapping() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    for dirname in [
        "agent-worktree-checkpoints",
        "agent-worktree-checkpoint-backups",
    ] {
        fs::create_dir_all(tmp.path().join(dirname)).unwrap();
        fs::write(tmp.path().join(dirname).join("deleted.json"), "{}").unwrap();
    }

    run_gc(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        vec![],
        NOW,
    );

    assert!(tmp
        .path()
        .join("agent-worktree-checkpoints/deleted.json")
        .exists());
    assert!(tmp
        .path()
        .join("agent-worktree-checkpoint-backups/deleted.json")
        .exists());
}

#[test]
fn active_session_and_running_workflow_guard_whole_data() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing-worktree");
    let active = write_session(tmp.path(), "active", &missing, "active", NOW);
    let pid_live_archived = write_session(tmp.path(), "pid-live", live.path(), "archived", NOW);
    write_workflow_execution(tmp.path(), "running-execution", &missing, "running");
    let protected = live_set(&[("missing-worktree", &missing)]);
    let protected_workspace_key = workspace_state_storage_key("missing-worktree");
    let protected_review_key = review_comment_storage_key(&missing.to_string_lossy());
    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    fs::write(
        tmp.path()
            .join("workspace_state")
            .join(format!("{protected_workspace_key}.json")),
        "{}",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("review-comments")).unwrap();
    fs::write(
        tmp.path()
            .join("review-comments")
            .join(format!("{protected_review_key}.events.json")),
        "[]",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("agent-worktree-checkpoints")).unwrap();
    fs::write(
        tmp.path()
            .join("agent-worktree-checkpoints")
            .join(format!("{protected_workspace_key}.json")),
        "{}",
    )
    .unwrap();

    run_gc_with_runtime_protection(
        tmp.path(),
        Some(live_set(&[("live", live.path())])),
        RuntimeProtection::new(
            HashSet::from(["active".to_string(), "pid-live".to_string()]),
            [missing.to_string_lossy().into_owned()],
            protected,
        ),
        vec![ProcessRecord {
            path: tmp.path().join("agent-processes/pid-live.json"),
            session_id: Some("pid-live".to_string()),
            status: ProcessRecordStatus::Live,
        }],
        NOW,
    );

    assert!(active.exists());
    assert!(pid_live_archived.exists());
    assert!(tmp
        .path()
        .join("workflow_executions/running-execution.json")
        .exists());
    assert!(tmp
        .path()
        .join("workspace_state")
        .join(format!("{protected_workspace_key}.json"))
        .exists());
    assert!(tmp
        .path()
        .join("review-comments")
        .join(format!("{protected_review_key}.events.json"))
        .exists());
    assert!(tmp
        .path()
        .join("agent-worktree-checkpoints")
        .join(format!("{protected_workspace_key}.json"))
        .exists());
}

#[test]
fn sweep_revalidation_keeps_session_restored_after_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let session = write_session(
        tmp.path(),
        "archived-session",
        live.path(),
        "archived",
        NOW - RetentionPolicy::default().archived_log_secs as f64 - 1.0,
    );
    let request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    write_session(tmp.path(), "archived-session", live.path(), "idle", NOW);

    run_startup_gc(
        request,
        &TestFs,
        &TestArchivePruner,
        &TestRevalidationReader,
    );

    assert!(session.exists());
}

#[test]
fn sweep_revalidation_keeps_candidate_that_enters_runtime_protection() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing-worktree");
    let session = write_session(tmp.path(), "session", &missing, "idle", NOW);
    let request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    let reader = RuntimeProtectionTestReader {
        runtime_protection: RuntimeProtection::new(
            HashSet::from(["session".to_string()]),
            Vec::<String>::new(),
            live_set(&[("missing", &missing)]),
        ),
    };

    run_startup_gc(request, &TestFs, &TestArchivePruner, &reader);

    assert!(session.exists());
}

#[test]
fn sweep_collects_runtime_protection_once_per_run() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let deleted = tmp.path().join("deleted-worktree");
    write_session(tmp.path(), "session", &deleted, "idle", NOW);
    write_workflow_execution(tmp.path(), "workflow-execution", &deleted, "completed");
    fs::create_dir_all(tmp.path().join("workspace_state")).unwrap();
    fs::write(tmp.path().join("workspace_state/deleted.json"), "{}").unwrap();
    fs::create_dir_all(tmp.path().join("review-comments")).unwrap();
    fs::write(tmp.path().join("review-comments/deleted.events.json"), "[]").unwrap();
    let request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    let reader = CountingRuntimeProtectionReader {
        runtime_protection: RuntimeProtection::default(),
        calls: Cell::new(0),
    };

    run_startup_gc(request, &TestFs, &TestArchivePruner, &reader);

    assert_eq!(reader.calls.get(), 1);
}

#[test]
fn workflow_execution_revalidation_uses_single_batch_read() {
    let tmp = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    let deleted = tmp.path().join("deleted-worktree");
    write_workflow_execution(tmp.path(), "deleted-execution-a", &deleted, "completed");
    write_workflow_execution(tmp.path(), "deleted-execution-b", &deleted, "completed");
    let request = startup_gc_request(
        tmp.path(),
        Some(full_resolution(live_set(&[("live", live.path())]))),
        RuntimeProtection::default(),
        Vec::new(),
        NOW,
    );
    let reader = CountingWorkflowRevalidationReader::default();

    run_startup_gc(request, &TestFs, &TestArchivePruner, &reader);

    assert_eq!(reader.batch_calls.get(), 1);
    assert_eq!(reader.single_calls.get(), 0);
}

struct RuntimeProtectionTestReader {
    runtime_protection: RuntimeProtection,
}

impl GcRevalidationReader for RuntimeProtectionTestReader {
    fn runtime_protection(
        &self,
        _app_data_dir: &std::path::Path,
        _process_records: &[ProcessRecord],
    ) -> RuntimeProtection {
        self.runtime_protection.clone()
    }

    fn session_state(
        &self,
        app_data_dir: &std::path::Path,
        session_id: &str,
    ) -> RevalidationRead<CurrentSessionState> {
        TestRevalidationReader.session_state(app_data_dir, session_id)
    }

    fn workflow_execution_state(
        &self,
        app_data_dir: &std::path::Path,
        execution_id: &str,
    ) -> RevalidationRead<CurrentWorkflowExecutionState> {
        TestRevalidationReader.workflow_execution_state(app_data_dir, execution_id)
    }
}

struct CountingRuntimeProtectionReader {
    runtime_protection: RuntimeProtection,
    calls: Cell<usize>,
}

impl GcRevalidationReader for CountingRuntimeProtectionReader {
    fn runtime_protection(
        &self,
        _app_data_dir: &std::path::Path,
        _process_records: &[ProcessRecord],
    ) -> RuntimeProtection {
        self.calls.set(self.calls.get() + 1);
        self.runtime_protection.clone()
    }

    fn session_state(
        &self,
        app_data_dir: &std::path::Path,
        session_id: &str,
    ) -> RevalidationRead<CurrentSessionState> {
        TestRevalidationReader.session_state(app_data_dir, session_id)
    }

    fn workflow_execution_state(
        &self,
        app_data_dir: &std::path::Path,
        execution_id: &str,
    ) -> RevalidationRead<CurrentWorkflowExecutionState> {
        TestRevalidationReader.workflow_execution_state(app_data_dir, execution_id)
    }
}

#[derive(Default)]
struct CountingWorkflowRevalidationReader {
    batch_calls: Cell<usize>,
    single_calls: Cell<usize>,
}

impl GcRevalidationReader for CountingWorkflowRevalidationReader {
    fn runtime_protection(
        &self,
        _app_data_dir: &std::path::Path,
        _process_records: &[ProcessRecord],
    ) -> RuntimeProtection {
        RuntimeProtection::default()
    }

    fn session_state(
        &self,
        app_data_dir: &std::path::Path,
        session_id: &str,
    ) -> RevalidationRead<CurrentSessionState> {
        TestRevalidationReader.session_state(app_data_dir, session_id)
    }

    fn workflow_execution_state(
        &self,
        app_data_dir: &std::path::Path,
        execution_id: &str,
    ) -> RevalidationRead<CurrentWorkflowExecutionState> {
        self.single_calls.set(self.single_calls.get() + 1);
        TestRevalidationReader.workflow_execution_state(app_data_dir, execution_id)
    }

    fn workflow_execution_states(
        &self,
        app_data_dir: &std::path::Path,
        execution_ids: &HashSet<String>,
    ) -> HashMap<String, RevalidationRead<CurrentWorkflowExecutionState>> {
        self.batch_calls.set(self.batch_calls.get() + 1);
        TestRevalidationReader.workflow_execution_states(app_data_dir, execution_ids)
    }
}

#[test]
fn report_counts_deleted_entries_and_reclaimed_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("comments")).unwrap();
    fs::write(tmp.path().join("comments/a.json"), "12345").unwrap();

    let report = run_gc(tmp.path(), None, vec![], NOW);

    assert_eq!(report.total_files, 1);
    assert_eq!(report.total_bytes, 5);
    assert_eq!(report.errors, 0);
    assert_eq!(
        report.categories[&GcCategory::LegacyComments].reclaimed_bytes,
        5
    );
}

#[test]
fn fixture_normalized_worktree_path_trims_trailing_separators() {
    let path = format!("{}///", std::env::temp_dir().display());
    assert_eq!(
        normalized_worktree_path(&path),
        std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
    );
}

#[test]
fn latest_mtime_uses_system_time() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("file"), "x").unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert!(modified_secs(&tmp.path().join("file")).unwrap() <= now + 1.0);
}
