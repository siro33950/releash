use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

mod commands;

pub use commands::*;

const MAX_REVIEW_TEXT_BYTES: usize = 65_536;
const MAX_REVIEW_TARGET_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewActorDto {
    pub kind: ReviewActorKind,
    pub backend_id: Option<String>,
    pub model: Option<String>,
    pub display_name: String,
}

impl ReviewActorDto {
    #[cfg(test)]
    pub(crate) fn human() -> Self {
        Self {
            kind: ReviewActorKind::Human,
            backend_id: None,
            model: None,
            display_name: "Human".to_string(),
        }
    }

    pub fn participant_key(&self) -> String {
        match self.kind {
            ReviewActorKind::Human => "human".to_string(),
            ReviewActorKind::Agent => format!(
                "agent:{}:{}",
                self.backend_id.as_deref().unwrap_or_default(),
                self.model.as_deref().unwrap_or_default()
            ),
        }
    }

    fn same_participant(&self, other: &ReviewActor) -> bool {
        self.participant_key() == other.participant_key()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewActor {
    pub(crate) kind: ReviewActorKind,
    pub(crate) backend_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) display_name: String,
}

impl ReviewActor {
    pub(crate) fn human() -> Self {
        Self {
            kind: ReviewActorKind::Human,
            backend_id: None,
            model: None,
            session_id: None,
            display_name: "Human".to_string(),
        }
    }

    pub(crate) fn agent(backend_id: String, model: String, session_id: Option<String>) -> Self {
        let display_name = format!("{backend_id}/{model}");
        Self {
            kind: ReviewActorKind::Agent,
            backend_id: Some(backend_id),
            model: Some(model),
            session_id,
            display_name,
        }
    }

    pub(crate) fn participant_key(&self) -> String {
        match self.kind {
            ReviewActorKind::Human => "human".to_string(),
            ReviewActorKind::Agent => format!(
                "agent:{}:{}",
                self.backend_id.as_deref().unwrap_or_default(),
                self.model.as_deref().unwrap_or_default()
            ),
        }
    }

    fn same_participant(&self, other: &Self) -> bool {
        self.participant_key() == other.participant_key()
    }

