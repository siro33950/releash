use super::*;
use crate::adaptor::controller::agent_session_wiring::{
    compose_agent_sessions, AgentSessionCompositionInput,
};
use crate::adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway;
use crate::adaptor::gateway::push::ClientAgentSessionChangeNotifier;
use crate::adaptor::gateway::repository::{scanner::DefaultRepositoryScanner, state::*};
use crate::adaptor::gateway::workflow::{
    node_process::WorkflowNodeProcesses, workflow_host::WorkflowRuntimeDependencies,
    RepositoryIsolatedWorktreeGateway,
};
use crate::domain::repository::worktree_operation::WorktreeDeletionTarget;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_state::RepositoryStateService;
use crate::usecase::repository_usecase::WorktreeExecutionArchiver;

#[tokio::test]
async fn test_worktree削除一覧_本番runtime配線で受理した削除状態を処理終了まで共有する() {
    // Given
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().canonicalize().unwrap();
    let repo_path = root.join("repo");
    let repo_path = repo_path.to_str().unwrap();
    let repo = git2::Repository::init(repo_path).unwrap();
    crate::test_support::git::create_initial_commit(&repo);
    let repository = Arc::new(build_repository_usecase());
    let worktree = repository
        .create_worktree(repo_path, "feature", true, None)
        .unwrap();
    let data_dir = root.join("data");
    let store = LocalEventStore::open(LocalEventStoreConfig::production(data_dir.clone())).unwrap();
    let config = Arc::new(AppConfig::new(
        ReleashConfig::default(),
        data_dir.join("releash.toml"),
    ));
    let push = Arc::new(PushSink::new());
    let terminal = Arc::new(build_terminal_surface_application_for_tests());
    let sessions = compose_agent_sessions(AgentSessionCompositionInput {
        store: store.clone(),
        data_dir: data_dir.clone(),
        provider_executable_config: config.clone(),
        provider_executable_probe: Arc::new(LocalProviderExecutableProbeGateway::with_search_path(
            None,
        )),
        claude_config_dir: root.join("claude"),
        codex_home: root.join("codex"),
        cli_binary: "releash".into(),
        terminal: terminal.clone(),
        change_notifier: Arc::new(ClientAgentSessionChangeNotifier::new(push.clone())),
    })
    .unwrap();
    let processes = Arc::new(WorkflowNodeProcesses::new(terminal));
    let (_, workspace_query) = build_workflow_services_with_repository_worktrees(
        data_dir,
        repository.clone(),
        config.clone(),
        config.clone(),
        store.clone(),
        processes.clone(),
    );
    let runtime = build_workflow_runtime_usecase(
        WorkflowRuntimeDependencies {
            processes: processes.clone(),
            store: Some(store),
            config: Some(config.clone()),
            secrets: Some(config.clone()),
            push: push.clone(),
        },
        WorkflowRuntimeCommandGatewayDeps {
            node_processes: processes,
            repository_usecase: repository.clone(),
            app_config: config,
            workspace_query,
            agent_session_launch: sessions.launch,
            agent_session_initial_instruction: sessions.initial_instruction,
            agent_session_lifecycle: sessions.lifecycle,
            provider_availability: sessions.availability_reader,
            isolated_worktrees: Arc::new(RepositoryIsolatedWorktreeGateway),
        },
    )
    .unwrap();
    let state = RepositoryStateService::new(
        Arc::new(RepositoryStateRepositoryGateway::new(repository.clone())),
        Arc::new(DefaultRepositoryScanner::new(
            repository.clone(),
            Arc::new(build_code_usecase()),
        )),
        Arc::new(ClientRepositoryStateNotifier::new(push)),
        Arc::new(NotifyRepositoryStateWatcher::new(repository)),
        Arc::new(TokioRepositoryStateWorkerRuntime),
        Arc::new(FsWorktreePathNormalizer),
    );
    assert!(state
        .list_branches_with_status_snapshot(repo_path)
        .unwrap()
        .branches
        .iter()
        .all(|card| !card.is_deleting));

    // When
    let mut deletion = runtime
        .begin_worktree_deletion(&worktree.path)
        .await
        .unwrap();
    runtime.archive_worktree(&worktree.path).await.unwrap();
    deletion
        .accept(WorktreeDeletionTarget {
            repository_root: repo_path.into(),
            path: worktree.path.clone(),
            branch: Some(worktree.branch.clone()),
        })
        .unwrap();

    // Then
    for git_registration_removed in [false, true] {
        if git_registration_removed {
            crate::adaptor::gateway::repository::worktree::remove_worktree(
                repo_path,
                &worktree.path,
                false,
            )
            .unwrap();
        }
        let snapshot = state.list_branches_with_status_snapshot(repo_path).unwrap();
        let areas = snapshot.worktree_display_groups.working_areas;
        assert_eq!(areas.len(), 2);
        let deleting: Vec<_> = areas.iter().filter(|card| card.is_deleting).collect();
        assert_eq!(deleting.len(), 1);
        assert_eq!(deleting[0].name, worktree.branch);
        assert_eq!(
            deleting[0].worktree_path.as_deref(),
            Some(worktree.path.as_str())
        );
    }
    drop(deletion);
    let snapshot = state.list_branches_with_status_snapshot(repo_path).unwrap();
    assert!(snapshot.branches.iter().all(|card| !card.is_deleting));
    assert_eq!(snapshot.worktree_display_groups.working_areas.len(), 1);
}
