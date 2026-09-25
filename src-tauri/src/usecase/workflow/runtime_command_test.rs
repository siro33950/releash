use super::*;
use crate::domain::failure::FailureKind;
use crate::usecase::agent_session::{
    ExecutionTreeCache, ExecutionTreeCacheReleaseError, StartedExecutionTreeRegistrar,
    StartedExecutionTreeRegistrationError,
};
use crate::usecase::provider_lifecycle::{
    ProviderExecutionTreeStopCommand, ProviderExecutionTreeStopTransaction,
};

#[tokio::test]
async fn test_workflow失敗_stopとcache解放と起動登録で元の分類を保持する() {
    // Given
    for error in [
        WorkflowError::Stopped(crate::domain::operation_context::OperationStopped::Expired),
        WorkflowError::Stopped(crate::domain::operation_context::OperationStopped::Cancelled),
        WorkflowError::External("internal".into()),
        WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
            "launch".into(),
        )),
        WorkflowError::StorageUnavailable {
            message: "busy".into(),
            kind: FailureKind::Temporary,
        },
        WorkflowError::StorageUnavailable {
            message: "expired".into(),
            kind: FailureKind::Expired,
        },
        WorkflowError::StorageUnavailable {
            message: "repair".into(),
            kind: FailureKind::StateRequired,
        },
    ] {
        let expected = error.failure_kind();
        let gateway = Arc::new(FakeRuntimeGateway {
            failure: Some(error),
            ..Default::default()
        });
        let usecase = WorkflowRuntimeUsecase::new(
            gateway,
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );
        // When / Then
        let stop_error = ProviderExecutionTreeStopTransaction::commit_provider_stop(
            &usecase,
            ProviderExecutionTreeStopCommand {
                tree_id: "tree".into(),
                node_execution_id: "node".into(),
                agent_session_id: "session".into(),
                binding_id: "binding".into(),
            },
            Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(stop_error.failure_kind(), expected);
        assert_eq!(
            ExecutionTreeCache::release_deleted_execution_tree(&usecase, "tree").await,
            Err(ExecutionTreeCacheReleaseError::Store(expected))
        );
        assert_eq!(
            StartedExecutionTreeRegistrar::register_started_execution_tree(&usecase, "tree").await,
            Err(StartedExecutionTreeRegistrationError::Store(expected))
        );
    }
}
