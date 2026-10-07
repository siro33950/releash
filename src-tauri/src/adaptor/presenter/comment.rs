use serde::{Deserialize, Serialize};

use crate::domain::comment as domain;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewActorKindWireDto {
    Human,
    Agent,
}

impl From<&domain::ReviewActorKind> for ReviewActorKindWireDto {
    fn from(kind: &domain::ReviewActorKind) -> Self {
        match kind {
            domain::ReviewActorKind::Human => Self::Human,
            domain::ReviewActorKind::Agent => Self::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewActorWireDto {
    pub kind: ReviewActorKindWireDto,
    pub backend_id: Option<String>,
    pub model: Option<String>,
    pub display_name: String,
}

impl From<&domain::ReviewActorDto> for ReviewActorWireDto {
    fn from(actor: &domain::ReviewActorDto) -> Self {
        Self {
            kind: ReviewActorKindWireDto::from(&actor.kind),
            backend_id: actor.backend_id.clone(),
            model: actor.model.clone(),
            display_name: actor.display_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewThreadStateDto {
    Open,
    Resolved,
}

impl From<&domain::ReviewThreadState> for ReviewThreadStateDto {
    fn from(state: &domain::ReviewThreadState) -> Self {
        match state {
            domain::ReviewThreadState::Open => Self::Open,
            domain::ReviewThreadState::Resolved => Self::Resolved,
        }
    }
}

impl From<ReviewThreadStateDto> for domain::ReviewThreadState {
    fn from(state: ReviewThreadStateDto) -> Self {
        match state {
            ReviewThreadStateDto::Open => Self::Open,
            ReviewThreadStateDto::Resolved => Self::Resolved,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTargetWireDto {
    pub file_path: Option<String>,
    pub line_number: Option<u32>,
    pub end_line: Option<u32>,
}

impl From<&domain::ReviewTarget> for ReviewTargetWireDto {
    fn from(target: &domain::ReviewTarget) -> Self {
        Self {
            file_path: target.file_path.clone(),
            line_number: target.line_number,
            end_line: target.end_line,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommentDto {
    pub id: String,
    pub thread_id: String,
    pub author: ReviewActorWireDto,
    pub content: String,
    pub created_at: f64,
}

impl From<&domain::ReviewComment> for ReviewCommentDto {
    fn from(comment: &domain::ReviewComment) -> Self {
        Self {
            id: comment.id.clone(),
            thread_id: comment.thread_id.clone(),
            author: ReviewActorWireDto::from(&comment.author),
            content: comment.content.clone(),
            created_at: comment.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResolveInfoDto {
    pub actor: ReviewActorWireDto,
    pub outcome: String,
    pub summary: String,
    pub resolved_at: f64,
}

impl From<&domain::ReviewResolveInfo> for ReviewResolveInfoDto {
    fn from(resolve: &domain::ReviewResolveInfo) -> Self {
        Self {
            actor: ReviewActorWireDto::from(&resolve.actor),
            outcome: resolve.outcome.clone(),
            summary: resolve.summary.clone(),
            resolved_at: resolve.resolved_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadDto {
    pub id: String,
    pub worktree_name: String,
    pub author: ReviewActorWireDto,
    pub target: ReviewTargetWireDto,
    pub state: ReviewThreadStateDto,
    pub comments: Vec<ReviewCommentDto>,
    pub resolve: Option<ReviewResolveInfoDto>,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u64,
    pub can_resolve: bool,
}

impl From<&domain::ReviewThread> for ReviewThreadDto {
    fn from(thread: &domain::ReviewThread) -> Self {
        Self {
            id: thread.id.clone(),
            worktree_name: thread.worktree_name.clone(),
            author: ReviewActorWireDto::from(&thread.author),
            target: ReviewTargetWireDto::from(&thread.target),
            state: ReviewThreadStateDto::from(&thread.state),
            comments: thread.comments.iter().map(ReviewCommentDto::from).collect(),
            resolve: thread.resolve.as_ref().map(ReviewResolveInfoDto::from),
            created_at: thread.created_at,
            updated_at: thread.updated_at,
            version: thread.version,
            can_resolve: thread.can_resolve,
        }
    }
}

impl From<domain::ReviewThread> for ReviewThreadDto {
    fn from(thread: domain::ReviewThread) -> Self {
        Self::from(&thread)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ReviewHistoryEntryDto {
    ThreadCreated {
        id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActorWireDto,
        target: ReviewTargetWireDto,
        content: String,
        at: f64,
    },
    CommentAppended {
        id: String,
        thread_id: String,
        comment_id: String,
        actor: ReviewActorWireDto,
        content: String,
        at: f64,
    },
    ThreadResolved {
        id: String,
        thread_id: String,
        actor: ReviewActorWireDto,
        outcome: String,
        summary: String,
        at: f64,
    },
    ThreadDeleted {
        id: String,
        thread_id: String,
        actor: ReviewActorWireDto,
        at: f64,
    },
}

impl From<&domain::ReviewHistoryEntry> for ReviewHistoryEntryDto {
    fn from(entry: &domain::ReviewHistoryEntry) -> Self {
        match entry {
            domain::ReviewHistoryEntry::ThreadCreated {
                id,
                thread_id,
                comment_id,
                actor,
                target,
                content,
                at,
            } => Self::ThreadCreated {
                id: id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: ReviewActorWireDto::from(actor),
                target: ReviewTargetWireDto::from(target),
                content: content.clone(),
                at: *at,
            },
            domain::ReviewHistoryEntry::CommentAppended {
                id,
                thread_id,
                comment_id,
                actor,
                content,
                at,
            } => Self::CommentAppended {
                id: id.clone(),
                thread_id: thread_id.clone(),
                comment_id: comment_id.clone(),
                actor: ReviewActorWireDto::from(actor),
                content: content.clone(),
                at: *at,
            },
            domain::ReviewHistoryEntry::ThreadResolved {
                id,
                thread_id,
                actor,
                outcome,
                summary,
                at,
            } => Self::ThreadResolved {
                id: id.clone(),
                thread_id: thread_id.clone(),
                actor: ReviewActorWireDto::from(actor),
                outcome: outcome.clone(),
                summary: summary.clone(),
                at: *at,
            },
            domain::ReviewHistoryEntry::ThreadDeleted {
                id,
                thread_id,
                actor,
                at,
            } => Self::ThreadDeleted {
                id: id.clone(),
                thread_id: thread_id.clone(),
                actor: ReviewActorWireDto::from(actor),
                at: *at,
            },
        }
    }
}

impl From<domain::ReviewHistoryEntry> for ReviewHistoryEntryDto {
    fn from(entry: domain::ReviewHistoryEntry) -> Self {
        Self::from(&entry)
    }
}

impl TryFrom<domain::ReviewHistoryEntry>
    for crate::adaptor::presenter::client::ReviewHistoryEntryDto
{
    type Error = String;
    fn try_from(value: domain::ReviewHistoryEntry) -> Result<Self, String> {
        use crate::adaptor::presenter::client as wire;
        use wire::review_history_entry_dto::Entry;
        let actor = |actor| wire::ReviewActorWireDto::try_from(actor);
        Ok(Self {
            entry: Some(match value {
                domain::ReviewHistoryEntry::ThreadCreated {
                    id,
                    thread_id,
                    comment_id,
                    actor: author,
                    target,
                    content,
                    at,
                } => Entry::ThreadCreated(wire::ReviewHistoryThreadCreatedDto {
                    id,
                    thread_id,
                    comment_id,
                    actor: Some(actor(author)?),
                    target: Some(wire::ReviewTargetWireDto::try_from(target)?),
                    content,
                    at,
                }),
                domain::ReviewHistoryEntry::CommentAppended {
                    id,
                    thread_id,
                    comment_id,
                    actor: author,
                    content,
                    at,
                } => Entry::CommentAppended(wire::ReviewHistoryCommentAppendedDto {
                    id,
                    thread_id,
                    comment_id,
                    actor: Some(actor(author)?),
                    content,
                    at,
                }),
                domain::ReviewHistoryEntry::ThreadResolved {
                    id,
                    thread_id,
                    actor: author,
                    outcome,
                    summary,
                    at,
                } => Entry::ThreadResolved(wire::ReviewHistoryThreadResolvedDto {
                    id,
                    thread_id,
                    actor: Some(actor(author)?),
                    outcome,
                    summary,
                    at,
                }),
                domain::ReviewHistoryEntry::ThreadDeleted {
                    id,
                    thread_id,
                    actor: author,
                    at,
                } => Entry::ThreadDeleted(wire::ReviewHistoryThreadDeletedDto {
                    id,
                    thread_id,
                    actor: Some(actor(author)?),
                    at,
                }),
            }),
        })
    }
}
