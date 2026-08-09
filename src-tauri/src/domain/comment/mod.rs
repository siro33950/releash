use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

pub(crate) const MAX_REVIEW_TEXT_BYTES: usize = 65_536;
pub(crate) const MAX_REVIEW_TARGET_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewActorDto {
    pub kind: ReviewActorKind,
    pub backend_id: Option<String>,
    pub model: Option<String>,
    pub display_name: String,
}

impl ReviewActorDto {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub(crate) fn provider_agent(backend_id: String, session_id: Option<String>) -> Self {
        Self {
            kind: ReviewActorKind::Agent,
            display_name: backend_id.clone(),
            backend_id: Some(backend_id),
            model: None,
            session_id,
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

    pub(crate) fn redacted_for_public(&self) -> ReviewActorDto {
        ReviewActorDto {
            kind: self.kind.clone(),
            backend_id: self.backend_id.clone(),
            model: self.model.clone(),
            display_name: self.display_name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewThreadState {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub file_path: Option<String>,
    pub line_number: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewComment {
    pub id: String,
    pub thread_id: String,
    pub author: ReviewActorDto,
    pub content: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewResolveInfo {
    pub actor: ReviewActorDto,
    pub outcome: String,
    pub summary: String,
    pub resolved_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewThread {
    pub id: String,
    pub worktree_name: String,
    pub author: ReviewActorDto,
    pub target: ReviewTarget,
    pub state: ReviewThreadState,
    pub comments: Vec<ReviewComment>,
    pub resolve: Option<ReviewResolveInfo>,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u64,
    pub can_resolve: bool,
}

/// `ReviewThreadFilter.author` の値。spec issues-1022 design.md List contract:
/// 「自分が作成した Thread」/「自分以外が作成した Thread」を表す。
///
/// 任意 author を指定するモードは contract で提供しない (List filter / unread 判定は
/// session から解決した participant_key で完結するため、外部から任意 key を渡す経路を
/// 露出しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorScope {
    /// viewer と同一 participant (自分が作成した Thread)
    Mine,
    /// viewer と異なる participant (自分以外が作成した Thread)
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct ReviewThreadFilter {
    /// Thread 対象の file path (完全一致)。
    pub file: Option<String>,
    /// Thread の `open` / `resolved` 状態。
    pub state: Option<ReviewThreadState>,
    /// 作成者が viewer か他者か。任意 author 文字列での絞り込みは contract で提供しない。
    pub author: Option<AuthorScope>,
    /// 未読 (viewer の最後の Comment 追記時刻以降に他者の Comment 追記があるか)。
    /// Resolve は「Comment 追記」に含めない。
    pub unread: Option<bool>,
    /// 指定 id の Thread のみ (空 = 絞らない、非空は配列内 OR、他軸とは AND)。
    pub thread_id: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum ReviewEvent {
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
    ThreadResolved {
        event_id: String,
        thread_id: String,
        actor: ReviewActor,
        outcome: String,
        summary: String,
        at: f64,
    },
    ThreadDeleted {
        event_id: String,
        thread_id: String,
        actor: ReviewActor,
        at: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
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
    ThreadResolved {
        id: String,
        thread_id: String,
        actor: ReviewActorDto,
        outcome: String,
        summary: String,
        at: f64,
    },
    ThreadDeleted {
        id: String,
        thread_id: String,
        actor: ReviewActorDto,
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
            ReviewEvent::ThreadDeleted {
                event_id,
                thread_id,
                actor,
                at,
            } => Self::ThreadDeleted {
                id: event_id.clone(),
                thread_id: thread_id.clone(),
                actor: actor.redacted_for_public(),
                at: *at,
            },
        }
    }
}

impl ReviewEvent {
    pub(crate) fn thread_id(&self) -> &str {
        match self {
            Self::ThreadCreated { thread_id, .. }
            | Self::CommentAppended { thread_id, .. }
            | Self::ThreadResolved { thread_id, .. }
            | Self::ThreadDeleted { thread_id, .. } => thread_id,
        }
    }

    pub(crate) fn at(&self) -> f64 {
        match self {
            Self::ThreadCreated { at, .. }
            | Self::CommentAppended { at, .. }
            | Self::ThreadResolved { at, .. }
            | Self::ThreadDeleted { at, .. } => *at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewErrorCode {
    InvalidInput,
    NotFound,
    AlreadyResolved,
    PermissionDenied,
    Io,
    Serialize,
}

#[derive(Debug)]
pub enum ReviewError {
    InvalidInput(String),
    NotFound(String),
    AlreadyResolved(String),
    PermissionDenied(String),
    Io(String),
    Serialize(String),
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

pub(crate) fn validate_content(content: &str, label: &str) -> Result<(), ReviewError> {
    if content.trim().is_empty() {
        return Err(ReviewError::InvalidInput(format!(
            "{label} must not be empty"
        )));
    }
    if content.contains('\0') {
        return Err(ReviewError::InvalidInput(format!(
            "{label} must not contain NUL bytes"
        )));
    }
    if content.len() > MAX_REVIEW_TEXT_BYTES {
        return Err(ReviewError::InvalidInput(format!(
            "{label} must be at most {MAX_REVIEW_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) fn validate_target(target: &ReviewTarget) -> Result<(), ReviewError> {
    if let Some(file_path) = target.file_path.as_deref() {
        validate_review_file_path(file_path)?;
    }
    validate_line_range(target.line_number, target.end_line)
}

pub(crate) fn validate_filter(filter: &Option<ReviewThreadFilter>) -> Result<(), ReviewError> {
    if let Some(filter) = filter {
        if let Some(file) = filter.file.as_deref() {
            validate_review_file_path(file)?;
        }
        for id in &filter.thread_id {
            if id.trim().is_empty() {
                return Err(ReviewError::InvalidInput(
                    "thread_id filter must contain only non-empty values".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// viewer から見て当該 Thread が未読か (viewer の最後 Comment 追記時刻以降に他者 Comment 追記
/// があるか) を判定する pure 関数。
///
/// spec issues-1022 design.md List contract「unread (自分の最後の Comment 追記以降に他者
/// Comment 追記があるか)。Resolve は『Comment 追記』に含めない」を実装する。
///
/// - viewer 未投稿で他者 Comment あり → true
/// - viewer が最後の Comment 投稿者 → false
/// - viewer 投稿後に他者 Comment あり → true
pub(crate) fn is_unread_for_viewer(comments: &[ReviewComment], viewer: &ReviewActor) -> bool {
    let viewer_key = viewer.participant_key();
    let viewer_last_at = comments
        .iter()
        .filter(|c| c.author.participant_key() == viewer_key)
        .map(|c| c.created_at)
        .fold(f64::NEG_INFINITY, f64::max);
    comments
        .iter()
        .any(|c| c.author.participant_key() != viewer_key && c.created_at > viewer_last_at)
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

pub(crate) fn ensure_thread_open(
    thread: &ReviewThread,
    thread_id: &str,
) -> Result<(), ReviewError> {
    if thread.state == ReviewThreadState::Resolved {
        return Err(ReviewError::AlreadyResolved(format!(
            "Review thread is already resolved: {thread_id}"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_can_delete(actor: &ReviewActor) -> Result<(), ReviewError> {
    if actor.kind != ReviewActorKind::Human {
        return Err(ReviewError::PermissionDenied(
            "Only a human reviewer can delete a review thread".to_string(),
        ));
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct ThreadAccumulator {
    author: Option<ReviewActor>,
    target: Option<ReviewTarget>,
    comments: Vec<ReviewComment>,
    resolve: Option<ReviewResolveInfo>,
    created_at: f64,
    updated_at: f64,
    version: u64,
    deleted: bool,
}

impl ThreadAccumulator {
    pub(crate) fn apply(&mut self, event: &ReviewEvent) {
        self.version += 1;
        self.updated_at = event.at();
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
                self.author = Some(actor.clone());
                self.target = Some(event_target.clone());
                self.created_at = *at;
                self.comments.push(ReviewComment {
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
            } => self.comments.push(ReviewComment {
                id: comment_id.clone(),
                thread_id: thread_id.clone(),
                author: actor.redacted_for_public(),
                content: content.clone(),
                created_at: *at,
            }),
            ReviewEvent::ThreadResolved {
                actor,
                outcome,
                summary,
                at,
                ..
            } => {
                self.resolve = Some(ReviewResolveInfo {
                    actor: actor.redacted_for_public(),
                    outcome: outcome.clone(),
                    summary: summary.clone(),
                    resolved_at: *at,
                });
            }
            ReviewEvent::ThreadDeleted { .. } => {
                self.deleted = true;
            }
        }
    }

    pub(crate) fn finish(self, worktree_name: &str, thread_id: &str) -> Option<ReviewThread> {
        if self.deleted {
            return None;
        }

        let author = self.author?;
        let target = self.target?;
        let state = if self.resolve.is_some() {
            ReviewThreadState::Resolved
        } else {
            ReviewThreadState::Open
        };
        let can_resolve = state == ReviewThreadState::Open;

        Some(ReviewThread {
            id: thread_id.to_string(),
            worktree_name: worktree_name.to_string(),
            author: author.redacted_for_public(),
            target,
            state,
            comments: self.comments,
            resolve: self.resolve,
            created_at: self.created_at,
            updated_at: self.updated_at,
            version: self.version,
            can_resolve,
        })
    }
}

pub(crate) fn project_thread(
    worktree_name: &str,
    thread_id: &str,
    events: &[ReviewEvent],
) -> Option<ReviewThread> {
    let mut accumulator = ThreadAccumulator::default();
    for event in events.iter().filter(|e| e.thread_id() == thread_id) {
        accumulator.apply(event);
    }
    accumulator.finish(worktree_name, thread_id)
}

fn project_threads_from_iter<'a>(
    worktree_name: &str,
    events: impl IntoIterator<Item = &'a ReviewEvent>,
) -> Vec<ReviewThread> {
    let mut order = Vec::<String>::new();
    let mut ordered = HashSet::<String>::new();
    let mut accumulators = HashMap::<String, ThreadAccumulator>::new();
    for event in events {
        let thread_id = event.thread_id().to_string();
        if matches!(event, ReviewEvent::ThreadCreated { .. }) && ordered.insert(thread_id.clone()) {
            order.push(thread_id.clone());
        }
        accumulators.entry(thread_id).or_default().apply(event);
    }
    let mut threads: Vec<_> = order
        .into_iter()
        .filter_map(|id| {
            accumulators
                .remove(&id)
                .and_then(|accumulator| accumulator.finish(worktree_name, &id))
        })
        .collect();
    threads.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    threads
}

pub(crate) fn project_threads(worktree_name: &str, events: &[ReviewEvent]) -> Vec<ReviewThread> {
    project_threads_from_iter(worktree_name, events.iter())
}

pub(crate) fn apply_filter(
    threads: Vec<ReviewThread>,
    filter: Option<ReviewThreadFilter>,
    viewer: &ReviewActor,
) -> Vec<ReviewThread> {
    let Some(filter) = filter else {
        return threads;
    };
    let viewer_key = viewer.participant_key();
    threads
        .into_iter()
        .filter(|thread| {
            if let Some(file) = &filter.file {
                if thread.target.file_path.as_deref() != Some(file.as_str()) {
                    return false;
                }
            }
            if let Some(state) = &filter.state {
                if &thread.state != state {
                    return false;
                }
            }
            if let Some(author) = &filter.author {
                let is_mine = thread.author.participant_key() == viewer_key;
                let matches = match author {
                    AuthorScope::Mine => is_mine,
                    AuthorScope::Other => !is_mine,
                };
                if !matches {
                    return false;
                }
            }
            if let Some(unread) = filter.unread {
                if is_unread_for_viewer(&thread.comments, viewer) != unread {
                    return false;
                }
            }
            if !filter.thread_id.is_empty() && !filter.thread_id.iter().any(|id| id == &thread.id) {
                return false;
            }
            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_tui_actor_does_not_fabricate_a_model_identity() {
        let actor =
            ReviewActor::provider_agent("claude".to_string(), Some("session-1".to_string()));

        assert_eq!(actor.backend_id.as_deref(), Some("claude"));
        assert_eq!(actor.model, None);
        assert_eq!(actor.session_id.as_deref(), Some("session-1"));
        assert_eq!(actor.display_name, "claude");
        assert_eq!(actor.participant_key(), "agent:claude:");
    }

    fn agent(backend_id: &str, model: &str) -> ReviewActor {
        ReviewActor::agent(backend_id.to_string(), model.to_string(), None)
    }

    fn target(file_path: Option<&str>) -> ReviewTarget {
        ReviewTarget {
            file_path: file_path.map(str::to_string),
            line_number: Some(1),
            end_line: None,
        }
    }

    fn created(
        thread_id: &str,
        actor: ReviewActor,
        file_path: Option<&str>,
        at: f64,
    ) -> ReviewEvent {
        ReviewEvent::ThreadCreated {
            event_id: format!("event-{thread_id}"),
            thread_id: thread_id.to_string(),
            comment_id: format!("comment-{thread_id}"),
            actor,
            target: target(file_path),
            content: format!("content-{thread_id}"),
            at,
        }
    }

    #[test]
    fn validates_content_target_and_filter_inputs() {
        assert!(matches!(
            validate_content(" ", "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content("has\0nul", "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content(&"a".repeat(MAX_REVIEW_TEXT_BYTES + 1), "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_line_range(None, Some(5)),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(ReviewError::InvalidInput(_))
        ));
    }

    #[test]
    fn validates_review_target_path_boundaries() {
        let absolute_path = if cfg!(windows) {
            "C:/repo/src/main.rs"
        } else {
            "/repo/src/main.rs"
        };
        let long_path = "a".repeat(MAX_REVIEW_TARGET_PATH_BYTES + 1);
        let invalid_paths = [
            absolute_path.to_string(),
            "src\\main.rs".to_string(),
            "src/main.rs\0".to_string(),
            long_path,
            "./src/main.rs".to_string(),
            "../outside".to_string(),
            "path/../traversal".to_string(),
        ];

        for file_path in invalid_paths {
            assert!(
                matches!(
                    validate_review_file_path(&file_path),
                    Err(ReviewError::InvalidInput(_))
                ),
                "expected invalid file_path: {file_path:?}"
            );
        }
    }

    #[test]
    fn validates_review_target_line_range_boundaries() {
        let invalid_ranges = [(Some(0), None), (Some(5), Some(4))];

        for (line_number, end_line) in invalid_ranges {
            assert!(
                matches!(
                    validate_line_range(line_number, end_line),
                    Err(ReviewError::InvalidInput(_))
                ),
                "expected invalid range: {line_number:?}..{end_line:?}"
            );
        }
    }

    #[test]
    fn validates_existing_target_and_filter_regressions() {
        assert!(matches!(
            validate_review_file_path("../outside"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_review_file_path("path/../traversal"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_line_range(None, Some(5)),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(ReviewError::InvalidInput(_))
        ));
    }

    #[test]
    fn projects_threads_sorted_by_updated_at_and_excludes_deleted() {
        let viewer = ReviewActor::human();
        let other = agent("codex", "gpt-5");
        let events = vec![
            created("old", viewer.clone(), Some("src/a.rs"), 1.0),
            created("new", other.clone(), Some("src/b.rs"), 2.0),
            ReviewEvent::CommentAppended {
                event_id: "event-old-2".to_string(),
                thread_id: "old".to_string(),
                comment_id: "comment-old-2".to_string(),
                actor: other,
                content: "update".to_string(),
                at: 3.0,
            },
            created("deleted", viewer.clone(), None, 4.0),
            ReviewEvent::ThreadDeleted {
                event_id: "event-delete".to_string(),
                thread_id: "deleted".to_string(),
                actor: viewer,
                at: 5.0,
            },
        ];

        let threads = project_threads("wt", &events);

        assert_eq!(
            threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["old", "new"]
        );
    }

    #[test]
    fn filter_combines_axes_and_unread_ignores_resolve() {
        let viewer = agent("codex", "gpt-5");
        let other = ReviewActor::human();
        let events = vec![
            created("mine", viewer.clone(), Some("src/a.rs"), 1.0),
            created("other", other.clone(), Some("src/a.rs"), 2.0),
            ReviewEvent::CommentAppended {
                event_id: "event-other-comment".to_string(),
                thread_id: "other".to_string(),
                comment_id: "comment-other-2".to_string(),
                actor: viewer.clone(),
                content: "viewer last".to_string(),
                at: 3.0,
            },
            ReviewEvent::ThreadResolved {
                event_id: "event-resolve".to_string(),
                thread_id: "other".to_string(),
                actor: other,
                outcome: "accepted".to_string(),
                summary: "done".to_string(),
                at: 4.0,
            },
        ];
        let threads = project_threads("wt", &events);

        let filtered = apply_filter(
            threads,
            Some(ReviewThreadFilter {
                file: Some("src/a.rs".to_string()),
                state: Some(ReviewThreadState::Resolved),
                author: Some(AuthorScope::Other),
                unread: Some(false),
                thread_id: vec!["other".to_string()],
            }),
            &viewer,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "other");
    }
}
