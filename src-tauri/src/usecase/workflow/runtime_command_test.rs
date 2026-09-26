use super::*;
use crate::domain::failure::{StorageFailure, StorageFailureSource, TechnicalFailureNature};
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
        WorkflowError::Technical(
            crate::common::operation_context::OperationStopped::Expired.into(),
        ),
        WorkflowError::Technical(
            crate::common::operation_context::OperationStopped::Cancelled.into(),
        ),
        WorkflowError::External("internal".into()),
        WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
            "launch".into(),
        )),
        WorkflowError::Store(
            crate::domain::failure::StorageFailure::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            )
            .with_message("busy"),
        ),
        WorkflowError::Store(
            crate::domain::failure::StorageFailure::from(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                },
            )
            .with_message("expired"),
        ),
        WorkflowError::Store(
            crate::domain::failure::StorageFailure::from(
                crate::domain::local_event::CommitBatchError::PayloadConflict,
            )
            .with_message("repair"),
        ),
    ] {
        let expected = match &error {
            WorkflowError::Store(failure) => failure.clone(),
            WorkflowError::Editor(error) => StorageFailure {
                nature: TechnicalFailureNature::Other,
                source: StorageFailureSource::Editor(error.clone()),
                context: None,
            },
            error => StorageFailure {
                nature: match error {
                    WorkflowError::Technical(failure) => failure.nature,
                    _ => TechnicalFailureNature::Other,
                },
                source: StorageFailureSource::Workflow(Box::new(error.clone())),
                context: None,
            },
        };
        let gateway = Arc::new(FakeRuntimeGateway {
            failure: Some(error),
            ..Default::default()
        });
        let usecase = WorkflowRuntimeUsecase::new(
            gateway,
            Arc::new(crate::usecase::workflow::NoopArchiveRepository),
        );
        // When / Then
        let stop = ProviderExecutionTreeStopTransaction::commit_provider_stop(
            &usecase,
            ProviderExecutionTreeStopCommand {
                tree_id: "tree".into(),
                node_execution_id: "node".into(),
                agent_session_id: "session".into(),
                binding_id: "binding".into(),
            },
            Vec::new(),
        );
        if expected.nature == TechnicalFailureNature::Transient {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), stop)
                    .await
                    .is_err()
            );
        } else {
            assert_eq!(
                stop.await.unwrap_err(),
                crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(expected.clone())
            );
        }
        assert_eq!(
            match ExecutionTreeCache::release_deleted_execution_tree(&usecase, "tree")
                .await
                .unwrap_err()
            {
                ExecutionTreeCacheReleaseError::Store(failure) => failure,
                error => panic!("unexpected error: {error:?}"),
            },
            expected
        );
        assert_eq!(
            match StartedExecutionTreeRegistrar::register_started_execution_tree(&usecase, "tree")
                .await
                .unwrap_err()
            {
                StartedExecutionTreeRegistrationError::Store(failure) => failure,
                error => panic!("unexpected error: {error:?}"),
            },
            expected
        );
    }
}
