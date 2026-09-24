use super::state_subscription::SubscriptionTimer;
use crate::domain::state_subscription::client::Cursor;
use crate::domain::state_subscription::{
    connection::{StateClientError, StateClientGateway, StateConnection},
    StateValue,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;

const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) type StateReceiver =
    Arc<dyn Fn(StateValue) -> Result<(), StateClientError> + Send + Sync>;

#[derive(Clone)]
struct Subscription {
    target: String,
    receive: StateReceiver,
}

type Subscriptions = HashMap<String, Subscription>;
type Cursors = HashMap<String, Cursor>;

pub(crate) struct StateClientUsecase {
    gateway: Arc<dyn StateClientGateway>,
    subscriptions: watch::Sender<Subscriptions>,
    timer: Arc<dyn SubscriptionTimer>,
    task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StateClientUsecase {
    pub fn new(gateway: Arc<dyn StateClientGateway>, timer: Arc<dyn SubscriptionTimer>) -> Self {
        Self {
            gateway,
            timer,
            subscriptions: watch::channel(HashMap::new()).0,
            task: Default::default(),
        }
    }
    pub fn start(&self, id: String, target: String, receive: StateReceiver) {
        let mut task = self.task.lock();
        self.subscriptions.send_modify(|subscriptions| {
            subscriptions.insert(id, Subscription { target, receive });
        });
        if task.is_none() {
            *task = Some(tokio::spawn(run(
                self.gateway.clone(),
                self.timer.clone(),
                self.subscriptions.subscribe(),
            )));
        }
    }
    pub fn stop(&self, id: &str) {
        self.subscriptions.send_modify(|subscriptions| {
            subscriptions.remove(id);
        });
    }
}
impl Drop for StateClientUsecase {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

async fn run(
    gateway: Arc<dyn StateClientGateway>,
    timer: Arc<dyn SubscriptionTimer>,
    mut desired: watch::Receiver<Subscriptions>,
) {
    let mut cursors = Cursors::new();
    let mut latest = HashMap::<String, StateValue>::new();
    let mut delivered = Subscriptions::new();
    loop {
        if desired.borrow().is_empty() {
            cursors.clear();
            latest.clear();
            delivered.clear();
            if desired.changed().await.is_err() {
                return;
            }
            continue;
        }
        let connection = tokio::select! {
            changed = desired.changed() => {
                if changed.is_err() { return; }
                continue;
            }
            connection = timed(&*timer, gateway.connect()) => connection,
        };
        let result = match connection {
            Ok(mut connection) => {
                receive(
                    &mut *connection,
                    &mut desired,
                    &mut cursors,
                    &mut latest,
                    &mut delivered,
                    &*timer,
                )
                .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            log::warn!("State subscription disconnected: {error}");
            tokio::select! {
                _ = timer.sleep(Duration::from_secs(1)) => {},
                changed = desired.changed() => if changed.is_err() { return; },
            }
        }
    }
}

async fn receive(
    connection: &mut dyn StateConnection,
    desired: &mut watch::Receiver<Subscriptions>,
    cursors: &mut Cursors,
    latest: &mut HashMap<String, StateValue>,
    delivered: &mut Subscriptions,
    timer: &dyn SubscriptionTimer,
) -> Result<(), StateClientError> {
    let mut active = HashSet::<String>::new();
    loop {
        let subscriptions = desired.borrow_and_update().clone();
        let targets: HashSet<_> = subscriptions.values().map(|s| s.target.clone()).collect();
        for target in active.difference(&targets) {
            timed(timer, connection.stop(target)).await?;
        }
        active.retain(|target| targets.contains(target));
        cursors.retain(|target, _| targets.contains(target));
        latest.retain(|target, _| targets.contains(target));
        delivered.retain(|id, _| subscriptions.contains_key(id));
        for target in &targets {
            if !active.contains(target) {
                let cursor = cursors.entry(target.clone()).or_default();
                timed(timer, connection.start(target, cursor.version())).await?;
                active.insert(target.clone());
            }
        }
        for (id, subscription) in &subscriptions {
            if delivered.get(id).is_none_or(|previous| {
                previous.target != subscription.target
                    || !Arc::ptr_eq(&previous.receive, &subscription.receive)
            }) {
                if let Some(value) = latest.get(&subscription.target) {
                    (subscription.receive)(value.clone())?;
                }
                delivered.insert(id.clone(), subscription.clone());
            }
        }
        if subscriptions.is_empty() {
            return Ok(());
        }
        tokio::select! {
            changed = desired.changed() => {
                changed.map_err(|error| StateClientError(error.to_string()))?;
            }
            event = timed(timer, connection.receive()) => {
                let event = event?;
                let Some(cursor) = cursors.get_mut(&event.target) else { continue; };
                cursor.accept(event.version, event.snapshot)
                    .map_err(|error| StateClientError(error.to_string()))?;
                if let Some(value) = event.value {
                    latest.insert(event.target.clone(), value.clone());
                    for subscription in subscriptions.values().filter(|s| s.target == event.target) {
                        (subscription.receive)(value.clone())?;
                    }
                }
            }
        }
    }
}

async fn timed<T>(
    timer: &dyn SubscriptionTimer,
    future: impl std::future::Future<Output = Result<T, StateClientError>>,
) -> Result<T, StateClientError> {
    tokio::select! {
        result = future => result,
        _ = timer.sleep(TIMEOUT) => Err(StateClientError("State stream timed out".into())),
    }
}

#[cfg(test)]
#[path = "state_client_test.rs"]
mod state_client_tests;
