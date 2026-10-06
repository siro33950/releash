use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::comment::{
    ReviewActor, ReviewActorKind, ReviewError, ReviewEvent, ReviewTarget,
};
use crate::infrastructure::platform::file_replace;
use crate::usecase::comment::{
    ReviewClock, ReviewEventMutation, ReviewEventStore, ReviewIdGenerator,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredReviewActorKind {
    Human,
    Agent,
}

impl From<&ReviewActorKind> for StoredReviewActorKind {
    fn from(kind: &ReviewActorKind) -> Self {
        match kind {
            ReviewActorKind::Human => Self::Human,
            ReviewActorKind::Agent => Self::Agent,
        }
    }
}

impl From<StoredReviewActorKind> for ReviewActorKind {
    fn from(kind: StoredReviewActorKind) -> Self {
        match kind {
            StoredReviewActorKind::Human => Self::Human,
            StoredReviewActorKind::Agent => Self::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredReviewActor {
    kind: StoredReviewActorKind,
    backend_id: Option<String>,
    model: Option<String>,
    session_id: Option<String>,
    display_name: String,
}

impl From<&ReviewActor> for StoredReviewActor {
    fn from(actor: &ReviewActor) -> Self {
        Self {
            kind: StoredReviewActorKind::from(&actor.kind),
            backend_id: actor.backend_id.clone(),
            model: actor.model.clone(),
            session_id: actor.session_id.clone(),
            display_name: actor.display_name.clone(),
        }
    }
}

impl From<StoredReviewActor> for ReviewActor {
    fn from(actor: StoredReviewActor) -> Self {
        Self {
            kind: actor.kind.into(),
            backend_id: actor.backend_id,
            model: actor.model,
            session_id: actor.session_id,
            display_name: actor.display_name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredReviewTarget {
    file_path: Option<String>,
    line_number: Option<u32>,
    end_line: Option<u32>,
}

impl From<&ReviewTarget> for StoredReviewTarget {
    fn from(target: &ReviewTarget) -> Self {
        Self {
            file_path: target.file_path.clone(),
            line_number: target.line_number,
            end_line: target.end_line,
        }
    }
}

impl From<StoredReviewTarget> for ReviewTarget {
    fn from(target: StoredReviewTarget) -> Self {
        Self {
            file_path: target.file_path,
            line_number: target.line_number,
            end_line: target.end_line,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "eventType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum StoredReviewEvent {
    ThreadCreated {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: StoredReviewActor,
        target: StoredReviewTarget,
        content: String,
        at: f64,
    },
    CommentAppended {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: StoredReviewActor,
        content: String,
        at: f64,
    },
    ThreadResolved {
        event_id: String,
        thread_id: String,
        actor: StoredReviewActor,
        outcome: String,
        summary: String,
        at: f64,
    },
    ThreadDeleted {
        event_id: String,
        thread_id: String,
        actor: StoredReviewActor,
        at: f64,
    },
}

impl StoredReviewEvent {
    fn thread_id(&self) -> &str {
        match self {
            Self::ThreadCreated { thread_id, .. }
            | Self::CommentAppended { thread_id, .. }
            | Self::ThreadResolved { thread_id, .. }
            | Self::ThreadDeleted { thread_id, .. } => thread_id,
        }
    }
}

impl From<&ReviewEvent> for StoredReviewEvent {
    fn from(event: &ReviewEvent) -> Self {
        match event {
            ReviewEvent::ThreadCreated {
                event_id,
                thread_id,
                comment_id,
                actor,
                target,
                content,
                at,
            } => Self::ThreadCreated {
                event_id: event_id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: StoredReviewActor::from(actor),
                target: StoredReviewTarget::from(target),
                content: content.clone(),
                at: *at,
            },
            ReviewEvent::CommentAppended {
                event_id,
                thread_id,
                comment_id,
                actor,
                content,
                at,
            } => Self::CommentAppended {
                event_id: event_id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: StoredReviewActor::from(actor),
                content: content.clone(),
                at: *at,
            },
            ReviewEvent::ThreadResolved {
                event_id,
                thread_id,
                actor,
                outcome,
                summary,
                at,
            } => Self::ThreadResolved {
                event_id: event_id.clone(),
                thread_id: thread_id.clone(),
                actor: StoredReviewActor::from(actor),
                outcome: outcome.clone(),
                summary: summary.clone(),
                at: *at,
            },
            ReviewEvent::ThreadDeleted {
                event_id,
                thread_id,
                actor,
                at,
            } => Self::ThreadDeleted {
                event_id: event_id.clone(),
                thread_id: thread_id.clone(),
                actor: StoredReviewActor::from(actor),
                at: *at,
            },
        }
    }
}

impl From<StoredReviewEvent> for ReviewEvent {
    fn from(event: StoredReviewEvent) -> Self {
        match event {
            StoredReviewEvent::ThreadCreated {
                event_id,
                thread_id,
                comment_id,
                actor,
                target,
                content,
                at,
            } => Self::ThreadCreated {
                event_id,
                thread_id,
                comment_id,
                actor: actor.into(),
                target: target.into(),
                content,
                at,
            },
            StoredReviewEvent::CommentAppended {
                event_id,
                thread_id,
                comment_id,
                actor,
                content,
                at,
            } => Self::CommentAppended {
                event_id,
                thread_id,
                comment_id,
                actor: actor.into(),
                content,
                at,
            },
            StoredReviewEvent::ThreadResolved {
                event_id,
                thread_id,
                actor,
                outcome,
                summary,
                at,
            } => Self::ThreadResolved {
                event_id,
                thread_id,
                actor: actor.into(),
                outcome,
                summary,
                at,
            },
            StoredReviewEvent::ThreadDeleted {
                event_id,
                thread_id,
                actor,
                at,
            } => Self::ThreadDeleted {
                event_id,
                thread_id,
                actor: actor.into(),
                at,
            },
        }
    }
}

fn io_error(error: std::io::Error) -> ReviewError {
    ReviewError::Io(error.to_string())
}

fn serialize_error(error: serde_json::Error) -> ReviewError {
    ReviewError::Serialize(error.to_string())
}

fn validate_stored_thread_id(thread_id: &str) -> Result<(), ReviewError> {
    Uuid::parse_str(thread_id)
        .map(|_| ())
        .map_err(|e| ReviewError::Serialize(format!("invalid stored review threadId: {e}")))
}

pub struct SystemReviewClock;

impl ReviewClock for SystemReviewClock {
    fn now(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

pub struct UuidReviewIdGenerator;

impl ReviewIdGenerator for UuidReviewIdGenerator {
    fn event_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

pub struct FileReviewEventStore {
    /// Reads rely on atomic rename and stay lock-free; writes take this lock for in-process exclusion.
    file_lock: Mutex<()>,
}

impl Default for FileReviewEventStore {
    fn default() -> Self {
        Self {
            file_lock: Mutex::new(()),
        }
    }
}

impl ReviewEventStore for FileReviewEventStore {
    fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        self.load_events(app_data_dir, worktree_name)
    }

    fn mutate(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        mutation: ReviewEventMutation<'_>,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        use crate::domain::repository::worktree_operation::WorktreeOperationLocks;
        let _mutation =
            super::repository::worktree_operation::FileWorktreeOperationLocks::new(app_data_dir)
                .mutation(worktree_name)
                .map_err(|error| match error {
                    crate::domain::repository::RepositoryError::Technical(error) => {
                        ReviewError::Technical(error)
                    }
                    crate::domain::repository::RepositoryError::Rule(message) => {
                        ReviewError::PermissionDenied(message)
                    }
                    crate::domain::repository::RepositoryError::External(message) => {
                        ReviewError::Io(message)
                    }
                })?;
        let _guard = crate::common::operation_context::poll(|| {
            self.file_lock.try_lock_for(Duration::from_millis(10))
        })?;
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load_events(app_data_dir, worktree_name)?;
        let appended = crate::common::operation_context::before(|| mutation(&events))??;
        events.extend(appended);
        crate::common::operation_context::before(|| {
            self.write_events(app_data_dir, worktree_name, &events)
        })??;
        Ok(events)
    }
}

impl FileReviewEventStore {
    fn load_events(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        let file_path = state_file(app_data_dir, worktree_name);
        let events = if file_path.exists() {
            let data = std::fs::read_to_string(&file_path).map_err(io_error)?;
            serde_json::from_str::<Vec<StoredReviewEvent>>(&data)
                .map_err(serialize_error)?
                .into_iter()
                .map(|event| {
                    validate_stored_thread_id(event.thread_id())?;
                    Ok(ReviewEvent::from(event))
                })
                .collect::<Result<Vec<_>, ReviewError>>()?
        } else {
            Vec::new()
        };
        Ok(events)
    }

    fn write_events(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        events: &[ReviewEvent],
    ) -> Result<(), ReviewError> {
        let dir = state_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let file_path = state_file(app_data_dir, worktree_name);
        let tmp_path = file_path.with_extension(format!("events.{}.tmp", Uuid::new_v4()));
        let stored: Vec<_> = events.iter().map(StoredReviewEvent::from).collect();
        let json = serde_json::to_string_pretty(&stored).map_err(serialize_error)?;
        {
            let mut file = File::create(&tmp_path).map_err(io_error)?;
            file.write_all(json.as_bytes()).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
        }
        file_replace::replace_file(&tmp_path, &file_path).map_err(io_error)?;
        Ok(())
    }
}

pub fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("review-comments")
}

pub fn worktree_storage_key(worktree: &str) -> String {
    let trimmed = worktree.trim();
    let canonical = Path::new(trimmed)
        .canonicalize()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| trimmed.to_string());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let label = Path::new(&canonical)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("worktree")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{label}-{}", &digest[..24])
}

pub fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.events.json"))
}

pub fn lock_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.events.lock"))
}

pub struct WorktreeFileLock {
    _file: File,
}

pub fn acquire_worktree_file_lock(
    app_data_dir: &Path,
    worktree_name: &str,
) -> Result<WorktreeFileLock, ReviewError> {
    let dir = state_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    let path = lock_file(app_data_dir, worktree_name);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(io_error)?;
    writeln!(file, "pid={}", std::process::id()).map_err(io_error)?;
    file.flush().map_err(io_error)?;
    crate::common::operation_context::timeout_sync(Duration::from_secs(10), || {
        crate::infrastructure::file_lock::exclusive(&file)
    })
    .map_err(|error| match error {
        crate::infrastructure::file_lock::LockError::Io(error) => io_error(error),
        crate::infrastructure::file_lock::LockError::Stopped(error) => error.into(),
    })?;
    Ok(WorktreeFileLock { _file: file })
}

#[cfg(feature = "test-support")]
impl FileReviewEventStore {
    pub fn test_file_lock(&self) -> &Mutex<()> {
        &self.file_lock
    }
}
