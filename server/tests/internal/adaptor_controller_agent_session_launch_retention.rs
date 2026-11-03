use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::providers::ProviderLifecycleUsecase;
use releashd::test_support::integration::sessions::hook_health_usecase;
use releashd::test_support::integration::sessions::provider_runtime;
use releashd::test_support::integration::sessions::started_execution_trees;
use releashd::test_support::integration::sessions::test_sessions;
use releashd::test_support::integration::sessions::AgentSessionLaunchUsecase;
use releashd::test_support::integration::sessions::AgentSessionLaunchUsecaseError;
use releashd::test_support::integration::sessions::FixedAvailability;
use releashd::test_support::integration::sessions::FixedHistory;
use releashd::test_support::integration::sessions::RecordingLaunchGateway;
use releashd::test_support::integration::sessions::RecordingLifecycleEvents;
use releashd::test_support::integration::sessions::RecordingTerminal;
use releashd::test_support::integration::sessions::WorkflowAgentSessionLaunchRequest;
use releashd::test_support::integration::workspace::WorkspaceIdentity;
use std::sync::Arc;
use std::sync::Mutex;

#[tokio::test]
pub async fn test_workflow起動保持_偽の期限通知だけでactivated記録を消す() {
    todo!()
}
