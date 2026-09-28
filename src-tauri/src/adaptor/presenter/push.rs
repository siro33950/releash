use std::sync::Arc;

use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect_wire::rpc;
use crate::infrastructure::push::PushSink;
use crate::usecase::push::{PushMessage, PushOutput};
use prost::Message;

impl PushOutput for PushSink {
    fn publish(&self, message: PushMessage) {
        let event = match message {
            PushMessage::FileChange {
                watcher_id,
                path,
                kind,
            } => wire::push::Event::FileChange(wire::FileChangeEvent {
                watcher_id: Some(watcher_id),
                path: Some(path),
                kind: Some(kind),
            }),
        };
        self.send(wire::Push { event: Some(event) }.encode_to_vec());
    }
}

pub(crate) fn resync() -> Arc<[u8]> {
    wire::Push {
        event: Some(wire::push::Event::Resync(wire::Unit {})),
    }
    .encode_to_vec()
    .into()
}

pub(crate) struct EncodedPush(pub(crate) Arc<[u8]>);

impl connectrpc::Encodable<rpc::Push> for EncodedPush {
    fn encode(
        &self,
        codec: connectrpc::CodecFormat,
    ) -> Result<axum::body::Bytes, connectrpc::ConnectError> {
        match codec {
            connectrpc::CodecFormat::Proto => Ok(axum::body::Bytes::from_owner(self.0.clone())),
            _ => {
                let push =
                    <rpc::Push as buffa::Message>::decode_from_slice(&self.0).map_err(|error| {
                        crate::adaptor::presenter::connect::classified_error(
                            crate::adaptor::presenter::error::AppError::new(error.to_string()),
                        )
                    })?;
                push.encode(codec)
            }
        }
    }
}

use crate::usecase::agent_session::AgentSessionChangeNotifier;

pub(crate) struct ClientAgentSessionChangeNotifier {
    publisher: crate::usecase::state_subscription::StateSubscriptionOutputRef,
}

impl ClientAgentSessionChangeNotifier {
    pub(crate) fn new(
        publisher: crate::usecase::state_subscription::StateSubscriptionOutputRef,
    ) -> Self {
        Self { publisher }
    }
}

impl AgentSessionChangeNotifier for ClientAgentSessionChangeNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.publisher.invalidate(
            crate::usecase::state_subscription::StateChangeSource::Worktree(worktree_path.into()),
        );
    }
}

#[cfg(test)]
#[path = "push_test.rs"]
mod push_tests;
