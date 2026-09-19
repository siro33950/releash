use std::pin::Pin;
use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

use super::application::{TerminalSurfaceApplication, TerminalSurfaceStreamItem};
use super::error::UsecaseError as ApplicationError;
use crate::domain::terminal_surface::subscriptions::{
    TerminalSubscriptionError, TerminalSubscriptions, TERMINAL_ATTACHMENT_LIMIT,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;

#[derive(Debug, thiserror::Error)]
pub(crate) enum UsecaseError {
    #[error(transparent)]
    Subscription(#[from] TerminalSubscriptionError),
    #[error(transparent)]
    Application(#[from] ApplicationError),
}

pub(crate) enum TerminalSubscriptionEvent {
    Ready,
    Item {
        attachment_id: String,
        stream_id: String,
        item: TerminalSurfaceStreamItem,
    },
    Closed {
        attachment_id: String,
        stream_id: String,
        resynchronize: bool,
    },
}
pub(crate) type TerminalStream = Pin<Box<dyn Stream<Item = TerminalSubscriptionEvent> + Send>>;

#[derive(Clone)]
pub(crate) struct TerminalSubscriptionUsecase {
    application: Arc<TerminalSurfaceApplication>,
    subscriptions: Arc<parking_lot::Mutex<TerminalSubscriptions<mpsc::Sender<TerminalStream>>>>,
}

impl TerminalSubscriptionUsecase {
    pub fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self {
            application,
            subscriptions: Default::default(),
        }
    }
    pub fn detach(&self, id: &str) {
        self.application.detach(id);
    }
    pub fn subscribe(&self, id: String) -> Result<TerminalStream, TerminalSubscriptionError> {
        let (sender, receiver) = mpsc::channel(TERMINAL_ATTACHMENT_LIMIT);
        self.subscriptions.lock().subscribe(id.clone(), sender)?;
        let subscription = Subscription {
            usecase: self.clone(),
            id,
            receiver,
            streams: futures_util::stream::SelectAll::new(),
        };
        let ready = futures_util::stream::once(async { TerminalSubscriptionEvent::Ready });
        let events = futures_util::stream::unfold(subscription, |mut subscription| async move {
            loop {
                tokio::select! {
                    incoming = subscription.receiver.recv() => subscription.streams.push(incoming?),
                    item = subscription.streams.next(), if !subscription.streams.is_empty() => {
                        if let Some(item) = item { return Some((item, subscription)); }
                    }
                }
            }
        });
        Ok(Box::pin(ready.chain(events)))
    }
    pub fn attach(
        &self,
        subscription_id: &str,
        attachment_id: String,
        stream_id: String,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), UsecaseError> {
        let sender = self.subscriptions.lock().get(subscription_id)?.clone();
        let pending = sender.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => TerminalSubscriptionError::PendingLimit,
            mpsc::error::TrySendError::Closed(_) => TerminalSubscriptionError::Ended,
        })?;
        self.subscriptions.lock().reserve_attachment()?;
        let permit = AttachmentPermit(self.clone());
        let stream = self.application.attach(&attachment_id, owner)?;
        let events = futures_util::stream::unfold(
            Some((stream, permit, attachment_id, stream_id)),
            |state| async move {
                let (mut stream, permit, attachment_id, stream_id) = state?;
                match stream.next().await {
                    Some(item) => Some((
                        TerminalSubscriptionEvent::Item {
                            attachment_id: attachment_id.clone(),
                            stream_id: stream_id.clone(),
                            item,
                        },
                        Some((stream, permit, attachment_id, stream_id)),
                    )),
                    None => Some((
                        TerminalSubscriptionEvent::Closed {
                            attachment_id,
                            stream_id,
                            resynchronize: stream.should_resynchronize(),
                        },
                        None,
                    )),
                }
            },
        );
        pending.send(Box::pin(events));
        Ok(())
    }
}

struct Subscription {
    usecase: TerminalSubscriptionUsecase,
    id: String,
    receiver: mpsc::Receiver<TerminalStream>,
    streams: futures_util::stream::SelectAll<TerminalStream>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.usecase.subscriptions.lock().unsubscribe(&self.id);
    }
}
struct AttachmentPermit(TerminalSubscriptionUsecase);
impl Drop for AttachmentPermit {
    fn drop(&mut self) {
        self.0.subscriptions.lock().release_attachment();
    }
}

#[cfg(test)]
#[path = "subscriptions_test.rs"]
mod subscriptions_tests;
