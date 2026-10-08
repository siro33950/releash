use releashd::test_support::integration::subscriptions::notion_target;
use releashd::test_support::integration::subscriptions::FakeDelivery;
use releashd::test_support::integration::subscriptions::RecordingOutput;
use releashd::test_support::integration::subscriptions::StateChangeSource;
use releashd::test_support::integration::subscriptions::StateReadError;
use releashd::test_support::integration::subscriptions::StateSubscriptionRead;
use releashd::test_support::integration::subscriptions::StateSubscriptionUsecase;
use releashd::test_support::integration::subscriptions::StateValue;
use releashd::test_support::integration::subscriptions::SubscriptionTarget;
use releashd::test_support::integration::subscriptions::WorkspaceStateReads;
use std::sync::Arc;

#[derive(Default)]
struct ReopeningNotionApi(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl releashd::test_support::integration::platform::NotionApiGateway for ReopeningNotionApi {
    async fn query_tasks(
        &self,
        _: &releashd::test_support::integration::settings::NotionRepoConfig,
        _: &releashd::test_support::integration::platform::NotionTaskQuery,
    ) -> Result<
        releashd::test_support::integration::platform::NotionTaskPage,
        releashd::test_support::integration::platform::NotionError,
    > {
        Ok(reopening_page(
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ))
    }
    async fn fetch_label_options(
        &self,
        _: &releashd::test_support::integration::settings::NotionRepoConfig,
    ) -> Result<
        Vec<releashd::test_support::integration::platform::NotionLabelOption>,
        releashd::test_support::integration::platform::NotionError,
    > {
        Ok(reopening_labels(
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ))
    }
    async fn validate(
        &self,
        _: &releashd::test_support::integration::settings::NotionRepoConfig,
    ) -> Result<
        releashd::test_support::integration::platform::NotionValidationResult,
        releashd::test_support::integration::platform::NotionError,
    > {
        Ok(releashd::test_support::integration::platform::NotionValidationResult::not_configured())
    }
}

fn reopening_page(
    sequence: usize,
) -> releashd::test_support::integration::platform::NotionTaskPage {
    releashd::test_support::integration::platform::NotionTaskPage {
        tasks: vec![releashd::test_support::integration::platform::NotionTask {
            id: sequence.to_string(),
            title: "Task".into(),
            url: String::new(),
            labels: Default::default(),
            branch_name: String::new(),
            created_at: String::new(),
            last_edited_at: String::new(),
        }],
        has_more: false,
        next_cursor: None,
    }
}

fn reopening_labels(
    sequence: usize,
) -> Vec<releashd::test_support::integration::platform::NotionLabelOption> {
    vec![
        releashd::test_support::integration::platform::NotionLabelOption {
            property_name: "Status".into(),
            property_type: "select".into(),
            options: vec![sequence.to_string()],
            option_ids: vec![],
        },
    ]
}

struct ReopeningReads {
    inner: WorkspaceStateReads,
    pause: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl StateSubscriptionRead for ReopeningReads {
    fn acquire_external(&self, target: &SubscriptionTarget) {
        self.inner.acquire_external(target);
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        if self.pause.load(std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.inner.refresh_external_blocking(target).await
    }
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        self.inner.read(target).await
    }
    fn release_external(&self, target: &SubscriptionTarget) {
        self.inner.release_external(target);
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
pub async fn test_notion購読_旧client終了中の新規開始は旧workerの解放から項目と結果を保護する() {
    // Given
    let targets = [
        notion_target(),
        SubscriptionTarget::NotionLabelOptions("/repo".into()),
    ];
    let mut results = Vec::new();
    let mut cases = Vec::new();
    for target in targets {
        let mut fixture = crate::state_subscription_reads::Fixture::new();
        fixture.reads.notion = Arc::new(
            releashd::test_support::integration::platform::NotionUsecase::new(
                fixture.config.clone(),
                fixture.config.clone(),
                Arc::new(ReopeningNotionApi::default()),
            ),
        );
        fixture
            .reads
            .notion
            .save_config(
                "/repo".into(),
                releashd::test_support::integration::settings::NotionRepoConfig {
                    api_token: "token".into(),
                    database_id: "database".into(),
                    property_mapping: Default::default(),
                },
            )
            .unwrap();
        let reads = Arc::new(ReopeningReads {
            inner: fixture.reads.clone(),
            pause: Default::default(),
            entered: Default::default(),
            resume: Default::default(),
        });
        let output = Arc::new(RecordingOutput::default());
        let subscriptions = StateSubscriptionUsecase::new_with_output(
            output.clone(),
            releashd::test_support::integration::subscriptions::pending_read_driver(),
        )
        .with_reads(reads.clone(), None, vec![], String::new());
        subscriptions.open_client("old".into()).unwrap();
        subscriptions.open_client("new".into()).unwrap();
        subscriptions
            .start_subscription("old", &target, &FakeDelivery)
            .await
            .unwrap();
        cases.push((target, fixture, reads, output, subscriptions));
    }
    // When
    for (target, _fixture, reads, output, subscriptions) in cases {
        subscriptions.test_remove_client_registration("old");
        reads.pause.store(true, std::sync::atomic::Ordering::SeqCst);
        let starting = subscriptions.clone();
        let start_target = target.clone();
        let start = tokio::spawn(async move {
            starting
                .start_subscription("new", &start_target, &FakeDelivery)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), reads.entered.notified())
            .await
            .unwrap();
        let old_workers = subscriptions.test_worker_count();
        subscriptions.close_client("old");
        let pending = reads.read(&target).await;
        let remaining_workers = subscriptions.test_worker_count();
        reads
            .pause
            .store(false, std::sync::atomic::Ordering::SeqCst);
        reads.resume.notify_one();
        let started = start.await.unwrap();
        let initial = reads.read(&target).await;
        subscriptions.notify(StateChangeSource::NotionConfig("/repo".into()));
        tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
            .await
            .unwrap();
        let updated = reads.read(&target).await;
        let initial_values = output.initial_values.lock().clone();
        let update_values = output.update_values.lock().clone();
        let failures = output.failures.lock().clone();
        subscriptions.close_client("new");
        let released = reads.read(&target).await;
        results.push((
            target,
            old_workers,
            remaining_workers,
            pending,
            started,
            initial,
            updated,
            initial_values,
            update_values,
            failures,
            released,
        ));
    }
    // Then
    for (
        target,
        old_workers,
        remaining_workers,
        pending,
        started,
        initial,
        updated,
        initial_values,
        update_values,
        failures,
        released,
    ) in results
    {
        assert_eq!(old_workers, 1);
        assert_eq!(remaining_workers, 0);
        let (empty, expected_initial, expected_updated) = match target {
            SubscriptionTarget::NotionTasks(_) => (
                StateValue::NotionTasks(Default::default()),
                StateValue::NotionTasks(
                    releashd::test_support::integration::platform::Fetched::ready(reopening_page(
                        1,
                    )),
                ),
                StateValue::NotionTasks(
                    releashd::test_support::integration::platform::Fetched::ready(reopening_page(
                        2,
                    )),
                ),
            ),
            SubscriptionTarget::NotionLabelOptions(_) => (
                StateValue::NotionLabelOptions(Default::default()),
                StateValue::NotionLabelOptions(
                    releashd::test_support::integration::platform::Fetched::ready(
                        reopening_labels(1),
                    ),
                ),
                StateValue::NotionLabelOptions(
                    releashd::test_support::integration::platform::Fetched::ready(
                        reopening_labels(2),
                    ),
                ),
            ),
            _ => unreachable!(),
        };
        assert_eq!(pending.unwrap(), empty);
        assert!(started.is_ok());
        assert_eq!(initial.unwrap(), expected_initial);
        assert_eq!(updated.unwrap(), expected_updated);
        assert_eq!(initial_values[1], expected_initial);
        assert_eq!(update_values, vec![expected_updated]);
        assert!(failures.is_empty());
        assert!(released.is_err());
    }
}
