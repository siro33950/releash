use super::*;

#[derive(Default)]
pub struct RecordingOutput {
    pub(crate) initial: Mutex<Vec<SubscriptionTarget>>,
    pub initial_values: Mutex<Vec<StateValue>>,
    pub update_values: Mutex<Vec<StateValue>>,
    pub failures: Mutex<Vec<(SubscriptionTarget, String)>>,
    pub(crate) fail_initial: std::sync::atomic::AtomicBool,
    pub(crate) updates: Mutex<Vec<SubscriptionTarget>>,
    pub updated: tokio::sync::Notify,
}

impl StateSubscriptionOutput for RecordingOutput {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        self.failures
            .lock()
            .push((target.clone(), error.to_string()));
        self.updated.notify_one();
        Ok(())
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        if self.fail_initial.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SubscriptionError::EncodingFailed);
        }
        self.initial.lock().push(target.clone());
        self.initial_values.lock().push(snapshot);
        Ok(())
    }

    fn publish(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
        _: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.updates.lock().push(target.clone());
        self.update_values.lock().push(snapshot);
        self.updated.notify_one();
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct RecordingReads {
    pub(crate) calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
#[cfg(test)]
impl StateSubscriptionRead for RecordingReads {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(match target {
            SubscriptionTarget::RepositoryPaths => {
                StateValue::RepositoryPaths(vec!["/repo".into()])
            }
            _ => StateValue::NodeDetail(None),
        })
    }

    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[derive(Default)]
#[cfg(test)]
pub(crate) struct GatedReads {
    pub(crate) reads: std::sync::atomic::AtomicUsize,
    pub(crate) external: std::sync::atomic::AtomicUsize,
    pub(crate) blocked: tokio::sync::Notify,
    pub(crate) release: tokio::sync::Notify,
}

#[cfg(test)]
impl GatedReads {
    pub(crate) fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(crate) fn external(&self) -> usize {
        self.external.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
#[cfg(test)]
impl StateSubscriptionRead for GatedReads {
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            self.blocked.notify_one();
            self.release.notified().await;
        }
        Ok(StateValue::NodeDetail(None))
    }

    async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
        self.external
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
pub(crate) struct FailingReads {
    pub(crate) fail_read: std::sync::atomic::AtomicBool,
    pub(crate) fail_refresh: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
#[cfg(test)]
impl StateSubscriptionRead for FailingReads {
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        if self.fail_read.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StateReadError::from_error(SubscriptionError::UnknownTarget));
        }
        Ok(StateValue::RepositoryPaths(vec!["/repo".into()]))
    }
    async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
        if self.fail_refresh.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StateReadError::from_error(
                SubscriptionError::EncodingFailed,
            ));
        }
        Ok(())
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

pub fn notion_target() -> SubscriptionTarget {
    SubscriptionTarget::NotionTasks(crate::usecase::notion::usecase::NotionTaskListRequest {
        path: "/repo".into(),
        count: 20,
        title: None,
        labels: Default::default(),
    })
}

pub struct FakeDelivery;

impl StateSubscriptionDelivery for FakeDelivery {
    fn start(&self) -> Result<Option<usize>, StateReadError> {
        Ok(None)
    }
    fn claim(&self) -> bool {
        true
    }
    fn finish(
        &self,
        _: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError> {
        Ok(())
    }
}
