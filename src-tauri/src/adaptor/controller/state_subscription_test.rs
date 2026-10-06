use crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter;
use crate::usecase::state_subscription::state_subscription_test_helpers::{
    notion_target, FailingReads, GatedReads, RecordingOutput, RecordingReads,
};
use crate::usecase::state_subscription::*;
use std::sync::Arc;
struct RecordingPresenter {
    recording: Arc<RecordingOutput>,
    presenter: Arc<StateSubscriptionPresenter>,
}

impl StateSubscriptionOutput for RecordingPresenter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        self.recording
            .failures
            .lock()
            .push((target.clone(), error.to_string()));
        self.presenter.publish_failure(target, error)?;
        self.recording.updated.notify_one();
        Ok(())
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        self.presenter.publish_initial(target, snapshot.clone())?;
        self.recording.publish_initial(target, snapshot)
    }

    fn publish(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.presenter
            .publish(target, snapshot.clone(), delta.clone())?;
        self.recording.publish(target, snapshot, delta)
    }
}

#[tokio::test]
async fn test_購読外部読取_再取得失敗で古いキャッシュを配信しない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(FailingReads {
        fail_read: false.into(),
        fail_refresh: false.into(),
    });
    let tick = Arc::new(tokio::sync::Notify::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::Issues("/repo".into());
    usecase.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    usecase
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    // When
    reads
        .fail_refresh
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    // Then
    assert_eq!(output.failures.lock().len(), 1);
    assert!(output.updates.lock().is_empty());
    usecase.close_client("client");
    presenter.close("client", &usecase.active_targets());
}

#[tokio::test]
async fn test_notion購読_タスクの一覧を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    // When
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("client");
    presenter.close("client", &subscriptions.active_targets());
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_notion購読_ラベルの選択肢を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::NotionLabelOptions("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    // When
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("client");
    presenter.close("client", &subscriptions.active_targets());
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_購読駆動_時刻streamが終わっても変更通知で読み直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
            Box::pin(futures_util::stream::empty())
        })),
    )
    .with_reads(
        Arc::new(RecordingReads {
            calls: Default::default(),
        }),
        None,
        vec![],
        String::new(),
    );
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    usecase
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    // When
    usecase.notify(StateChangeSource::Repositories);
    output.updated.notified().await;
    // Then
    assert_eq!(*output.updates.lock(), vec![target]);
    usecase.close_client("client");
    presenter.close("client", &usecase.active_targets());
}

async fn assert_stopped(close: bool) {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(RecordingReads {
        calls: Default::default(),
    });
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        super::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::Issues("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    let calls = reads.calls.load(std::sync::atomic::Ordering::SeqCst);
    let snapshots = output.initial_values.lock().len() + output.update_values.lock().len();
    let failures = output.failures.lock().len();
    // When
    if close {
        subscriptions.close_client("client");
        presenter.close("client", &subscriptions.active_targets());
    } else {
        subscriptions
            .stop_subscription("client", &target, &delivery)
            .await
            .unwrap();
    }
    subscriptions.notify(StateChangeSource::Issues("/repo".into()));
    tick.notify_one();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // Then
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), calls);
    assert_eq!(
        output.initial_values.lock().len() + output.update_values.lock().len(),
        snapshots
    );
    assert_eq!(output.failures.lock().len(), failures);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_購読駆動_最終購読者の停止後は通知と周期で読取も配信もしない() {
    assert_stopped(false).await;
}

#[tokio::test]
async fn test_購読駆動_最終clientの切断後は通知と周期で読取も配信もしない() {
    assert_stopped(true).await;
}