    fn redacted_for_public(&self) -> ReviewActorDto {
        ReviewActorDto {
            kind: self.kind.clone(),
            backend_id: self.backend_id.clone(),
            model: self.model.clone(),
            display_name: self.display_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewThreadState {
    Open,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStanceValue {
    Agree,
    Disagree,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTarget {
    pub file_path: Option<String>,
    pub line_number: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub thread_id: String,
    pub author: ReviewActorDto,
    pub content: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStance {
    pub actor: ReviewActorDto,
    pub value: ReviewStanceValue,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResolveInfo {
    pub actor: ReviewActorDto,
    pub outcome: String,
    pub summary: String,
    pub resolved_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub id: String,
    pub worktree_name: String,
    pub author: ReviewActorDto,
    pub target: ReviewTarget,
    pub state: ReviewThreadState,
    pub comments: Vec<ReviewComment>,
    pub stances: Vec<ReviewStance>,
    pub resolve: Option<ReviewResolveInfo>,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u64,
    pub can_resolve: bool,
    pub my_stance: ReviewStanceValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadFilter {
    pub file_path: Option<String>,
    pub state: Option<ReviewThreadState>,
    pub author_key: Option<String>,
    pub my_stance: Option<ReviewStanceValue>,
    pub updated_after: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "eventType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ReviewEvent {
    ThreadCreated {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActor,
        target: ReviewTarget,
        content: String,
        at: f64,
    },
    CommentAppended {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActor,
        content: String,
        at: f64,
    },
    StanceSet {
        event_id: String,
        thread_id: String,
        actor: ReviewActor,
        value: ReviewStanceValue,
        at: f64,
    },
    ThreadResolved {
        event_id: String,
        thread_id: String,
        actor: ReviewActor,
        outcome: String,
        summary: String,
        at: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewHistoryEntry {
    ThreadCreated {
        id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActorDto,
        target: ReviewTarget,
        content: String,
        at: f64,
    },
    CommentAppended {
        id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActorDto,
        content: String,
        at: f64,
    },
    StanceSet {
        id: String,
        thread_id: String,
        actor: ReviewActorDto,
        value: ReviewStanceValue,
        at: f64,
    },
    ThreadResolved {
        id: String,
        thread_id: String,
        actor: ReviewActorDto,
        outcome: String,
        summary: String,
        at: f64,
    },
}

impl From<&ReviewEvent> for ReviewHistoryEntry {
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
                id: event_id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: actor.redacted_for_public(),
                target: target.clone(),
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
                id: event_id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: actor.redacted_for_public(),
                content: content.clone(),
                at: *at,
            },
            ReviewEvent::StanceSet {
                event_id,
                thread_id,
                actor,
                value,
                at,
            } => Self::StanceSet {
                id: event_id.clone(),
                thread_id: thread_id.clone(),
                actor: actor.redacted_for_public(),
                value: value.clone(),
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
                id: event_id.clone(),
                thread_id: thread_id.clone(),
                actor: actor.redacted_for_public(),
                outcome: outcome.clone(),
                summary: summary.clone(),
                at: *at,
            },
        }
    }
}

impl ReviewEvent {
    #[cfg(test)]
    fn event_id(&self) -> &str {
        match self {
            Self::ThreadCreated { event_id, .. }
            | Self::CommentAppended { event_id, .. }
            | Self::StanceSet { event_id, .. }
            | Self::ThreadResolved { event_id, .. } => event_id,
        }
    }

    fn thread_id(&self) -> &str {
        match self {
            Self::ThreadCreated { thread_id, .. }
            | Self::CommentAppended { thread_id, .. }
            | Self::StanceSet { thread_id, .. }
            | Self::ThreadResolved { thread_id, .. } => thread_id,
        }
    }

    fn at(&self) -> f64 {
        match self {
            Self::ThreadCreated { at, .. }
            | Self::CommentAppended { at, .. }
            | Self::StanceSet { at, .. }
            | Self::ThreadResolved { at, .. } => *at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewErrorCode {
    InvalidInput,
    NotFound,
    AlreadyResolved,
    PermissionDenied,
    Io,
    Serialize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewErrorDto {
    pub code: ReviewErrorCode,
    pub message: String,
}

#[derive(Debug)]
pub enum ReviewError {
    InvalidInput(String),
    NotFound(String),
    AlreadyResolved(String),
    PermissionDenied(String),
    Io(std::io::Error),
    Serialize(serde_json::Error),
}

impl ReviewError {
    pub fn code(&self) -> ReviewErrorCode {
        match self {
            Self::InvalidInput(_) => ReviewErrorCode::InvalidInput,
            Self::NotFound(_) => ReviewErrorCode::NotFound,
            Self::AlreadyResolved(_) => ReviewErrorCode::AlreadyResolved,
            Self::PermissionDenied(_) => ReviewErrorCode::PermissionDenied,
            Self::Io(_) => ReviewErrorCode::Io,
            Self::Serialize(_) => ReviewErrorCode::Serialize,
        }
    }

    pub fn dto(&self) -> ReviewErrorDto {
        ReviewErrorDto {
            code: self.code(),
            message: self.to_string(),
        }
    }
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg)
            | Self::NotFound(msg)
            | Self::AlreadyResolved(msg)
            | Self::PermissionDenied(msg) => write!(f, "{msg}"),
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Serialize(e) => write!(f, "Serialization error: {e}"),
        }
    }
}

impl From<std::io::Error> for ReviewError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for ReviewError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialize(e)
    }
}

impl From<ReviewError> for String {
    fn from(e: ReviewError) -> Self {
        serde_json::to_string(&e.dto()).unwrap_or_else(|_| e.to_string())
    }
}

pub struct ReviewPersistenceGateway {
    file_lock: Mutex<()>,
}

impl Default for ReviewPersistenceGateway {
    fn default() -> Self {
        Self {
            file_lock: Mutex::new(()),
        }
    }
}

#[derive(Default)]
pub struct ReviewCommentStore {
    gateway: ReviewPersistenceGateway,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn event_id() -> String {
    Uuid::new_v4().to_string()
}

fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("review-comments")
}

fn worktree_storage_key(worktree: &str) -> String {
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

fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
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
    std::fs::create_dir_all(&dir)?;
    let path = lock_file(app_data_dir, worktree_name);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    writeln!(file, "pid={}", std::process::id())?;
    file.flush()?;
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
            Err(e) => return Err(ReviewError::Io(e)),
        }
    }
}

fn validate_content(content: &str, label: &str) -> Result<(), ReviewError> {
    if content.trim().is_empty() {
        return Err(ReviewError::InvalidInput(format!(
            "{label} must not be empty"
        )));
    }
    if content.len() > MAX_REVIEW_TEXT_BYTES {
        return Err(ReviewError::InvalidInput(format!(
            "{label} must be at most {MAX_REVIEW_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_target(target: &ReviewTarget) -> Result<(), ReviewError> {
    if let Some(file_path) = target.file_path.as_deref() {
        validate_review_file_path(file_path)?;
    }
    validate_line_range(target.line_number, target.end_line)
}

fn validate_filter(filter: &Option<ReviewThreadFilter>) -> Result<(), ReviewError> {
    if let Some(filter) = filter {
        if let Some(file_path) = filter.file_path.as_deref() {
            validate_review_file_path(file_path)?;
        }
    }
    Ok(())
}

fn validate_review_file_path(file_path: &str) -> Result<(), ReviewError> {
    if file_path.is_empty() || file_path.trim() != file_path {
        return Err(ReviewError::InvalidInput(
            "file_path must be a non-empty repo-relative path".to_string(),
        ));
    }
    if file_path.len() > MAX_REVIEW_TARGET_PATH_BYTES {
        return Err(ReviewError::InvalidInput(format!(
            "file_path must be at most {MAX_REVIEW_TARGET_PATH_BYTES} bytes"
        )));
    }
    if file_path.contains('\0') {
        return Err(ReviewError::InvalidInput(
            "file_path must not contain NUL bytes".to_string(),
        ));
    }
    if file_path.contains('\\') {
        return Err(ReviewError::InvalidInput(
            "file_path must use repo-relative '/' separators".to_string(),
        ));
    }
    let path = Path::new(file_path);
    if path.is_absolute() {
        return Err(ReviewError::InvalidInput(
            "file_path must be repo-relative".to_string(),
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(ReviewError::InvalidInput(
                    "file_path must not contain root, prefix, '.', or '..' components".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_line_range(line_number: Option<u32>, end_line: Option<u32>) -> Result<(), ReviewError> {
    if matches!(line_number, Some(0)) || matches!(end_line, Some(0)) {
        return Err(ReviewError::InvalidInput(
            "line_number and end_line must be positive".to_string(),
        ));
    }
    if end_line.is_some() && line_number.is_none() {
        return Err(ReviewError::InvalidInput(
            "line_number is required when end_line is set".to_string(),
        ));
    }
    if let (Some(start), Some(end)) = (line_number, end_line) {
        if end < start {
            return Err(ReviewError::InvalidInput(
                "end_line must be greater than or equal to line_number".to_string(),
            ));
        }
    }
    Ok(())
}

fn ensure_thread_open(thread: &ReviewThread, thread_id: &str) -> Result<(), ReviewError> {
    if thread.state == ReviewThreadState::Resolved {
        return Err(ReviewError::AlreadyResolved(format!(
            "Review thread is already resolved: {thread_id}"
        )));
    }
    Ok(())
}

fn ensure_can_resolve(actor: &ReviewActor, thread: &ReviewThread) -> Result<(), ReviewError> {
    if actor.kind != ReviewActorKind::Human && !thread.author.same_participant(actor) {
        return Err(ReviewError::PermissionDenied(
            "Only the thread author or a human reviewer can resolve this thread".to_string(),
        ));
    }
    Ok(())
}

fn project_thread(
    worktree_name: &str,
    thread_id: &str,
    events: &[ReviewEvent],
    viewer: &ReviewActor,
) -> Option<ReviewThread> {
    let mut author: Option<ReviewActor> = None;
    let mut target: Option<ReviewTarget> = None;
    let mut comments = Vec::new();
    let mut stances_by_actor: HashMap<String, ReviewStance> = HashMap::new();
    let mut resolve = None;
    let mut created_at = 0.0;
    let mut updated_at = 0.0;
    let mut version = 0_u64;

    for event in events.iter().filter(|e| e.thread_id() == thread_id) {
        version += 1;
        updated_at = event.at();
        match event {
            ReviewEvent::ThreadCreated {
                thread_id,
                comment_id,
                actor,
                target: event_target,
                content,
                at,
                ..
            } => {
                author = Some(actor.clone());
                target = Some(event_target.clone());
                created_at = *at;
                comments.push(ReviewComment {
                    id: comment_id.clone(),
                    thread_id: thread_id.clone(),
                    author: actor.redacted_for_public(),
                    content: content.clone(),
                    created_at: *at,
                });
            }
            ReviewEvent::CommentAppended {
                thread_id,
                comment_id,
                actor,
                content,
                at,
                ..
            } => comments.push(ReviewComment {
                id: comment_id.clone(),
                thread_id: thread_id.clone(),
                author: actor.redacted_for_public(),
                content: content.clone(),
                created_at: *at,
            }),
            ReviewEvent::StanceSet {
                actor, value, at, ..
            } => {
                stances_by_actor.insert(
                    actor.participant_key(),
                    ReviewStance {
                        actor: actor.redacted_for_public(),
                        value: value.clone(),
                        updated_at: *at,
                    },
                );
            }
            ReviewEvent::ThreadResolved {
                actor,
                outcome,
                summary,
                at,
                ..
            } => {
                resolve = Some(ReviewResolveInfo {
                    actor: actor.redacted_for_public(),
                    outcome: outcome.clone(),
                    summary: summary.clone(),
                    resolved_at: *at,
                });
            }
        }
    }

    let author = author?;
    let target = target?;
    stances_by_actor
        .entry(author.participant_key())
        .or_insert_with(|| ReviewStance {
            actor: author.redacted_for_public(),
            value: ReviewStanceValue::None,
            updated_at: created_at,
        });
    let mut stances: Vec<ReviewStance> = stances_by_actor.into_values().collect();
    stances.sort_by(|a, b| {
        a.actor
            .display_name
            .cmp(&b.actor.display_name)
            .then_with(|| a.updated_at.total_cmp(&b.updated_at))
    });
    let state = if resolve.is_some() {
        ReviewThreadState::Resolved
    } else {
        ReviewThreadState::Open
    };
    let my_stance = stances
        .iter()
        .find(|stance| stance.actor.same_participant(viewer))
        .map(|stance| stance.value.clone())
        .unwrap_or(ReviewStanceValue::None);
    let can_resolve = state == ReviewThreadState::Open
        && (viewer.kind == ReviewActorKind::Human || author.same_participant(viewer));

    Some(ReviewThread {
        id: thread_id.to_string(),
        worktree_name: worktree_name.to_string(),
        author: author.redacted_for_public(),
        target,
        state,
        comments,
        stances,
        resolve,
        created_at,
        updated_at,
        version,
        can_resolve,
        my_stance,
    })
}

fn project_threads(
    worktree_name: &str,
    events: &[ReviewEvent],
    viewer: &ReviewActor,
) -> Vec<ReviewThread> {
    let mut ids = Vec::<String>::new();
    for event in events {
        if matches!(event, ReviewEvent::ThreadCreated { .. }) {
            ids.push(event.thread_id().to_string());
        }
    }
    let mut threads: Vec<_> = ids
        .iter()
        .filter_map(|id| project_thread(worktree_name, id, events, viewer))
        .collect();
    threads.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    threads
}

fn apply_filter(
    threads: Vec<ReviewThread>,
    filter: Option<ReviewThreadFilter>,
) -> Vec<ReviewThread> {
    let Some(filter) = filter else {
        return threads;
    };
    threads
        .into_iter()
        .filter(|thread| {
            if let Some(file_path) = &filter.file_path {
                if thread.target.file_path.as_deref() != Some(file_path.as_str()) {
                    return false;
                }
            }
            if let Some(state) = &filter.state {
                if &thread.state != state {
                    return false;
                }
            }
            if let Some(author_key) = &filter.author_key {
                if thread.author.participant_key() != *author_key {
                    return false;
                }
            }
            if let Some(updated_after) = filter.updated_after {
                if thread.updated_at <= updated_after {
                    return false;
                }
            }
            if let Some(my_stance) = &filter.my_stance {
                if &thread.my_stance != my_stance {
                    return false;
                }
            }
            true
        })
        .collect()
}

impl ReviewPersistenceGateway {
    fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        let file_path = state_file(app_data_dir, worktree_name);
        let events = if file_path.exists() {
            let data = std::fs::read_to_string(&file_path)?;
            serde_json::from_str::<Vec<ReviewEvent>>(&data)?
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
        std::fs::create_dir_all(&dir)?;
        let file_path = state_file(app_data_dir, worktree_name);
        let tmp_path = file_path.with_extension(format!("events.{}.tmp", event_id()));
        let json = serde_json::to_string_pretty(events)?;
        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        replace_file(&tmp_path, &file_path)?;
        Ok(())
    }
}

fn replace_file(tmp_path: &Path, file_path: &Path) -> Result<(), ReviewError> {
    #[cfg(windows)]
    {
        replace_file_windows(tmp_path, file_path)
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(tmp_path, file_path).map_err(ReviewError::Io)
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
        return Err(ReviewError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

impl ReviewCommentStore {
    fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        self.gateway.load(app_data_dir, worktree_name)
    }

    pub fn list_threads(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        filter: Option<ReviewThreadFilter>,
        viewer: ReviewActor,
    ) -> Result<Vec<ReviewThread>, ReviewError> {
        validate_filter(&filter)?;
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let events = self.load(app_data_dir, worktree_name)?;
        Ok(apply_filter(
            project_threads(worktree_name, &events, &viewer),
            filter,
        ))
    }

    pub fn get_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
        viewer: ReviewActor,
    ) -> Result<ReviewThread, ReviewError> {
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let events = self.load(app_data_dir, worktree_name)?;
        project_thread(worktree_name, thread_id, &events, &viewer)
            .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
    }

    pub fn history(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewHistoryEntry>, ReviewError> {
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        Ok(self
            .history_events(app_data_dir, worktree_name, thread_id)?
            .iter()
            .map(ReviewHistoryEntry::from)
            .collect())
    }

    fn history_events(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        let events: Vec<_> = self
            .load(app_data_dir, worktree_name)?
            .into_iter()
            .filter(|event| event.thread_id() == thread_id)
            .collect();
        if !events
            .iter()
            .any(|event| matches!(event, ReviewEvent::ThreadCreated { .. }))
        {
            return Err(ReviewError::NotFound(format!(
                "Review thread not found: {thread_id}"
            )));
        }
        Ok(events)
    }

    pub fn create_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        target: ReviewTarget,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&content, "content")?;
        validate_target(&target)?;
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load(app_data_dir, worktree_name)?;
        let thread_id = event_id();
        let at = now();
        events.push(ReviewEvent::ThreadCreated {
            event_id: event_id(),
            thread_id: thread_id.clone(),
            comment_id: event_id(),
            actor: actor.clone(),
            target,
            content,
            at,
        });
        self.gateway
            .write_events(app_data_dir, worktree_name, &events)?;
        project_thread(worktree_name, &thread_id, &events, &actor)
            .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
    }

    pub fn append_comment(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&content, "content")?;
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load(app_data_dir, worktree_name)?;
        let thread =
            project_thread(worktree_name, thread_id, &events, &actor).ok_or_else(|| {
                ReviewError::NotFound(format!("Review thread not found: {thread_id}"))
            })?;
        ensure_thread_open(&thread, thread_id)?;
        events.push(ReviewEvent::CommentAppended {
            event_id: event_id(),
            thread_id: thread_id.to_string(),
            comment_id: event_id(),
            actor: actor.clone(),
            content,
            at: now(),
        });
        self.gateway
            .write_events(app_data_dir, worktree_name, &events)?;
        project_thread(worktree_name, thread_id, &events, &actor)
            .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
    }

    pub fn set_stance(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
        value: ReviewStanceValue,
    ) -> Result<ReviewThread, ReviewError> {
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load(app_data_dir, worktree_name)?;
        let thread =
            project_thread(worktree_name, thread_id, &events, &actor).ok_or_else(|| {
                ReviewError::NotFound(format!("Review thread not found: {thread_id}"))
            })?;
        ensure_thread_open(&thread, thread_id)?;
        events.push(ReviewEvent::StanceSet {
            event_id: event_id(),
            thread_id: thread_id.to_string(),
            actor: actor.clone(),
            value,
            at: now(),
        });
        self.gateway
            .write_events(app_data_dir, worktree_name, &events)?;
        project_thread(worktree_name, thread_id, &events, &actor)
            .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
    }

    pub fn resolve_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
        outcome: String,
        summary: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&outcome, "outcome")?;
        validate_content(&summary, "summary")?;
        let _guard = self.gateway.file_lock.lock();
        let _process_guard = acquire_worktree_file_lock(app_data_dir, worktree_name)?;
        let mut events = self.load(app_data_dir, worktree_name)?;
        let thread =
            project_thread(worktree_name, thread_id, &events, &actor).ok_or_else(|| {
                ReviewError::NotFound(format!("Review thread not found: {thread_id}"))
            })?;
        ensure_thread_open(&thread, thread_id)?;
        ensure_can_resolve(&actor, &thread)?;
        events.push(ReviewEvent::ThreadResolved {
            event_id: event_id(),
            thread_id: thread_id.to_string(),
            actor: actor.clone(),
            outcome,
            summary,
            at: now(),
        });
        self.gateway
            .write_events(app_data_dir, worktree_name, &events)?;
        project_thread(worktree_name, thread_id, &events, &actor)
            .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn agent_a(session_id: &str) -> ReviewActor {
        ReviewActor::agent(
            "codex".to_string(),
            "gpt-5".to_string(),
            Some(session_id.to_string()),
        )
    }

    fn agent_b(session_id: &str) -> ReviewActor {
        ReviewActor::agent(
            "claude".to_string(),
            "opus".to_string(),
            Some(session_id.to_string()),
        )
    }

    #[test]
    fn create_thread_requires_initial_comment() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let result = store.create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            "  ".to_string(),
        );
        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
    }

    #[test]
    fn creator_stance_defaults_to_none() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: Some("src/main.rs".to_string()),
                    line_number: Some(10),
                    end_line: None,
                },
                "Initial claim".to_string(),
            )
            .unwrap();
        assert_eq!(thread.state, ReviewThreadState::Open);
        assert_eq!(thread.comments.len(), 1);
        assert_eq!(thread.stances[0].value, ReviewStanceValue::None);
    }

    #[test]
    fn same_backend_model_agent_can_resolve_across_sessions() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();
        let resolved = store
            .resolve_thread(
                dir.path(),
                "wt",
                agent_a("s2"),
                &thread.id,
                "accepted".to_string(),
                "Fixed by follow-up change".to_string(),
            )
            .unwrap();
        assert_eq!(resolved.state, ReviewThreadState::Resolved);
    }

    #[test]
    fn non_author_agent_cannot_resolve() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();
        let result = store.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::agent(
                "claude".to_string(),
                "opus".to_string(),
                Some("s3".to_string()),
            ),
            &thread.id,
            "rejected".to_string(),
            "I disagree".to_string(),
        );
        assert!(matches!(result, Err(ReviewError::PermissionDenied(_))));
        let current = store
            .get_thread(dir.path(), "wt", &thread.id, ReviewActor::human())
            .unwrap();
        assert_eq!(current.state, ReviewThreadState::Open);
    }

    #[test]
    fn human_can_resolve_any_open_thread_and_resolved_blocks_mutation() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();
        store
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "withdrawn".to_string(),
                "Closed by reviewer".to_string(),
            )
            .unwrap();
        let append = store.append_comment(
            dir.path(),
            "wt",
            agent_a("s1"),
            &thread.id,
            "More".to_string(),
        );
        assert!(matches!(append, Err(ReviewError::AlreadyResolved(_))));
    }

    #[test]
    fn stance_last_write_wins_per_actor() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();
        store
            .set_stance(
                dir.path(),
                "wt",
                agent_a("s1"),
                &thread.id,
                ReviewStanceValue::Agree,
            )
            .unwrap();
        let current = store
            .set_stance(
                dir.path(),
                "wt",
                agent_a("s2"),
                &thread.id,
                ReviewStanceValue::Disagree,
            )
            .unwrap();
        let stance = current
            .stances
            .iter()
            .find(|s| s.actor.participant_key() == agent_a("s3").participant_key())
            .unwrap();
        assert_eq!(stance.value, ReviewStanceValue::Disagree);
        assert_eq!(current.my_stance, ReviewStanceValue::Disagree);
    }

    #[test]
    fn stances_are_independent_per_participant_and_viewer() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();

        store
            .set_stance(
                dir.path(),
                "wt",
                agent_a("s2"),
                &thread.id,
                ReviewStanceValue::Agree,
            )
            .unwrap();
        store
            .set_stance(
                dir.path(),
                "wt",
                agent_b("s3"),
                &thread.id,
                ReviewStanceValue::Disagree,
            )
            .unwrap();

        let as_agent_a = store
            .get_thread(dir.path(), "wt", &thread.id, agent_a("s4"))
            .unwrap();
        let as_agent_b = store
            .get_thread(dir.path(), "wt", &thread.id, agent_b("s5"))
            .unwrap();
        let agent_a_stance = as_agent_a
            .stances
            .iter()
            .find(|stance| stance.actor.participant_key() == agent_a("s6").participant_key())
            .unwrap();
        let agent_b_stance = as_agent_a
            .stances
            .iter()
            .find(|stance| stance.actor.participant_key() == agent_b("s7").participant_key())
            .unwrap();

        assert_eq!(agent_a_stance.value, ReviewStanceValue::Agree);
        assert_eq!(agent_b_stance.value, ReviewStanceValue::Disagree);
        assert_eq!(as_agent_a.my_stance, ReviewStanceValue::Agree);
        assert_eq!(as_agent_b.my_stance, ReviewStanceValue::Disagree);
    }

