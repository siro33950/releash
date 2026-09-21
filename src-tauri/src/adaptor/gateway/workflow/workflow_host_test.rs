use super::test_helpers::{TestSessions, TestWorktrees};
use super::workflow_host_tests::{AcceptingWorktreeResolver, UnusedWorkflowResolver};
use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::{
    WorkflowExecutionArchiveFileRepository, WorkflowRuntimeCommandGateway,
};
use crate::adaptor::gateway::workspace_tree::{
    SqliteWorkspaceQueryService, SqliteWorkspaceTreeRepository,
};
use crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand;
use crate::usecase::workflow::command::SubmitOutputCommand;
use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;

#[tokio::test]
async fn test_workflow永続化_本番構成で起動から完了とabortまで状態ファイルを作らない() {
    for status in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let app = test_helpers::dependencies(Some(store.clone()));
        let query = SqliteWorkspaceQueryService::with_repository(
            SqliteWorkspaceTreeRepository::new(store),
            Arc::new(WorkflowExecutionArchiveFileRepository::new(
                directory.path(),
            )),
        );
        let execution_store = Arc::new(ExecutionStore::new_canonical(query.clone()));
        let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            execution_store.clone(),
            Arc::new(TestSessions::default()),
            Arc::new(TestWorktrees::default()),
        ));
        let workflow = serde_saphyr::from_str(
            "name: persistence\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}",
        ).unwrap();

        // When
        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                None,
                ExecutionOrigin::Cli,
            )
            .await
            .unwrap();
        let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();

        // Then
        assert_eq!(snapshot.state, RuntimeExecutionState::Running);
        assert!(execution_store
            .active_execution_snapshot(&execution_id)
            .await
            .is_some());
        assert!(!directory.path().join("workflow_executions").exists());

        // When
        if status == ExecutionStatus::Aborted {
            host.abort_workflow_execution(&app, &execution_id, None)
                .await
                .unwrap();
        } else {
            let node = &snapshot.node_executions[0];
            let control = WorkflowControlPlaneUsecase::new(Arc::new(
                WorkflowRuntimeCommandGateway::new_with_driver(app, host),
            ));
            control
                .submit_output(SubmitOutputCommand {
                    node_execution_id: node.id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            control
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: node.session_id.clone().unwrap(),
                        tree_id: execution_id.clone(),
                        node_execution_id: node.id.clone(),
                        binding_id: "binding-persistence-test".into(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
        }

        // Then
        assert!(execution_store
            .active_execution_snapshot(&execution_id)
            .await
            .is_none());
        let record = execution_store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, status);
        let reloaded = ExecutionStore::new_canonical(query)
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded, record);
        assert!(!directory.path().join("workflow_executions").exists());
    }
}
