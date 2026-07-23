use std::collections::{BTreeMap, HashSet};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use crate::domain::app_data_gc::{is_expired, GcCategory, RetentionPolicy};
#[cfg(test)]
use crate::usecase::agent_session::session::{ChatMessage, MessagePart, SessionState};

#[cfg(test)]
use super::ports::GcFileSystem;
use super::request::{
    CacheGcRecord, LiveWorktreeResolution, LiveWorktreeSet, ProcessRecord, ProcessRecordStatus,
    ReviewCommentGcRecord, RuntimeProtection, WorkspaceStateGcRecord,
};
#[cfg(test)]
use super::request::{SessionBlobStore, SessionGcRecord, WorkflowExecutionGcRecord};

#[cfg(test)]
pub(super) struct SessionDeletionContext<'a> {
    pub(super) live_worktrees: &'a LiveWorktreeResolution,
    pub(super) session_records: &'a [SessionGcRecord],
    pub(super) active_session_ids: &'a HashSet<String>,
    pub(super) running_worktrees: &'a HashSet<String>,
    pub(super) now_secs: f64,
    pub(super) retention: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeletionRevalidation {
    None,
    #[cfg(test)]
    Session {
        session_id: String,
    },
    #[cfg(test)]
    WorkflowExecutionMetadata {
        execution_id: String,
    },
    WorkspaceState {
        key: String,
    },
    ReviewComment {
        key: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct DeletionCandidate {
    pub(super) path: PathBuf,
    pub(super) category: GcCategory,
    pub(super) workflow_execution_id: Option<String>,
    pub(super) revalidation: DeletionRevalidation,
}

#[derive(Debug, Default)]
pub(super) struct DeletionPlan {
    pub(super) app_data_dir: PathBuf,
    pub(super) candidates: Vec<DeletionCandidate>,
    paths: HashSet<PathBuf>,
    pub(super) workflow_archive_records: BTreeMap<GcCategory, HashSet<String>>,
}

impl DeletionPlan {
    pub(super) fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            candidates: Vec::new(),
            paths: HashSet::new(),
            workflow_archive_records: BTreeMap::new(),
        }
    }

    fn add(&mut self, path: PathBuf, category: GcCategory) {
        self.add_with_revalidation(path, category, DeletionRevalidation::None);
    }

    fn add_with_revalidation(
        &mut self,
        path: PathBuf,
        category: GcCategory,
        revalidation: DeletionRevalidation,
    ) {
        if self.paths.insert(path.clone()) {
            self.candidates.push(DeletionCandidate {
                path,
                category,
                workflow_execution_id: None,
                revalidation,
            });
        }
    }

    #[cfg(test)]
    fn add_workflow_path(&mut self, path: PathBuf, category: GcCategory, execution_id: &str) {
        if self.paths.insert(path.clone()) {
            self.candidates.push(DeletionCandidate {
                path,
                category,
                workflow_execution_id: Some(execution_id.to_string()),
                revalidation: DeletionRevalidation::WorkflowExecutionMetadata {
                    execution_id: execution_id.to_string(),
                },
            });
        }
    }

    #[cfg(test)]
    fn add_workflow_archive_record(&mut self, execution_id: String, category: GcCategory) {
        self.workflow_archive_records
            .entry(category)
            .or_default()
            .insert(execution_id);
    }
}

#[cfg(test)]
pub(super) fn collect_session_deletions(
    context: &SessionDeletionContext<'_>,
    plan: &mut DeletionPlan,
) {
    for record in context.session_records {
        if context.active_session_ids.contains(&record.id) {
            continue;
        }
        let Some(worktree_path) = record.worktree_path.as_ref() else {
            add_session_delete(plan, record, GcCategory::UnrecoverableSession);
            continue;
        };
        if worktree_path.is_unresolved() {
            continue;
        }
        if context.running_worktrees.contains(worktree_path.key()) {
            continue;
        }
        if !context.live_worktrees.contains_worktree_path(worktree_path) {
            if context
                .live_worktrees
                .worktree_path_may_be_unresolved(worktree_path)
            {
                continue;
            }
            add_session_delete(plan, record, GcCategory::DeletedWorkspace);
            continue;
        }
        if session_is_recoverable_expired(record.state.as_ref(), record.updated_at, context) {
            add_session_delete(plan, record, GcCategory::RecoverableExpired);
        }
    }
}

