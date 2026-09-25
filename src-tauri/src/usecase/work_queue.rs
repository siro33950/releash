use crate::common::retry::{RetryBackoff, RetryBucket};
use crate::domain::failure::{ClassifiedFailure, FailureKind, RetryAction};
use crate::domain::failure_records::FailureRecord;
use crate::domain::work_queue::WorkQueue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

type Task = Pin<Box<dyn Future<Output = ()> + Send>>;
pub type Attempt<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Duration>, WorkFailure>> + Send + 'a>>;
pub type Job = Arc<dyn Fn(RetryAction) -> Attempt<'static> + Send + Sync>;

#[async_trait::async_trait]
pub trait WorkQueueRuntime: Send + Sync {
    fn now(&self) -> Duration;
    fn timestamp_ms(&self) -> u64;
    fn jitter(&self) -> f64;
    fn spawn(&self, task: Task);
    async fn sleep(&self, duration: Duration);
    async fn attempt(&self, attempt: Attempt<'_>) -> Result<Option<Duration>, WorkFailure>;
}

#[derive(Debug, Clone)]
pub struct WorkFailure {
    pub kind: FailureKind,
    pub message: String,
}
impl WorkFailure {
    pub fn from_error(error: &(impl ClassifiedFailure + std::fmt::Debug)) -> Self {
        Self {
            kind: error.failure_kind(),
            message: format!("{error:?}"),
        }
    }
}
impl std::fmt::Display for WorkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl ClassifiedFailure for WorkFailure {
    fn failure_kind(&self) -> FailureKind {
        self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkKey {
    pub operation: String,
    pub target: String,
    stage: bool,
}
impl WorkKey {
    pub fn new(operation: &str, target: &str) -> Self {
        Self {
            operation: operation.into(),
            target: target.into(),
            stage: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureObservation {
    pub record: FailureRecord,
    pub requires_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePage {
    pub items: Vec<FailureObservation>,
    pub next_offset: Option<usize>,
    pub requires_attention: bool,
}

#[derive(Clone, Copy)]
enum AttemptMode {
    Timed,
    Changed,
    Borrowed { record_failures: bool },
}

struct Entry {
    job: Job,
    backoff: RetryBackoff,
    action: RetryAction,
    retrying: bool,
    mode: AttemptMode,
    completed: Option<tokio::sync::oneshot::Sender<Result<(), WorkFailure>>>,
}
struct State {
    queue: WorkQueue<WorkKey>,
    jobs: HashMap<WorkKey, Entry>,
}

pub struct WorkQueueUsecase {
    runtime: Arc<dyn WorkQueueRuntime>,
    state: std::sync::Mutex<State>,
    bucket: Arc<Mutex<RetryBucket>>,
    query: Arc<super::failure_query_service::FailureQueryService>,
    changed: Notify,
    publisher:
        std::sync::Mutex<Option<crate::usecase::state_subscription::StateSubscriptionPublisher>>,
}

#[cfg(test)]
pub fn shared() -> &'static Arc<WorkQueueUsecase> {
    static SHARED: std::sync::OnceLock<Arc<WorkQueueUsecase>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| WorkQueueUsecase::new(super::work_queue_test_runtime::runtime()))
}

impl WorkQueueUsecase {
    #[cfg(test)]
    pub fn new(runtime: Arc<dyn WorkQueueRuntime>) -> Arc<Self> {
        let bucket = Arc::new(Mutex::new(RetryBucket::new(runtime.now())));
        Self::with_retry_bucket(runtime, bucket)
    }

    pub fn with_retry_bucket(
        runtime: Arc<dyn WorkQueueRuntime>,
        bucket: Arc<Mutex<RetryBucket>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bucket,
            runtime,
            state: std::sync::Mutex::new(State {
                queue: WorkQueue::default(),
                jobs: HashMap::new(),
            }),
            query: Arc::new(super::failure_query_service::FailureQueryService::new()),
            changed: Notify::new(),
            publisher: Default::default(),
        })
    }

    pub(crate) async fn attempt<T: Send>(
        &self,
        operation: impl Future<Output = T> + Send,
    ) -> Result<T, WorkFailure> {
        let mut value = None;
        self.runtime
            .attempt(Box::pin(async {
                value = Some(operation.await);
                Ok(None)
            }))
            .await?;
        Ok(value.expect("completed attempt"))
    }

    pub fn spawn(&self, task: Task) {
        self.runtime.spawn(task);
    }

    pub async fn enqueue(self: &Arc<Self>, key: WorkKey, backoff: RetryBackoff, job: Job) {
        self.enqueue_after(key, backoff, job, Duration::ZERO).await;
    }

    pub async fn enqueue_after(
        self: &Arc<Self>,
        key: WorkKey,
        backoff: RetryBackoff,
        job: Job,
        delay: Duration,
    ) {
        self.enqueue_with_completion(key, backoff, job, None, delay, AttemptMode::Timed)
            .await;
    }

    pub async fn enqueue_changed_after(
        self: &Arc<Self>,
        key: WorkKey,
        backoff: RetryBackoff,
        job: Job,
        delay: Duration,
    ) {
        self.enqueue_with_completion(key, backoff, job, None, delay, AttemptMode::Changed)
            .await;
    }

    async fn enqueue_with_completion(
        self: &Arc<Self>,
        key: WorkKey,
        backoff: RetryBackoff,
        job: Job,
        mut completed: Option<tokio::sync::oneshot::Sender<Result<(), WorkFailure>>>,
        delay: Duration,
        mode: AttemptMode,
    ) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let first = {
                let mut state = self.state.lock().expect("work queue state");
                if matches!(mode, AttemptMode::Borrowed { .. }) && state.jobs.contains_key(&key) {
                    None
                } else {
                    if state.jobs.contains_key(&key) {
                        if let Some(completed) = completed.take() {
                            let _ = completed.send(Err(WorkFailure {
                                kind: FailureKind::AlreadyPresent,
                                message: "同じ対象の処理が実行中です".into(),
                            }));
                            return;
                        }
                    }
                    let first = state.jobs.is_empty();
                    if completed.is_some() || matches!(mode, AttemptMode::Changed) {
                        state.queue.reset(&key);
                    }
                    if !state.queue.add(key.clone(), self.runtime.now() + delay) {
                        return;
                    }
                    state.jobs.entry(key.clone()).or_insert_with(|| Entry {
                        job: job.clone(),
                        backoff,
                        action: RetryAction::Retry,
                        retrying: false,
                        mode,
                        completed: completed.take(),
                    });
                    Some(first)
                }
            };
            if let Some(first) = first {
                self.changed.notify_waiters();
                if first {
                    let queue = self.clone();
                    self.runtime
                        .spawn(Box::pin(async move { queue.dispatch().await }));
                }
                return;
            }
            changed.await;
        }
    }

