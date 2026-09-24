use crate::domain::state_subscription::{
    Delivery, Event, StateValue, SubscriptionError, Subscriptions, Version,
};
use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::Notify;

pub(crate) const REPO_PATHS: &str = "repository-paths";
pub(crate) const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) trait SubscriptionTimer: Send + Sync {
    #[cfg(feature = "desktop")]
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
    fn interval(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>>;
}

pub(crate) enum StateSubscriptionEvent {
    Ready,
    Item(String, Event<StateValue>),
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionUsecase {
    publisher: StateSubscriptionPublisher,
    timer: Arc<dyn SubscriptionTimer>,
}

impl StateSubscriptionUsecase {
    pub fn new(paths: Vec<String>, timer: Arc<dyn SubscriptionTimer>) -> Self {
        let mut state = Subscriptions::new(uuid::Uuid::new_v4().to_string());
        state
            .register(
                REPO_PATHS.into(),
                StateValue::RepositoryPaths(paths),
                Delivery::Full,
            )
            .expect("unique target");
        Self {
            publisher: StateSubscriptionPublisher {
                state: Arc::new(Mutex::new(state)),
                changed: Arc::new(Notify::new()),
            },
            timer,
        }
    }

    pub fn publisher(&self) -> StateSubscriptionPublisher {
        self.publisher.clone()
    }

    pub fn start(
        &self,
        client: &str,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        self.publisher
            .update(|state| state.start(client, target, version))
    }

    pub fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        self.publisher.update(|state| state.stop(client, target))
    }

    pub fn open(
        &self,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        self.publisher.state.lock().open(id.clone())?;
        let permit = StreamPermit {
            usecase: self.clone(),
            id,
        };
        let timer = self.timer.interval(BOOKMARK_INTERVAL);
        let events = futures_util::stream::unfold(
            (permit, timer),
            |(permit, mut timer)| async move {
                loop {
                    let notify = permit.usecase.publisher.changed.clone();
                    let changed = notify.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let item = permit.usecase.publisher.state.lock().next(&permit.id);
                    if let Some((target, event)) = item {
                        return Some((
                            StateSubscriptionEvent::Item(target, event),
                            (permit, timer),
                        ));
                    }
                    tokio::select! {
                        _ = changed => {},
                        _ = timer.next() => permit.usecase.publisher.state.lock().bookmark(&permit.id),
                    }
                }
            },
        );
        Ok(futures_util::stream::once(async { StateSubscriptionEvent::Ready }).chain(events))
    }
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionPublisher {
    state: Arc<Mutex<Subscriptions<StateValue>>>,
    changed: Arc<Notify>,
}

impl StateSubscriptionPublisher {
    fn update(
        &self,
        update: impl FnOnce(&mut Subscriptions<StateValue>) -> Result<(), SubscriptionError>,
    ) -> Result<(), SubscriptionError> {
        update(&mut self.state.lock())?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn publish(
        &self,
        target: &str,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.update(|state| state.publish(target, snapshot, delta))
    }
}

struct StreamPermit {
    usecase: StateSubscriptionUsecase,
    id: String,
}
impl Drop for StreamPermit {
    fn drop(&mut self) {
        self.usecase.publisher.state.lock().close(&self.id);
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