#[cfg(test)]
fn session_is_recoverable_expired(
    state: Option<&SessionState>,
    updated_at: Option<f64>,
    context: &SessionDeletionContext<'_>,
) -> bool {
    matches!(state, Some(SessionState::Archived | SessionState::Closed))
        && updated_at.is_some_and(|updated_at| {
            is_expired(
                context.now_secs,
                updated_at,
                context.retention.archived_log_secs,
            )
        })
}

#[cfg(test)]
fn add_session_delete(plan: &mut DeletionPlan, record: &SessionGcRecord, category: GcCategory) {
    for path in &record.delete_paths {
        plan.add_with_revalidation(
            path.clone(),
            category,
            DeletionRevalidation::Session {
                session_id: record.id.clone(),
            },
        );
    }
}

#[cfg(test)]
pub(super) fn collect_workflow_deletions(
    workflow_executions: &[WorkflowExecutionGcRecord],
    live_worktrees: &LiveWorktreeResolution,
    now_secs: f64,
    retention: RetentionPolicy,
    plan: &mut DeletionPlan,
) {
    for execution in workflow_executions {
        if !execution.is_terminal || execution.worktree_path.is_unresolved() {
            continue;
        }
        if !live_worktrees.contains_worktree_path(&execution.worktree_path) {
            if live_worktrees.worktree_path_may_be_unresolved(&execution.worktree_path) {
                continue;
            }
            add_workflow_execution_delete(execution, GcCategory::DeletedWorkspace, plan);
            continue;
        }
        if execution.manual_archived_at.is_some_and(|archived_at| {
            is_expired(now_secs, archived_at, retention.archived_log_secs)
        }) {
            add_workflow_execution_delete(execution, GcCategory::RecoverableExpired, plan);
        }
    }
}

#[cfg(test)]
fn add_workflow_execution_delete(
    execution: &WorkflowExecutionGcRecord,
    category: GcCategory,
    plan: &mut DeletionPlan,
) {
    for path in &execution.delete_paths {
        plan.add_workflow_path(path.clone(), category, &execution.execution_id);
    }
    plan.add_workflow_archive_record(execution.execution_id.clone(), category);
}

pub(super) fn collect_workspace_keyed_deletions(
    workspace_state_records: &[WorkspaceStateGcRecord],
    review_comment_records: &[ReviewCommentGcRecord],
    checkpoint_paths: &[PathBuf],
    live_worktrees: &LiveWorktreeResolution,
    runtime_protection: &RuntimeProtection,
    plan: &mut DeletionPlan,
) {
    if !runtime_protection.workspace_keyed_protection_complete {
        log::info!(
            "app data gc skipped workspace-keyed cleanup because runtime protection metadata was incomplete"
        );
        return;
    }
    let protected_worktrees = &runtime_protection.protected_worktrees;
    collect_workspace_state_deletions(
        workspace_state_records,
        live_worktrees,
        protected_worktrees,
        plan,
    );
    collect_review_comment_deletions(
        review_comment_records,
        live_worktrees,
        protected_worktrees,
        plan,
    );
    skip_checkpoint_deletions(checkpoint_paths);
}

fn collect_workspace_state_deletions(
    records: &[WorkspaceStateGcRecord],
    live_worktrees: &LiveWorktreeResolution,
    protected_worktrees: &LiveWorktreeSet,
    plan: &mut DeletionPlan,
) {
    if live_worktrees.has_unresolved_repos() {
        log::info!(
            "app data gc skipped workspace-state cleanup because some repo worktrees were unavailable"
        );
        return;
    }
    for record in records {
        if !live_worktrees.contains_workspace_state_key(&record.key)
            && !protected_worktrees.contains_workspace_state_key(&record.key)
            && !live_worktrees.workspace_state_key_may_be_unresolved(&record.key)
        {
            plan.add_with_revalidation(
                record.path.clone(),
                GcCategory::DeletedWorkspace,
                DeletionRevalidation::WorkspaceState {
                    key: record.key.clone(),
                },
            );
        }
    }
}

fn collect_review_comment_deletions(
    records: &[ReviewCommentGcRecord],
    live_worktrees: &LiveWorktreeResolution,
    protected_worktrees: &LiveWorktreeSet,
    plan: &mut DeletionPlan,
) {
    if live_worktrees.has_unresolved_repos() {
        log::info!(
            "app data gc skipped review-comment workspace cleanup because some repo worktrees were unavailable"
        );
        return;
    }
    for record in records {
        if !live_worktrees.contains_review_comment_key(&record.key)
            && !protected_worktrees.contains_review_comment_key(&record.key)
        {
            plan.add_with_revalidation(
                record.path.clone(),
                GcCategory::DeletedWorkspace,
                DeletionRevalidation::ReviewComment {
                    key: record.key.clone(),
                },
            );
        }
    }
}