    async fn dispatch(self: Arc<Self>) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (work, next) = {
                let mut state = self.state.lock().expect("work queue state");
                if state.jobs.is_empty() {
                    return;
                }
                let State { queue, jobs, .. } = &mut *state;
                let work = queue
                    .get_matching(self.runtime.now(), |key| jobs.contains_key(key))
                    .map(|key| {
                        let entry = &jobs[&key];
                        (
                            key,
                            entry.job.clone(),
                            entry.action,
                            entry.backoff,
                            entry.retrying,
                            entry.mode,
                        )
                    });
                let next = queue.next_due_matching(|key| jobs.contains_key(key));
                (work, next)
            };
            if let Some((key, job, action, backoff, retrying, mode)) = work {
                let queue = self.clone();
                self.runtime.spawn(Box::pin(async move {
                    if retrying {
                        queue.acquire_retry().await;
                    }
                    let result = if matches!(mode, AttemptMode::Borrowed { .. }) {
                        job(action).await
                    } else {
                        queue.runtime.attempt(job(action)).await
                    };
                    let next = match &result {
                        Ok(delay) => {
                            queue.clear_attention(&key).await;
                            delay.map(|delay| queue.runtime.now() + delay)
                        }
                        Err(error) => {
                            if !matches!(
                                mode,
                                AttemptMode::Borrowed {
                                    record_failures: false
                                }
                            ) && (!key.stage || error.kind.retry_action() == RetryAction::Retry)
                            {
                                queue.observe(&key, error).await;
                            }
                            None
                        }
                    };
                    let mut state = queue.state.lock().expect("work queue state");
                    if !state
                        .jobs
                        .get(&key)
                        .is_some_and(|entry| Arc::ptr_eq(&entry.job, &job))
                    {
                        return;
                    }
                    match result {
                        Err(error)
                            if error.kind.retry_action() != RetryAction::Stop
                                && (!key.stage
                                    || error.kind.retry_action() == RetryAction::Retry) =>
                        {
                            let due = retry_due(
                                &mut state.queue,
                                &key,
                                error.kind,
                                backoff,
                                queue.runtime.now(),
                                queue.runtime.jitter(),
                            );
                            let entry = state.jobs.get_mut(&key).expect("processing job");
                            entry.action = error.kind.retry_action();
                            entry.retrying = true;
                            state.queue.done(&key, Some(due), false);
                        }
                        result => {
                            let stopped = result.is_err();
                            if let Some(completed) = state
                                .jobs
                                .get_mut(&key)
                                .and_then(|entry| entry.completed.take())
                            {
                                let _ = completed.send(result.map(|_| ()));
                            }
                            let pending = if stopped && !matches!(mode, AttemptMode::Changed) {
                                state.queue.stop(&key);
                                false
                            } else {
                                state.queue.done(&key, next, true)
                            };
                            if !pending {
                                state.jobs.remove(&key);
                            } else {
                                state.jobs.get_mut(&key).expect("pending job").retrying = false;
                            }
                        }
                    }
                    drop(state);
                    queue.changed.notify_waiters();
                }));
                continue;
            }
            if let Some(next) = next {
                tokio::select! {
                    _ = notified => {},
                    _ = self.runtime.sleep(next.saturating_sub(self.runtime.now())) => {},
                }
            } else {
                notified.await;
            }
        }
    }

    pub async fn execute<T, F, Fut>(
        self: &Arc<Self>,
        key: WorkKey,
        backoff: RetryBackoff,
        operation: F,
    ) -> Result<T, WorkFailure>
    where
        T: Send + 'static,
        F: Fn(RetryAction) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, WorkFailure>> + Send + 'static,
    {
        let value = Arc::new(Mutex::new(None));
        let result = value.clone();
        let (completed, completion) = tokio::sync::oneshot::channel();
        self.enqueue_with_completion(
            key,
            backoff,
            Arc::new(move |action| {
                let attempt = operation(action);
                let value = value.clone();
                Box::pin(async move {
                    *value.lock().await = Some(attempt.await?);
                    Ok(None)
                })
            }),
            Some(completed),
            Duration::ZERO,
            AttemptMode::Timed,
        )
        .await;
        completion.await.map_err(|_| WorkFailure {
            kind: FailureKind::Cancelled,
            message: "作業列が終了しました".into(),
        })??;
        let value = result
            .lock()
            .await
            .take()
            .expect("successful attempt value");
        Ok(value)
    }

    pub async fn observe(&self, key: &WorkKey, error: &WorkFailure) {
        let record_attention_changed = self.query.records.lock().await.observe(
            &key.operation,
            &key.target,
            error.kind,
            error.message.clone(),
            self.runtime.timestamp_ms(),
        );
        let attention_changed =
            if let Some(state) = self.query.target_failures.get(key.operation.as_str()) {
                state.lock().await.observe(&key.target, error.kind)
            } else {
                record_attention_changed
            };
        self.publish_failure_change(key, attention_changed);
    }

    pub(crate) fn set_publisher(
        &self,
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) {
        *self.publisher.lock().expect("failure publisher") = Some(publisher);
    }

    #[cfg(test)]
    pub async fn records(&self, target: &str) -> Vec<FailureObservation> {
        self.query.records(target).await
    }

    pub async fn records_page(&self, target: &str, offset: usize) -> FailurePage {
        self.query.records_page(target, offset).await
    }

    pub(crate) fn failure_query(&self) -> Arc<super::failure_query_service::FailureQueryService> {
        self.query.clone()
    }

    async fn clear_attention(&self, key: &WorkKey) {
        let record_changed = self
            .query
            .records
            .lock()
            .await
            .resolve(&key.operation, &key.target);
        let attention_changed =
            if let Some(state) = self.query.target_failures.get(key.operation.as_str()) {
                state.lock().await.clear(&key.target)
            } else {
                record_changed
            };
        self.publish_failure_change(key, attention_changed);
    }

    fn publish_failure_change(&self, key: &WorkKey, attention_changed: bool) {
        if let Some(publisher) = self.publisher.lock().expect("failure publisher").as_ref() {
            if attention_changed && key.operation.starts_with("workflow_") {
                publisher.invalidate(
                    crate::domain::state_subscription::StateChangeSource::WorkspaceList,
                );
            }
            publisher.invalidate(
                crate::domain::state_subscription::StateChangeSource::Failures(key.target.clone()),
            );
        }
    }

    pub async fn acquire_retry(&self) {
        loop {
            let wait = self.bucket.lock().await.acquire(self.runtime.now());
            if wait.is_zero() {
                return;
            }
            self.runtime.sleep(wait).await;
        }
    }
}

