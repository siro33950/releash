use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::comment::{
    ReviewActor, ReviewActorKind, ReviewError, ReviewEvent, ReviewTarget,
};
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

pub(crate) struct SystemReviewClock;

impl ReviewClock for SystemReviewClock {
    fn now(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

pub(crate) struct UuidReviewIdGenerator;

impl ReviewIdGenerator for UuidReviewIdGenerator {
    fn event_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

pub(crate) struct FileReviewEventStore {
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
        let _guard = self.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load_events(app_data_dir, worktree_name)?;
        let appended = mutation(&events)?;
        events.extend(appended);
        self.write_events(app_data_dir, worktree_name, &events)?;
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
        replace_file(&tmp_path, &file_path)?;
        Ok(())
    }
}

pub(crate) fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("review-comments")
}

pub(crate) fn worktree_storage_key(worktree: &str) -> String {
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

pub(crate) fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.events.json"))
}

fn lock_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.events.lock"))
}

struct WorktreeFileLock {
    _file: File,
}

fn acquire_worktree_file_lock(
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
    let start = Instant::now();
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(WorktreeFileLock { _file: file }),
            Err(e)
                if e.kind() == ErrorKind::WouldBlock
                    && start.elapsed() < Duration::from_secs(10) =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(ReviewError::Io(e.to_string())),
        }
    }
}

fn replace_file(tmp_path: &Path, file_path: &Path) -> Result<(), ReviewError> {
    #[cfg(windows)]
    {
        replace_file_windows(tmp_path, file_path)
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(tmp_path, file_path).map_err(io_error)
    }
}