fn skip_checkpoint_deletions(checkpoint_paths: &[PathBuf]) {
    for path in checkpoint_paths {
        log::info!(
            "app data gc skipped checkpoint cleanup for {} because checkpoint worktree mapping is not verified",
            path.display()
        );
    }
}

pub(super) fn collect_cache_deletions(
    cache_records: &[CacheGcRecord],
    now_secs: f64,
    retention: RetentionPolicy,
    plan: &mut DeletionPlan,
) {
    for record in cache_records {
        if is_expired(now_secs, record.updated_at, retention.cache_secs) {
            plan.add(record.path.clone(), GcCategory::RegenerableCache);
        }
    }
}

pub(super) fn collect_legacy_comment_deletions(
    legacy_comment_paths: &[PathBuf],
    plan: &mut DeletionPlan,
) {
    for path in legacy_comment_paths {
        plan.add(path.clone(), GcCategory::LegacyComments);
    }
}

#[cfg(test)]
pub(super) fn collect_orphan_blob_deletions(
    session_blob_stores: &[SessionBlobStore],
    fs: &dyn GcFileSystem,
    plan: &mut DeletionPlan,
) {
    for store in session_blob_stores {
        let Some((tool_output_refs, attachment_refs)) =
            read_message_blob_refs(&store.messages_dir, fs)
        else {
            continue;
        };
        collect_unreferenced_files(&store.tool_outputs_dir, &tool_output_refs, fs, plan);
        collect_unreferenced_files(&store.attachments_dir, &attachment_refs, fs, plan);
    }
}

#[cfg(test)]
fn collect_unreferenced_files(
    dir: &Path,
    referenced: &HashSet<String>,
    fs: &dyn GcFileSystem,
    plan: &mut DeletionPlan,
) {
    let Ok(entries) = fs.read_dir(dir) else {
        return;
    };
    for path in entries {
        if !fs.is_file(&path) {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !referenced.contains(id) {
            plan.add(path, GcCategory::OrphanBlob);
        }
    }
}

pub(super) fn collect_stale_process_deletions(
    process_records: &[ProcessRecord],
    plan: &mut DeletionPlan,
) {
    for record in process_records {
        if record.status == ProcessRecordStatus::Stale {
            plan.add(record.path.clone(), GcCategory::StaleProcessRecord);
        }
    }
}

#[cfg(test)]
fn read_message_blob_refs(
    messages_dir: &Path,
    fs: &dyn GcFileSystem,
) -> Option<(HashSet<String>, HashSet<String>)> {
    let mut tool_outputs = HashSet::new();
    let mut attachments = HashSet::new();
    let entries = match fs.read_dir(messages_dir) {
        Ok(entries) => entries,
        Err(error) if error.is_not_found() => return Some((tool_outputs, attachments)),
        Err(error) => {
            log::warn!(
                "app data gc skipped orphan blob cleanup for unreadable messages dir {}: {error}",
                messages_dir.display()
            );
            return None;
        }
    };
    for path in entries {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let content = match fs.read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                log::warn!(
                    "app data gc skipped orphan blob cleanup for unreadable message {}: {error}",
                    path.display()
                );
                return None;
            }
        };
        let message: ChatMessage = match crate::adaptor::gateway::agent_session::session_storage::decode_legacy_chat_message_for_gc(
            content.as_bytes(),
            path.to_string_lossy().into_owned(),
        ) {
            Ok(message) => message,
            Err(error) => {
                log::warn!(
                    "app data gc skipped orphan blob cleanup for invalid message {}: {error}",
                    path.display()
                );
                return None;
            }
        };
        collect_blob_refs_from_message(&message, &mut tool_outputs, &mut attachments);
    }
    Some((tool_outputs, attachments))
}

#[cfg(test)]
fn collect_blob_refs_from_message(
    message: &ChatMessage,
    tool_outputs: &mut HashSet<String>,
    attachments: &mut HashSet<String>,
) {
    for part in message.parts.as_deref().unwrap_or_default() {
        match part {
            MessagePart::ToolResult {
                content_ref: Some(content_ref),
                ..
            } => {
                tool_outputs.insert(content_ref.id.clone());
            }
            MessagePart::ImageRef { attachment } => {
                attachments.insert(attachment.id.clone());
            }
            _ => {}
        }
    }
}
