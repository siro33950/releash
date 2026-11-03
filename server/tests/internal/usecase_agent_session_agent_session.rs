use releashd::test_support::integration::platform::install_capturing_logger;
use releashd::test_support::integration::sessions::test_sessions;
use releashd::test_support::integration::sessions::AgentSessionLifecycleUsecase;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::providers::ProviderLifecycleUsecase;
use releashd::test_support::integration::sessions::AgentSessionArchiveOutcome;
use releashd::test_support::integration::sessions::AgentSessionHistoryMetadata;
use releashd::test_support::integration::sessions::AgentSessionHistoryResumeOutcome;
use releashd::test_support::integration::sessions::AgentSessionHistoryResumeRequest;
use releashd::test_support::integration::sessions::AgentSessionLaunchRequest;
use releashd::test_support::integration::sessions::AgentSessionLaunchUsecase;
use releashd::test_support::integration::sessions::AgentSessionLaunchUsecaseError;
use releashd::test_support::integration::sessions::AgentSessionLifecycle;
use releashd::test_support::integration::sessions::AgentSessionProcessExitOutcome;
use releashd::test_support::integration::sessions::AgentSessionRecoveryResult;
use releashd::test_support::integration::sessions::AgentSessionUsecase;
use releashd::test_support::integration::sessions::AgentSessionUsecaseError;
use releashd::test_support::integration::sessions::ExecutionTreeCacheReleaseError;
use releashd::test_support::integration::sessions::ManagedPtyPresence;
use releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError;
use releashd::test_support::integration::sessions::ProviderSessionLaunch;
use releashd::test_support::integration::sessions::StartedExecutionTreeRegistrationError;
use releashd::test_support::integration::sessions::WorkflowAgentSessionLaunchRequest;
use releashd::test_support::integration::workspace::WorkspaceIdentity;

use releashd::test_support::integration::sessions::{
    captured_terminal_spawn_failure, hook_health_usecase, provider_runtime, session_location,
    started_execution_trees, workflow_location, BlockingLaunchTerminal,
    FailingFirstLifecycleEvents, FixedAvailability, FixedHistory, RecordingLaunchGateway,
    RecordingLifecycleEvents, RecordingStartedExecutionTrees, RecordingTerminal,
};

#[tokio::test]
pub async fn test_agent_session_history_resume_実行木登録失敗ではcreateをrollbackする() {
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_history_resume_同一要求の再送は既存sessionへ収束する() {
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_history_resumeは新しいsessionを作り失敗時もidを保持する() {
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_history_resume_lifecycle準備失敗でもpausedへ収束する() {
    todo!()
}

fn durable_usecase(
    directory: &tempfile::TempDir,
) -> (
    Arc<releashd::test_support::integration::persistence::LocalEventStore>,
    AgentSessionUsecase,
) {
    let store = releashd::test_support::integration::persistence::LocalEventStore::open(
        releashd::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ),
    )
    .unwrap();
    let repository = Arc::new(
        releashd::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    );
    (
        store,
        test_sessions(repository, Arc::default(), Arc::default(), Arc::default()),
    )
}

#[tokio::test]
pub async fn test_agent_session_usecase_process_exit_resume_archive_deleteを永続化する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-session-1"),
            "create-1",
        )
        .await
        .unwrap();
    usecase
        .associate_provider_session("agent-session-1", "claude-session-1", None, "associate-1")
        .await
        .unwrap();

    assert_eq!(
        usecase
            .observe_process_exit("agent-session-1", Some(0), "exit-1")
            .await
            .unwrap(),
        AgentSessionProcessExitOutcome::Paused
    );
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Failed,
            "resume-failed-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Succeeded,
            "resume-success-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .archive("agent-session-1", "archive-1")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    usecase.delete("agent-session-1", "delete-1").await.unwrap();
    assert!(usecase.find("agent-session-1").await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_agent_session_usecase_id不明archiveも確認なしで記録する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-unknown",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-unknown"),
            "create-unknown",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .archive("agent-session-unknown", "archive-unknown")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert!(usecase
        .find("agent-session-unknown")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
pub async fn test_agent_session_usecase_gcはpty不在確定時だけunknown_idを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-gc",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-gc"),
            "create-gc",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .garbage_collect(
                "agent-session-gc",
                ManagedPtyPresence::Unknown,
                "gc-unknown"
            )
            .await
            .unwrap_err(),
        AgentSessionUsecaseError::InvalidOperation
    );
    usecase
        .garbage_collect(
            "agent-session-gc",
            ManagedPtyPresence::ConfirmedAbsent,
            "gc-confirmed",
        )
        .await
        .unwrap();
    assert!(usecase.find("agent-session-gc").await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_provider_agent_workflow_session_launch_workflow関連付け後に初回指示付きprovider_tuiを起動する(
) {
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn test_provider_agent_workflow_session_launch_別sessionのactivateを起動待ちで直列化しない(
) {
    todo!()
}

#[tokio::test]
pub async fn test_provider_agent_workflow_session_launch_activate後のrollbackで起動資源を解放する()
{
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_launch_spawn失敗時はsessionとlaunch資源をrollbackする() {
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_launch_prepare失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    todo!()
}

#[tokio::test]
pub async fn test_agent_session_launch_spawn失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    todo!()
}

#[tokio::test]
pub async fn test_session選択_欠落はcorruptとしqueryの技術的性質を保持する() {
    todo!()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub async fn test_agent_session_launch_pty起動中のsessionをgcしない() {
    todo!()
}
