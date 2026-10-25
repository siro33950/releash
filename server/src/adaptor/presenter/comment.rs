use serde::{Deserialize, Serialize};

use crate::domain::comment as domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewThreadStateDto {
    Open,
    Resolved,
}

impl From<&domain::State> for ReviewThreadStateDto {
    fn from(state: &domain::State) -> Self {
        match state {
            domain::State::Open => Self::Open,
            domain::State::Resolved => Self::Resolved,
        }
    }
}

impl From<ReviewThreadStateDto> for domain::State {
    fn from(state: ReviewThreadStateDto) -> Self {
        match state {
            ReviewThreadStateDto::Open => Self::Open,
            ReviewThreadStateDto::Resolved => Self::Resolved,
        }
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