#[cfg(windows)]
fn replace_file_windows(tmp_path: &Path, file_path: &Path) -> Result<(), ReviewError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let tmp = wide(tmp_path);
    let dest = wide(file_path);
    let ok = if file_path.exists() {
        unsafe {
            ReplaceFileW(
                dest.as_ptr(),
                tmp.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        }
    } else {
        unsafe {
            MoveFileExW(
                tmp.as_ptr(),
                dest.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if ok == 0 {
        return Err(ReviewError::Io(std::io::Error::last_os_error().to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::comment::{
        ReviewActor, ReviewTarget, ReviewThreadState, MAX_REVIEW_TEXT_BYTES,
    };
    use crate::usecase::comment::ReviewCommentUsecase;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    };
    use tempfile::TempDir;

    fn usecase(store: Arc<FileReviewEventStore>) -> ReviewCommentUsecase {
        ReviewCommentUsecase::new(
            store,
            Arc::new(SystemReviewClock),
            Arc::new(UuidReviewIdGenerator),
        )
    }

    fn target() -> ReviewTarget {
        ReviewTarget {
            file_path: None,
            line_number: None,
            end_line: None,
        }
    }

    fn agent(session_id: &str) -> ReviewActor {
        ReviewActor::agent(
            "codex".to_string(),
            "gpt-5".to_string(),
            Some(session_id.to_string()),
        )
    }

    #[test]
    fn missing_state_file_loads_as_empty_list() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let threads = usecase(store)
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();

        assert!(threads.is_empty());
    }

    #[test]
    fn load_propagates_parse_error_without_overwriting_file() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{not-json").unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            "A".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::Serialize(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not-json");
    }

    #[test]
    fn load_rejects_non_uuid_thread_id_before_projection() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[
  {
    "eventType": "thread_created",
    "eventId": "event-1",
    "threadId": "thread-1",
    "commentId": "comment-1",
    "actor": {
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": "legacy-session",
      "displayName": "codex/gpt-5"
    },
    "target": {
      "filePath": null,
      "lineNumber": null,
      "endLine": null
    },
    "content": "Claim",
    "at": 1.0
  }
]"#,
        )
        .unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).build_handoff(dir.path(), "wt", "thread-1", "releash");

        assert!(matches!(result, Err(ReviewError::Serialize(_))));
    }

    #[test]
    fn same_basename_worktrees_use_distinct_storage_keys() {
        let dir = TempDir::new().unwrap();
        let parent_a = TempDir::new().unwrap();
        let parent_b = TempDir::new().unwrap();
        let wt_a = parent_a.path().join("repo");
        let wt_b = parent_b.path().join("repo");
        std::fs::create_dir(&wt_a).unwrap();
        std::fs::create_dir(&wt_b).unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let usecase = usecase(store);

        usecase
            .create_thread(
                dir.path(),
                &wt_a.to_string_lossy(),
                ReviewActor::human(),
                target(),
                "A".to_string(),
            )
            .unwrap();

        assert!(usecase
            .list_threads(
                dir.path(),
                &wt_b.to_string_lossy(),
                None,
                ReviewActor::human()
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    fn existing_lock_file_is_reused_without_ttl_steal() {
        let dir = TempDir::new().unwrap();
        let lock = lock_file(dir.path(), "wt");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();

        let guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();
        assert!(lock.exists());
        drop(guard);
        assert!(lock.exists());
    }

    #[test]
    fn multi_store_writes_keep_all_comments_and_resolve_once() {
        let dir = TempDir::new().unwrap();
        let first_store = Arc::new(FileReviewEventStore::default());
        let first_usecase = usecase(first_store);
        let thread = first_usecase
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();

        let mut handles = Vec::new();
        for content in ["A", "B"] {
            let path = dir.path().to_path_buf();
            let thread_id = thread.id.clone();
            handles.push(std::thread::spawn(move || {
                let store = Arc::new(FileReviewEventStore::default());
                usecase(store)
                    .append_comment(&path, "wt", agent(content), &thread_id, content.to_string())
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let current = first_usecase
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();
        assert_eq!(current.comments.len(), 3);

        let resolved = usecase(Arc::new(FileReviewEventStore::default()))
            .resolve_thread(
                dir.path(),
                "wt",
                agent("s2"),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        assert_eq!(resolved.state, ReviewThreadState::Resolved);

        let second = usecase(Arc::new(FileReviewEventStore::default())).resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "again".to_string(),
        );
        assert!(matches!(second, Err(ReviewError::AlreadyResolved(_))));
    }

    #[test]
    fn lockless_reads_do_not_observe_torn_json_during_concurrent_writes() {
        let dir = TempDir::new().unwrap();
        let initial_usecase = usecase(Arc::new(FileReviewEventStore::default()));
        let thread = initial_usecase
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let app_data_dir = dir.path().to_path_buf();
        let thread_id = thread.id.clone();
        let start = Arc::new(Barrier::new(2));
        let done = Arc::new(AtomicBool::new(false));
        let writer_start = Arc::clone(&start);
        let writer_done = Arc::clone(&done);
        let writer = std::thread::spawn(move || {
            writer_start.wait();
            for index in 0..30 {
                usecase(Arc::new(FileReviewEventStore::default()))
                    .append_comment(
                        &app_data_dir,
                        "wt",
                        agent(&format!("writer-{index}")),
                        &thread_id,
                        format!("comment-{index}"),
                    )
                    .unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
            writer_done.store(true, Ordering::Release);
        });

        let reader_usecase = usecase(Arc::new(FileReviewEventStore::default()));
        let mut last_list_count = 1;
        let mut last_get_count = 1;
        let mut observed_concurrent_write = false;
        start.wait();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done.load(Ordering::Acquire) && Instant::now() < deadline {
            let threads = reader_usecase
                .list_threads(dir.path(), "wt", None, ReviewActor::human())
                .unwrap();
            assert_eq!(threads.len(), 1);
            let list_count = threads[0].comments.len();
            assert!(
                list_count >= last_list_count,
                "list observed comment count rollback: {last_list_count} -> {list_count}"
            );
            last_list_count = list_count;

            let got = reader_usecase
                .get_thread(dir.path(), "wt", &thread.id)
                .unwrap();
            let get_count = got.comments.len();
            assert!(
                get_count >= last_get_count,
                "get observed comment count rollback: {last_get_count} -> {get_count}"
            );
            if get_count > 1 {
                observed_concurrent_write = true;
            }
            last_get_count = get_count;
            std::thread::yield_now();
        }

        writer.join().unwrap();
        assert!(observed_concurrent_write);
        let final_thread = reader_usecase
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();
        assert_eq!(final_thread.comments.len(), 31);
    }

    #[test]
    fn read_only_operations_do_not_wait_for_in_process_write_guard() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let thread = usecase(Arc::clone(&store))
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let _guard = store.file_lock.lock();

        let started = Instant::now();
        let threads = usecase(Arc::clone(&store))
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();
        let got = usecase(Arc::clone(&store))
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(got.id, thread.id);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn read_only_operations_do_not_wait_for_process_file_lock() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let thread = usecase(Arc::clone(&store))
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let _guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();

        let started = Instant::now();
        let threads = usecase(Arc::clone(&store))
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();
        let got = usecase(Arc::clone(&store))
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(got.id, thread.id);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn invalid_content_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let too_long = "a".repeat(MAX_REVIEW_TEXT_BYTES + 1);

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            too_long,
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        assert!(!state_file(dir.path(), "wt").exists());
    }

    #[test]
    fn invalid_target_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let absolute_path = if cfg!(windows) {
            "C:/repo/src/main.rs"
        } else {
            "/repo/src/main.rs"
        };
        let invalid_targets = [
            ReviewTarget {
                file_path: Some(absolute_path.to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src\\main.rs".to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs\0".to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(0),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(5),
                end_line: Some(4),
            },
        ];

        for target in invalid_targets {
            let result = usecase(Arc::clone(&store)).create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target,
                "Claim".to_string(),
            );

            assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
            assert!(!state_file(dir.path(), "wt").exists());
        }
    }

    #[test]
    fn nul_content_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            "bad\0content".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        assert!(!state_file(dir.path(), "wt").exists());
    }

    #[test]
    fn nul_mutations_do_not_update_existing_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let usecase = usecase(store);
        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        let file = state_file(dir.path(), "wt");
        let before = std::fs::read_to_string(&file).unwrap();

        let append = usecase.append_comment(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0comment".to_string(),
        );
        assert!(matches!(append, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

        let outcome = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0outcome".to_string(),
            "done".to_string(),
        );
        assert!(matches!(outcome, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

        let summary = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "bad\0summary".to_string(),
        );
        assert!(matches!(summary, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn persisted_actor_keeps_existing_session_id_field() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .create_thread(
                dir.path(),
                "wt",
                agent("secret-session"),
                target(),
                "Claim".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(state_file(dir.path(), "wt")).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(events[0]["actor"]["sessionId"], "secret-session");
    }

    #[test]
    fn persisted_human_actor_keeps_session_id_null_field() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(state_file(dir.path(), "wt")).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(events[0]["actor"].get("sessionId").is_some());
        assert!(events[0]["actor"]["sessionId"].is_null());
    }

    #[test]
    fn loaded_actor_session_id_survives_rewrite() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        let thread_id = "018f8f6d-0e6a-7b2c-9d10-111111111111";
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let json = r#"[
  {
    "eventType": "thread_created",
    "eventId": "event-1",
    "threadId": "__THREAD_ID__",
    "commentId": "comment-1",
    "actor": {
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": "legacy-session",
      "displayName": "codex/gpt-5"
    },
    "target": {
      "filePath": null,
      "lineNumber": null,
      "endLine": null
    },
    "content": "Claim",
    "at": 1.0
  }
]"#
        .replace("__THREAD_ID__", thread_id);
        std::fs::write(&file, json).unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .append_comment(
                dir.path(),
                "wt",
                agent("new-session"),
                thread_id,
                "Follow-up".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(file).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(events[0]["actor"]["sessionId"], "legacy-session");
        assert_eq!(events[1]["actor"]["sessionId"], "new-session");
    }
}