pub async fn retry<T, E, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    key: WorkKey,
    policy: RetryBackoff,
    operation: F,
) -> Result<T, E>
where
    E: ClassifiedFailure + std::fmt::Debug,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    retry_with_scope(queue, key, policy, operation, true).await
}

pub async fn retry_stage<T, E, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    key: WorkKey,
    policy: RetryBackoff,
    operation: F,
) -> Result<T, E>
where
    E: ClassifiedFailure + std::fmt::Debug,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    retry_with_scope(queue, key, policy, operation, false).await
}

pub(crate) async fn retry_with_scope<T, E, F, Fut>(
    queue: &Arc<WorkQueueUsecase>,
    key: WorkKey,
    policy: RetryBackoff,
    mut operation: F,
    restart: bool,
) -> Result<T, E>
where
    E: ClassifiedFailure + std::fmt::Debug,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    run_borrowed(queue, key, policy, |_| operation(), restart, true).await
}

pub(crate) async fn run_borrowed<T, E, F, Fut>(
    queue: &Arc<WorkQueueUsecase>,
    key: WorkKey,
    policy: RetryBackoff,
    operation: F,
    restart: bool,
    record_failures: bool,
) -> Result<T, E>
where
    E: ClassifiedFailure + std::fmt::Debug,
    F: FnMut(RetryAction) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let (_work, attempts, completion) = queue.borrowed(key, policy, restart, record_failures).await;
    crate::common::retry::requested(attempts, completion, operation, |value| {
        value
            .as_ref()
            .map(|_| None)
            .map_err(WorkFailure::from_error)
    })
    .await
}

