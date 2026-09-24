use std::sync::Arc;

use crate::adaptor::gateway::repository::watch::{FileChangeEvent, GitStatusChangedEvent};
use crate::infrastructure::push::PushSink;

pub enum BackendPush<'a> {
    FileChange(FileChangeEvent),
    GitStatusChanged(GitStatusChangedEvent),
    ReviewCommentsChanged(&'a str),
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
    publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
}

impl ClientAgentSessionChangeNotifier {
    pub(crate) fn new(
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) -> Self {
        Self { publisher }
    }
}

impl AgentSessionChangeNotifier for ClientAgentSessionChangeNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.publisher.invalidate(
            crate::domain::state_subscription::StateChangeSource::Worktree(worktree_path.into()),
        );
    }
}
