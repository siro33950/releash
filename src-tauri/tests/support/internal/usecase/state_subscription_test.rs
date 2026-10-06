use super::state_subscription_test_helpers::{notion_target, FakeDelivery, RecordingOutput};
use super::*;

#[derive(Default)]
struct ReopeningNotionApi(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl crate::domain::notion::NotionApiGateway for ReopeningNotionApi {
    async fn query_tasks(
        &self,
        _: &crate::domain::app_config::value_objects::NotionRepoConfig,
        _: &crate::domain::notion::NotionTaskQuery,
    ) -> Result<crate::domain::notion::NotionTaskPage, crate::domain::notion::NotionError> {
        Ok(reopening_page(
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ))
    }
    async fn fetch_label_options(
        &self,
        _: &crate::domain::app_config::value_objects::NotionRepoConfig,
    ) -> Result<Vec<crate::domain::notion::NotionLabelOption>, crate::domain::notion::NotionError>
    {
        Ok(reopening_labels(
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ))
    }
    async fn validate(
        &self,
        _: &crate::domain::app_config::value_objects::NotionRepoConfig,
    ) -> Result<crate::domain::notion::NotionValidationResult, crate::domain::notion::NotionError>
    {
        Ok(crate::domain::notion::NotionValidationResult::not_configured())
    }
}

fn reopening_page(sequence: usize) -> crate::domain::notion::NotionTaskPage {
    crate::domain::notion::NotionTaskPage {
        tasks: vec![crate::domain::notion::NotionTask {
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

fn reopening_labels(sequence: usize) -> Vec<crate::domain::notion::NotionLabelOption> {
    vec![crate::domain::notion::NotionLabelOption {
        property_name: "Status".into(),
        property_type: "select".into(),
        options: vec![sequence.to_string()],
        option_ids: vec![],
    }]
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
        let mut fixture = crate::test_support::state_subscription::StateReadsFixture::new();
        fixture.reads.notion = Arc::new(crate::usecase::notion::usecase::NotionUsecase::new(
            fixture.config.clone(),
            fixture.config.clone(),
            Arc::new(ReopeningNotionApi::default()),
        ));
        fixture
            .reads
            .notion
            .save_config(
                "/repo".into(),
                crate::domain::app_config::value_objects::NotionRepoConfig {
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
            crate::test_support::state_subscription::pending_read_driver(),
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
                StateValue::NotionTasks(crate::usecase::fetched::Fetched::ready(reopening_page(1))),
                StateValue::NotionTasks(crate::usecase::fetched::Fetched::ready(reopening_page(2))),
            ),
            SubscriptionTarget::NotionLabelOptions(_) => (
                StateValue::NotionLabelOptions(Default::default()),
                StateValue::NotionLabelOptions(crate::usecase::fetched::Fetched::ready(
                    reopening_labels(1),
                )),
                StateValue::NotionLabelOptions(crate::usecase::fetched::Fetched::ready(
                    reopening_labels(2),
                )),
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
