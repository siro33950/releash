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
    FileChange(&'a FileChangeEvent),
    GitStatusChanged(&'a GitStatusChangedEvent),
    RepoPathsChanged(&'a [String]),
    RepositorySnapshotChanged(&'a RepositorySnapshotChangedEvent),
    ReviewCommentsChanged(&'a str),
    WorkflowExecutionChanged(&'a WorkflowExecutionChangedPayloadView),
}

impl BackendPush<'_> {
    pub fn emit<R: tauri::Runtime>(self, app: &tauri::AppHandle<R>) {
        let sink = app.state::<Arc<PushSink>>();
        match self {
            Self::AgentSessionChanged(payload) => sink.emit(app, "agent-session-changed", payload),
            Self::BranchListSync => sink.emit(app, "branch-list-sync", ()),
            Self::FileChange(payload) => sink.emit(app, "file-change", payload),
            Self::GitStatusChanged(payload) => sink.emit(app, "git-status-changed", payload),
            Self::RepoPathsChanged(payload) => sink.emit(app, "repo-paths-changed", payload),
            Self::RepositorySnapshotChanged(payload) => {
                sink.emit(app, "repository-snapshot-changed", payload)
            }
            Self::ReviewCommentsChanged(payload) => {
                sink.emit(app, "review-comments-changed", payload)
            }
            Self::WorkflowExecutionChanged(payload) => {
                sink.emit(app, "workflow-execution-changed", payload)
            }
        }
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

pub(crate) struct ClientPushSubscription(tokio::sync::broadcast::Receiver<Arc<str>>);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClientPushError {
    #[error("client missed {0} pushes")]
    Lagged(u64),
    #[error("push channel closed")]
    Closed,
}

impl ClientPushSubscription {
    pub(crate) async fn recv(&mut self) -> Result<Arc<str>, ClientPushError> {
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
