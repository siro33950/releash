use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::domain::client_operation::registry::{
    OperationDecision, OperationIdentity, OperationReference, OperationRegistry, RecoveryAttempt,
};

type ReleaseWatch = Arc<dyn Fn(u64) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub(crate) struct OperationCompletion<R> {
    pub result: R,
    pub started_watch: Option<u64>,
    pub stopped_watch: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct ClientOperationUsecase<R> {
    pub instance_id: Arc<str>,
    registry: Arc<Mutex<OperationRegistry<R>>>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    release_watch: ReleaseWatch,
    save_tail: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
}

impl<R: Clone + Send + 'static> ClientOperationUsecase<R> {
    pub fn new(
        instance_id: String,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        release_watch: ReleaseWatch,
    ) -> Self {
        Self {
            instance_id: Arc::from(instance_id.clone()),
            registry: Arc::new(Mutex::new(OperationRegistry::new(instance_id))),
            clock,
            release_watch,
            save_tail: Arc::new(Mutex::new(None)),
        }
    }

    fn registry(&self) -> MutexGuard<'_, OperationRegistry<R>> {
        let mut registry = self.registry.lock().unwrap();
        registry.advance((self.clock)());
        registry
    }

    pub fn prepare(
        &self,
        id: &str,
        generation: &str,
        identity: &OperationIdentity,
        predecessors: &[OperationReference],
        attempt: &RecoveryAttempt<'_>,
    ) -> OperationDecision<R> {
        self.registry()
            .prepare(id, generation, identity, predecessors, attempt)
    }

    pub fn query(&self, id: &str, generation: &str) -> OperationDecision<R> {
        self.registry().query(id, generation)
    }

    pub fn execute<F: Future<Output = OperationCompletion<R>> + Send + 'static>(
        &self,
        id: String,
        generation: &str,
        identity: OperationIdentity,
        predecessors: &[OperationReference],
        attempt: &RecoveryAttempt<'_>,
        run: impl FnOnce() -> F,
    ) -> Pin<Box<dyn Future<Output = OperationDecision<R>> + Send>> {
        let save = identity.command == "save_workspace_state";
        let admission = self
            .registry()
            .admit(&id, generation, identity, predecessors, attempt);
        let (previous, done) = if save && matches!(admission, OperationDecision::Ready) {
            let (done, next) = tokio::sync::oneshot::channel();
            (self.save_tail.lock().unwrap().replace(next), Some(done))
        } else {
            (None, None)
        };
        let run = matches!(admission, OperationDecision::Ready).then(run);
        let operations = self.clone();
        Box::pin(async move {
            if !matches!(admission, OperationDecision::Ready) {
                return admission;
            }
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let completion = run.expect("admitted execution").await;
            let release = {
                let mut registry = operations.registry();
                if let Some(id) = completion.stopped_watch {
                    registry.forget_watch(id);
                }
                registry.complete(&id, &completion.result, completion.started_watch)
            };
            if let Some(id) = release {
                (operations.release_watch)(id).await;
            }
            operations.maintain().await;
            drop(done);
            OperationDecision::Completed(completion.result)
        })
    }

    pub async fn maintain(&self) {
        let release = self.registry().take_released_watches();
        for id in release {
            (self.release_watch)(id).await;
        }
    }

    pub async fn acknowledge(&self, id: &str, release_watch: bool) -> bool {
        let release = self.registry().acknowledge(id, release_watch);
        if let Some(id) = release {
            (self.release_watch)(id).await;
        }
        self.registry().watch_active(id)
    }

    pub async fn disconnect(&self, connection: &str) {
        let release = self.registry().disconnect(connection);
        for id in release {
            (self.release_watch)(id).await;
        }
    }
}

#[cfg(test)]
#[path = "client_operation_test.rs"]
mod client_operation_tests;
