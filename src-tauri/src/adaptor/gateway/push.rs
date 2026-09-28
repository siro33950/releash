use std::sync::Arc;

use crate::adaptor::gateway::repository::watch::FileChangeEvent;
use crate::infrastructure::push::PushSink;
use crate::usecase::push::{PushMessage, PushOutput};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionOutputRef};

pub enum BackendPush {
    FileChange(FileChangeEvent),
}

impl BackendPush {
    pub(crate) fn emit(self, output: &dyn PushOutput) {
        output.publish(match self {
            Self::FileChange(value) => PushMessage::FileChange {
                watcher_id: value.watcher_id,
                path: value.path,
                kind: value.kind,
            },
        });
    }
}

pub(crate) struct CommentChangeGateway {
    publisher: StateSubscriptionOutputRef,
}
impl CommentChangeGateway {
    pub(crate) fn new(publisher: StateSubscriptionOutputRef) -> Self {
        Self { publisher }
    }
    pub(crate) fn notify(&self, worktree: &str) {
        self.publisher
            .invalidate(StateChangeSource::ReviewComments(Some(worktree.into())));
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
