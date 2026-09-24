use std::sync::Arc;

use crate::adaptor::gateway::repository::watch::{FileChangeEvent, GitStatusChangedEvent};
use crate::adaptor::protocol::workflow::WorkflowExecutionChangedPayloadView;
use crate::infrastructure::push::PushSink;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionChangedPayload<'a> {
    pub worktree_path: &'a str,
}

pub enum BackendPush<'a> {
    AgentSessionChanged(AgentSessionChangedPayload<'a>),
    BranchListSync,
    WorkspaceListChanged,
    FileChange(FileChangeEvent),
    GitStatusChanged(GitStatusChangedEvent),
    ReviewCommentsChanged(&'a str),
    WorkflowExecutionChanged(Box<WorkflowExecutionChangedPayloadView>),
}

impl BackendPush<'_> {
    pub(crate) fn emit(self, sink: &PushSink) {
        use crate::adaptor::protocol::client as wire;
        use prost::Message;
        macro_rules! publish {
            ($name:literal, $variant:ident, $value:expr) => {{
                let event = $value.map(wire::push::Event::$variant);
                match event {
                    Ok(event) => sink.send(wire::Push { event: Some(event) }.encode_to_vec()),
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
            Self::WorkspaceListChanged => publish!(
                "workspace-list-changed",
                WorkspaceListChanged,
                Ok::<_, String>(wire::Unit {})
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
    pub(crate) fn new(sink: std::sync::Arc<crate::infrastructure::push::PushSink>) -> Self {
        Self {
            notify: Box::new(move |worktree| {
                BackendPush::ReviewCommentsChanged(worktree).emit(&sink)
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

use crate::usecase::agent_session::AgentSessionChangeNotifier;

pub(crate) struct ClientAgentSessionChangeNotifier {
    sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
}

impl ClientAgentSessionChangeNotifier {
    pub(crate) fn new(sink: std::sync::Arc<crate::infrastructure::push::PushSink>) -> Self {
        Self { sink }
    }
}

impl AgentSessionChangeNotifier for ClientAgentSessionChangeNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        BackendPush::AgentSessionChanged(AgentSessionChangedPayload { worktree_path })
            .emit(&self.sink);
    }
}
