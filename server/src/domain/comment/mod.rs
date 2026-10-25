mod entites;
pub use entites::{Author, AuthorKind, Comment, LineSide, Resolve, State, Target, Thread};
mod repository;
pub(crate) use repository::ThreadRepository;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

pub const MAX_REVIEW_TEXT_BYTES: usize = 65_536;
pub(crate) const MAX_REVIEW_TARGET_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewActorDto {
    pub kind: AuthorKind,
    pub backend_id: Option<String>,
    pub model: Option<String>,
    pub display_name: String,
}

pub fn unresolved_thread_count(threads: &[Thread]) -> u32 {
    threads
        .iter()
        .filter(|thread| thread.state == State::Open)
        .count() as u32
}

/// `ReviewThreadFilter.author` の値。spec issues-1022 design.md List contract:
/// 「自分が作成した Thread」/「自分以外が作成した Thread」を表す。
///
/// 任意 author を指定するモードは contract で提供しない (List filter / unread 判定は
/// session から解決した participant_key で完結するため、外部から任意 key を渡す経路を
/// 露出しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthorScope {
    /// viewer と同一 participant (自分が作成した Thread)
    Mine,
    /// viewer と異なる participant (自分以外が作成した Thread)
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ReviewThreadFilter {
    /// Thread 対象の file path (完全一致)。
    pub file: Option<String>,
    /// Thread の `open` / `resolved` 状態。
    pub state: Option<State>,
    /// 作成者が viewer か他者か。任意 author 文字列での絞り込みは contract で提供しない。
    pub author: Option<AuthorScope>,
    /// 未読 (viewer の最後の Comment 追記時刻以降に他者の Comment 追記があるか)。
    /// Resolve は「Comment 追記」に含めない。
    pub unread: Option<bool>,
    /// 指定 id の Thread のみ (空 = 絞らない、非空は配列内 OR、他軸とは AND)。
    pub thread_id: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ReviewEvent {
    ThreadCreated {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: Author,
        target: Target,
        content: String,
        at: f64,
    },
    CommentAppended {
        event_id: String,
        thread_id: String,
        comment_id: String,
        actor: Author,
        content: String,
        at: f64,
    },
    ThreadResolved {
        event_id: String,
        thread_id: String,
        actor: Author,
        outcome: String,
        summary: String,
        at: f64,
    },
    ThreadDeleted {
        event_id: String,
        thread_id: String,
        actor: Author,
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
        target: Target,
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
        let public_actor = |actor: &Author| ReviewActorDto {
            kind: actor.kind.clone(),
            backend_id: actor.backend_id.clone(),
            model: actor.model.clone(),
            display_name: actor.display_name.clone(),
        };
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
                actor: public_actor(actor),
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
                actor: public_actor(actor),
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
                actor: public_actor(actor),
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
                actor: public_actor(actor),
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

/// Thread の操作で生じる業務上または技術上の失敗。
#[derive(Debug)]
pub enum Error {
    SessionNotOpen(String),
    Technical(crate::domain::failure::TechnicalFailure),
    InvalidInput(String),
    NotFound(String),
    AlreadyResolved(String),
    PermissionDenied(String),
    Io(String),
    Serialize(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Technical(error) => std::fmt::Display::fmt(error, f),
            Self::SessionNotOpen(id) => write!(
                f,
                "Session is not open and cannot be used as a review actor: {id}"
            ),
            Self::InvalidInput(msg)
            | Self::NotFound(msg)
            | Self::AlreadyResolved(msg)
            | Self::PermissionDenied(msg) => write!(f, "{msg}"),
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Serialize(e) => write!(f, "Serialization error: {e}"),
        }
    }
}

pub(crate) fn validate_content(content: &str, label: &str) -> Result<(), Error> {
    if content.trim().is_empty() {
        return Err(Error::InvalidInput(format!("{label} must not be empty")));
    }
    if content.contains('\0') {
        return Err(Error::InvalidInput(format!(
            "{label} must not contain NUL bytes"
        )));
    }
    if content.len() > MAX_REVIEW_TEXT_BYTES {
        return Err(Error::InvalidInput(format!(
            "{label} must be at most {MAX_REVIEW_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) fn validate_filter(filter: &Option<ReviewThreadFilter>) -> Result<(), Error> {
    if let Some(filter) = filter {
        if let Some(file) = filter.file.as_deref() {
            validate_review_file_path(file)?;
        }
        for id in &filter.thread_id {
            if id.trim().is_empty() {
                return Err(Error::InvalidInput(
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
pub(crate) fn is_unread_for_viewer(comments: &[Comment], viewer: &Author) -> bool {
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

fn validate_review_file_path(file_path: &str) -> Result<(), Error> {
    if file_path.is_empty() || file_path.trim() != file_path {
        return Err(Error::InvalidInput(
            "file_path must be a non-empty repo-relative path".to_string(),
        ));
    }
    if file_path.len() > MAX_REVIEW_TARGET_PATH_BYTES {
        return Err(Error::InvalidInput(format!(
            "file_path must be at most {MAX_REVIEW_TARGET_PATH_BYTES} bytes"
        )));
    }
    if file_path.contains('\0') {
        return Err(Error::InvalidInput(
            "file_path must not contain NUL bytes".to_string(),
        ));
    }
    if file_path.contains('\\') {
        return Err(Error::InvalidInput(
            "file_path must use repo-relative '/' separators".to_string(),
        ));
    }
    let path = Path::new(file_path);
    if path.is_absolute() {
        return Err(Error::InvalidInput(
            "file_path must be repo-relative".to_string(),
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(Error::InvalidInput(
                    "file_path must not contain root, prefix, '.', or '..' components".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn ensure_thread_open(thread: &Thread, thread_id: &str) -> Result<(), Error> {
    if thread.state == State::Resolved {
        return Err(Error::AlreadyResolved(format!(
            "Review thread is already resolved: {thread_id}"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_can_delete(actor: &Author) -> Result<(), Error> {
    if actor.kind != AuthorKind::Human {
        return Err(Error::PermissionDenied(
            "Only a human reviewer can delete a review thread".to_string(),
        ));
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct ThreadAccumulator {
    target: Option<Target>,
    comments: Vec<Comment>,
    resolve: Option<Resolve>,
    created_at: f64,
    updated_at: f64,
    version: u64,
    deleted: bool,
}

impl ThreadAccumulator {
    /// Thread の事実を会話、対象、解決結果へ反映する。
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
                self.target = Some(event_target.clone());
                self.created_at = *at;
                self.comments.push(Comment {
                    id: comment_id.clone(),
                    thread_id: thread_id.clone(),
                    author: actor.clone(),
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
            } => self.comments.push(Comment {
                id: comment_id.clone(),
                thread_id: thread_id.clone(),
                author: actor.clone(),
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
                self.resolve = Some(Resolve {
                    actor: actor.clone(),
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

    /// 削除済みでない Thread を、最初の Comment を本文として復元する。
    pub(crate) fn finish(self, worktree_name: &str, thread_id: &str) -> Option<Thread> {
        if self.deleted {
            return None;
        }

        self.comments.first()?;
        let target = self.target?;
        let state = if self.resolve.is_some() {
            State::Resolved
        } else {
            State::Open
        };
        let can_resolve = state == State::Open;

        Some(Thread {
            id: thread_id.to_string(),
            worktree_name: worktree_name.to_string(),
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
) -> Option<Thread> {
    let mut accumulator = ThreadAccumulator::default();
    for event in events.iter().filter(|e| e.thread_id() == thread_id) {
        accumulator.apply(event);
    }
    accumulator.finish(worktree_name, thread_id)
}

fn project_threads_from_iter<'a>(
    worktree_name: &str,
    events: impl IntoIterator<Item = &'a ReviewEvent>,
) -> Vec<Thread> {
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

pub(crate) fn project_threads(worktree_name: &str, events: &[ReviewEvent]) -> Vec<Thread> {
    project_threads_from_iter(worktree_name, events.iter())
}

/// 作成者を最初の Comment から判定し、指定された条件に合う Thread を選ぶ。
pub(crate) fn apply_filter(
    threads: Vec<Thread>,
    filter: Option<ReviewThreadFilter>,
    viewer: &Author,
) -> Vec<Thread> {
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
                let is_mine = thread
                    .comments
                    .first()
                    .is_some_and(|comment| comment.author.participant_key() == viewer_key);
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

impl AuthorScope {
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "self" => Ok(Self::Mine),
            "other" => Ok(Self::Other),
            _ => Err(Error::InvalidInput("Invalid review author".into())),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mine => "self",
            Self::Other => "other",
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ReviewWorktreeFilter {
    pub file: Option<String>,
    pub state: Option<State>,
    pub thread_id: Vec<String>,
}
impl From<ReviewWorktreeFilter> for ReviewThreadFilter {
    fn from(value: ReviewWorktreeFilter) -> Self {
        Self {
            file: value.file,
            state: value.state,
            thread_id: value.thread_id,
            ..Default::default()
        }
    }
}
pub(crate) fn ensure_session_can_review(
    lifecycle: crate::domain::agent_session::aggregates::AgentSessionLifecycle,
    id: &str,
) -> Result<(), Error> {
    if lifecycle == crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open {
        Ok(())
    } else {
        Err(Error::SessionNotOpen(id.into()))
    }
}

impl ReviewThreadFilter {
    pub fn parse_unread(value: &str) -> Result<bool, Error> {
        value
            .parse()
            .map_err(|_| Error::InvalidInput("Invalid review unread".into()))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;
