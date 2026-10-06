use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::providers::ProviderLifecycleUsecase;
use releash_lib::test_support::integration::sessions::hook_health_usecase;
use releash_lib::test_support::integration::sessions::provider_runtime;
use releash_lib::test_support::integration::sessions::started_execution_trees;
use releash_lib::test_support::integration::sessions::AgentSessionLaunchUsecase;
use releash_lib::test_support::integration::sessions::AgentSessionLaunchUsecaseError;
use releash_lib::test_support::integration::sessions::AgentSessionUsecase;
use releash_lib::test_support::integration::sessions::FixedAvailability;
use releash_lib::test_support::integration::sessions::FixedHistory;
use releash_lib::test_support::integration::sessions::RecordingLaunchGateway;
use releash_lib::test_support::integration::sessions::RecordingLifecycleEvents;
use releash_lib::test_support::integration::sessions::RecordingTerminal;
use releash_lib::test_support::integration::sessions::WorkflowAgentSessionLaunchRequest;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
use std::sync::Arc;
use std::sync::Mutex;

#[tokio::test]
pub async fn test_workflow起動保持_偽の期限通知だけでactivated記録を消す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let elapsed = Arc::new(tokio::sync::Notify::new());
    let armed = Arc::new(tokio::sync::Notify::new());
    let activated = releash_lib::test_support::integration::platform::run(Arc::new({
        let elapsed = elapsed.clone();
        let armed = armed.clone();
        move || {
            armed.notify_one();
            let elapsed = elapsed.clone();
            Box::pin(async move {
                elapsed.notified().await;
            })
        }
    }));
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory { entries: Vec::new() }),
hook_health_usecase(),
started_execution_trees(),
activated, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let launched = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-1".to_string(),
            node_execution_id: "node-1".to_string(),
            initial_instruction: "Implement the workflow node.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-launch-1".to_string(),
        })
        .await
        .unwrap();

    let usecase = Arc::new(usecase);
    usecase
        .activate_workflow_node(launched.session().id())
        .await
        .unwrap();
    armed.notified().await;
    assert_eq!(Arc::strong_count(&usecase), 1);
    assert!(
        usecase
            .test_has_activated_launch(launched.session().id())
            .await
    );
    // When
    elapsed.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while usecase
            .test_has_activated_launch(launched.session().id())
            .await
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Then
    assert_eq!(
        usecase
            .confirm_workflow_node_attachment(launched.session().id())
            .await,
        Err(AgentSessionLaunchUsecaseError::InvalidInput)
    );
}