    #[test]
    fn resolved_thread_rejects_stance_and_second_resolve_without_state_change() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();
        let resolved = store
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();

        let stance = store.set_stance(
            dir.path(),
            "wt",
            agent_a("s1"),
            &thread.id,
            ReviewStanceValue::Agree,
        );
        let second_resolve = store.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "again".to_string(),
        );
        let current = store
            .get_thread(dir.path(), "wt", &thread.id, ReviewActor::human())
            .unwrap();

        assert!(matches!(stance, Err(ReviewError::AlreadyResolved(_))));
        assert!(matches!(
            second_resolve,
            Err(ReviewError::AlreadyResolved(_))
        ));
        assert_eq!(current.state, ReviewThreadState::Resolved);
        assert_eq!(current.version, resolved.version);
        assert_eq!(
            current
                .resolve
                .as_ref()
                .map(|resolve| resolve.summary.as_str()),
            Some("done")
        );
    }

    #[test]
    fn worktree_scope_is_independent() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        store
            .create_thread(
                dir.path(),
                "wt-a",
                ReviewActor::human(),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "A".to_string(),
            )
            .unwrap();
        assert!(store
            .list_threads(dir.path(), "wt-b", None, ReviewActor::human())
            .unwrap()
            .is_empty());
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
        let store = ReviewCommentStore::default();

        store
            .create_thread(
                dir.path(),
                &wt_a.to_string_lossy(),
                ReviewActor::human(),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "A".to_string(),
            )
            .unwrap();

        assert!(store
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
    fn public_projection_redacts_session_id() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("secret-session"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();

        let json = serde_json::to_string(&thread).unwrap();
        assert!(!json.contains("sessionId"));
        assert!(!json.contains("secret-session"));
    }

    #[test]
    fn review_text_fields_have_byte_limits() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let too_long = "a".repeat(MAX_REVIEW_TEXT_BYTES + 1);

        let result = store.create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            too_long,
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
    }

    #[test]
    fn target_rejects_paths_outside_repo_and_invalid_ranges() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();

        for file_path in [
            "/etc/passwd",
            "../secret.txt",
            "src/../secret.txt",
            "./src/main.rs",
            "C:\\repo\\secret.txt",
            "bad\0path",
        ] {
            let result = store.create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                ReviewTarget {
                    file_path: Some(file_path.to_string()),
                    line_number: Some(1),
                    end_line: None,
                },
                "A".to_string(),
            );
            assert!(
                matches!(result, Err(ReviewError::InvalidInput(_))),
                "accepted invalid path {file_path:?}"
            );
        }

        for target in [
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(0),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: None,
                end_line: Some(2),
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(3),
                end_line: Some(2),
            },
        ] {
            let result = store.create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target,
                "A".to_string(),
            );
            assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        }
    }

    #[test]
    fn history_rejects_missing_thread() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();

        let result = store.history(dir.path(), "wt", "missing-thread");

        assert!(matches!(result, Err(ReviewError::NotFound(_))));
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
    fn history_returns_thread_events_in_order() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "A".to_string(),
            )
            .unwrap();
        store
            .append_comment(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "B".to_string(),
            )
            .unwrap();
        let history = store.history_events(dir.path(), "wt", &thread.id).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history[0].at() <= history[1].at());
        assert!(!history[0].event_id().is_empty());
    }

    #[test]
    fn load_propagates_parse_error_without_overwriting_file() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{not-json").unwrap();
        let store = ReviewCommentStore::default();

        let result = store.create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            "A".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::Serialize(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not-json");
    }

    #[test]
    fn multi_store_writes_keep_all_comments_and_resolve_once() {
        let dir = TempDir::new().unwrap();
        let first_store = ReviewCommentStore::default();
        let thread = first_store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();

        let mut handles = Vec::new();
        for content in ["A", "B"] {
            let path = dir.path().to_path_buf();
            let thread_id = thread.id.clone();
            handles.push(std::thread::spawn(move || {
                let store = ReviewCommentStore::default();
                store
                    .append_comment(
                        &path,
                        "wt",
                        agent_a(content),
                        &thread_id,
                        content.to_string(),
                    )
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let current = first_store
            .get_thread(dir.path(), "wt", &thread.id, ReviewActor::human())
            .unwrap();
        assert_eq!(current.comments.len(), 3);

        let store_a = ReviewCommentStore::default();
        let resolved = store_a
            .resolve_thread(
                dir.path(),
                "wt",
                agent_a("s2"),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        assert_eq!(resolved.state, ReviewThreadState::Resolved);

        let store_b = ReviewCommentStore::default();
        let second = store_b.resolve_thread(
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
    fn parallel_stance_updates_are_ordered_and_project_last_value() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let thread = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: None,
                    line_number: None,
                    end_line: None,
                },
                "Claim".to_string(),
            )
            .unwrap();

        let mut handles = Vec::new();
        for value in [ReviewStanceValue::Agree, ReviewStanceValue::Disagree] {
            let path = dir.path().to_path_buf();
            let thread_id = thread.id.clone();
            handles.push(std::thread::spawn(move || {
                let store = ReviewCommentStore::default();
                store
                    .set_stance(&path, "wt", agent_a("s2"), &thread_id, value)
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let history = store.history_events(dir.path(), "wt", &thread.id).unwrap();
        let last_stance = history
            .iter()
            .filter_map(|event| match event {
                ReviewEvent::StanceSet { value, .. } => Some(value.clone()),
                _ => None,
            })
            .last()
            .unwrap();
        let current = store
            .get_thread(dir.path(), "wt", &thread.id, agent_a("s2"))
            .unwrap();

        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event, ReviewEvent::StanceSet { .. }))
                .count(),
            2
        );
        assert_eq!(current.my_stance, last_stance);
    }

    #[test]
    fn filters_cover_file_state_author_stance_updated_after_and_combined() {
        let dir = TempDir::new().unwrap();
        let store = ReviewCommentStore::default();
        let first = store
            .create_thread(
                dir.path(),
                "wt",
                agent_a("s1"),
                ReviewTarget {
                    file_path: Some("src/a.rs".to_string()),
                    line_number: Some(1),
                    end_line: None,
                },
                "A".to_string(),
            )
            .unwrap();
        let threshold = first.updated_at;
        let second = store
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                ReviewTarget {
                    file_path: Some("src/b.rs".to_string()),
                    line_number: Some(2),
                    end_line: None,
                },
                "B".to_string(),
            )
            .unwrap();
        store
            .set_stance(
                dir.path(),
                "wt",
                agent_a("s2"),
                &second.id,
                ReviewStanceValue::Agree,
            )
            .unwrap();
        store
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &second.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();

        let filtered = store
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    file_path: Some("src/b.rs".to_string()),
                    state: Some(ReviewThreadState::Resolved),
                    author_key: Some("human".to_string()),
                    my_stance: Some(ReviewStanceValue::Agree),
                    updated_after: Some(threshold),
                }),
                agent_a("viewer"),
            )
            .unwrap();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, second.id);
    }
}
