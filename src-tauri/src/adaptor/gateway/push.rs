use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::gateway::repository::watch::{FileChangeEvent, GitStatusChangedEvent};
use crate::adaptor::protocol::workflow::WorkflowExecutionChangedPayloadView;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_state::snapshot::RepositorySnapshotChangedEvent;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionChangedPayload<'a> {
    pub worktree_path: &'a str,
}

pub enum BackendPush<'a> {
    AgentSessionChanged(AgentSessionChangedPayload<'a>),
    BranchListSync,
    FileChange(FileChangeEvent),
    GitStatusChanged(GitStatusChangedEvent),
    RepoPathsChanged(&'a [String]),
    RepositorySnapshotChanged(RepositorySnapshotChangedEvent),
    ReviewCommentsChanged(&'a str),
    WorkflowExecutionChanged(Box<WorkflowExecutionChangedPayloadView>),
}

impl BackendPush<'_> {
    pub fn emit<R: tauri::Runtime>(self, app: &tauri::AppHandle<R>) {
        use crate::adaptor::controller::api::protocol::client as wire;
        use prost::Message;
        let sink = app.state::<Arc<PushSink>>();
        macro_rules! publish {
            ($name:literal, $variant:ident, $value:expr) => {{
                let event = $value.map(wire::push::Event::$variant);
                match event {
                    Ok(event) => sink.send(
                        wire::Envelope {
                            body: Some(wire::envelope::Body::Push(wire::Push {
                                event: Some(event),
                            })),
                        }
                        .encode_to_vec(),
                    ),
                    Err(error) => {
                        log::error!("Client push conversion failed for {}: {error}", $name)
                    }
                }
            }};
        }
        match self {
            Self::AgentSessionChanged(payload) => publish!(
                "agent-session-changed",
                AgentSessionChanged,
                Ok::<_, String>(wire::AgentSessionChangedPayload {
                    worktree_path: Some(payload.worktree_path.into())
                })
            ),
            Self::BranchListSync => publish!(
                "branch-list-sync",
                BranchListSync,
                Ok::<_, String>(wire::Unit {})
            ),
            Self::FileChange(payload) => publish!(
                "file-change",
                FileChange,
                wire::FileChangeEvent::try_from(payload)
            ),
            Self::GitStatusChanged(payload) => publish!(
                "git-status-changed",
                GitStatusChanged,
                wire::GitStatusChangedEvent::try_from(payload)
            ),
            Self::RepoPathsChanged(payload) => publish!(
                "repo-paths-changed",
                RepoPathsChanged,
                Ok::<_, String>(wire::Liststring {
                    items: payload.to_vec()
                })
            ),
            Self::RepositorySnapshotChanged(payload) => publish!(
                "repository-snapshot-changed",
                RepositorySnapshotChanged,
                wire::RepositorySnapshotChangedEvent::try_from(payload)
            ),
            Self::ReviewCommentsChanged(payload) => publish!(
                "review-comments-changed",
                ReviewCommentsChanged,
                Ok::<_, String>(wire::ResultString {
                    value: Some(payload.into())
                })
            ),
            Self::WorkflowExecutionChanged(payload) => publish!(
                "workflow-execution-changed",
                WorkflowExecutionChanged,
                wire::WorkflowExecutionChangedPayloadView::try_from(*payload).map(Box::new)
            ),
        }
    }
}

pub(crate) struct CommentChangeGateway {
    notify: Box<dyn Fn(&str) + Send + Sync>,
}
impl CommentChangeGateway {
    pub(crate) fn new<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Self {
        Self {
            notify: Box::new(move |worktree| {
                BackendPush::ReviewCommentsChanged(worktree).emit(&app)
            }),
        }
    }
    pub(crate) fn notify(&self, worktree: &str) {
        (self.notify)(worktree);
    }
}

#[derive(Clone)]
pub(crate) struct ClientPushGateway {
    sink: Arc<PushSink>,
}

impl ClientPushGateway {
    pub(crate) fn new(sink: Arc<PushSink>) -> Self {
        Self { sink }
    }

    pub(crate) fn subscribe(&self) -> ClientPushSubscription {
        ClientPushSubscription(self.sink.subscribe())
    }
}

pub(crate) struct ClientPushSubscription(tokio::sync::broadcast::Receiver<Arc<[u8]>>);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClientPushError {
    #[error("client missed {0} pushes")]
    Lagged(u64),
    #[error("push channel closed")]
    Closed,
}

impl ClientPushSubscription {
    pub(crate) fn resubscribe(&mut self) {
        self.0 = self.0.resubscribe();
    }

    pub(crate) async fn recv(&mut self) -> Result<Arc<[u8]>, ClientPushError> {
        self.0.recv().await.map_err(|error| match error {
            tokio::sync::broadcast::error::RecvError::Lagged(count) => {
                ClientPushError::Lagged(count)
            }
            tokio::sync::broadcast::error::RecvError::Closed => ClientPushError::Closed,
        })
    }
}

#[cfg(test)]
#[path = "push_test.rs"]
mod push_tests;