pub(crate) struct BorrowedWork {
    queue: Arc<WorkQueueUsecase>,
    key: WorkKey,
    job: Job,
}

impl Drop for BorrowedWork {
    fn drop(&mut self) {
        let mut state = self.queue.state.lock().expect("work queue state");
        if state
            .jobs
            .get(&self.key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.job, &self.job))
        {
            state.jobs.remove(&self.key);
            state.queue.remove(&self.key);
        }
        drop(state);
        self.queue.changed.notify_waiters();
    }
}

fn cancelled() -> WorkFailure {
    WorkFailure {
        kind: FailureKind::Cancelled,
        message: "作業の呼び出し元が終了しました".into(),
    }
}

#[cfg(test)]
pub(crate) struct ImmediateWorkQueueRuntime {
    now: std::sync::atomic::AtomicU64,
}
#[cfg(test)]
impl Default for ImmediateWorkQueueRuntime {
    fn default() -> Self {
        Self {
            now: Default::default(),
        }
    }
}
#[cfg(test)]
#[async_trait::async_trait]
impl WorkQueueRuntime for ImmediateWorkQueueRuntime {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.now.load(std::sync::atomic::Ordering::SeqCst))
    }
    fn timestamp_ms(&self) -> u64 {
        self.now().as_millis() as u64
    }
    fn jitter(&self) -> f64 {
        1.0
    }
    fn spawn(&self, task: Task) {
        tokio::spawn(task);
    }
    async fn sleep(&self, duration: Duration) {
        self.now.fetch_add(
            duration.as_nanos().min(u64::MAX as u128) as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
        tokio::task::yield_now().await;
    }
    async fn attempt(&self, attempt: Attempt<'_>) -> Result<Option<Duration>, WorkFailure> {
        attempt.await
    }
}

#[cfg(test)]
#[path = "work_queue_test.rs"]
pub(crate) mod work_queue_tests;

fn retry_due<K: Eq + std::hash::Hash + Clone>(
    queue: &mut WorkQueue<K>,
    key: &K,
    kind: FailureKind,
    policy: RetryBackoff,
    now: Duration,
    jitter: f64,
) -> Duration {
    let policy = if kind.retry_action() == RetryAction::Restart {
        RetryBackoff::CONFLICT
    } else {
        policy
    };
    now + policy.delay(queue.failed(key), jitter)
}

type BorrowedRequest = (
    RetryAction,
    tokio::sync::oneshot::Sender<Result<Option<Duration>, WorkFailure>>,
);
impl WorkQueueUsecase {
    pub(crate) async fn borrowed(
        self: &Arc<Self>,
        mut key: WorkKey,
        policy: RetryBackoff,
        restart: bool,
        record_failures: bool,
    ) -> (
        BorrowedWork,
        tokio::sync::mpsc::UnboundedReceiver<BorrowedRequest>,
        tokio::sync::oneshot::Receiver<Result<(), WorkFailure>>,
    ) {
        key.stage = !restart;
        let (requests, attempts) = tokio::sync::mpsc::unbounded_channel();
        let (completed, completion) = tokio::sync::oneshot::channel();
        let job: Job = Arc::new(move |action| {
            let (reply, result) = tokio::sync::oneshot::channel();
            let sent = requests.send((action, reply));
            Box::pin(async move {
                if sent.is_err() {
                    return Err(cancelled());
                }
                result.await.unwrap_or_else(|_| Err(cancelled()))
            })
        });
        self.enqueue_with_completion(
            key.clone(),
            policy,
            job.clone(),
            Some(completed),
            Duration::ZERO,
            AttemptMode::Borrowed { record_failures },
        )
        .await;
        let work = BorrowedWork {
            queue: self.clone(),
            key,
            job,
        };
        (work, attempts, completion)
    }
}
