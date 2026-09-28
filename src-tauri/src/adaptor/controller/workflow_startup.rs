use crate::common::retry::{bounded, RetryBackoff};
use crate::domain::workflow::WorkflowError;
use crate::usecase::failure::{attempt_expired, FailureKey, ATTEMPT_LIMIT};
use crate::usecase::retry::Retrying;
use crate::usecase::workflow::startup::WorkflowStartupUsecase;

fn expired() -> WorkflowError {
    let failure = attempt_expired();
    let message = failure.message.clone();
    WorkflowError::Store(
        crate::domain::failure::StorageFailure::from(failure).with_message(message),
    )
}

pub(crate) async fn recover(
    retrying: &Retrying,
    usecase: &WorkflowStartupUsecase,
) -> Result<(), WorkflowError> {
    let tree_ids = retrying
        .restart(
            FailureKey::new("workflow_recovery_list", "daemon"),
            RetryBackoff::RECOVERY,
            |_| bounded(ATTEMPT_LIMIT, expired, usecase.list_tree_ids()),
        )
        .await?;
    let results = futures_util::future::join_all(tree_ids.into_iter().map(|tree_id| async move {
        retrying
            .restart(
                FailureKey::new("workflow_recovery", &tree_id),
                RetryBackoff::RECOVERY,
                |progress| {
                    bounded(
                        ATTEMPT_LIMIT,
                        expired,
                        usecase.recover_tree(&tree_id, progress),
                    )
                },
            )
            .await
    }))
    .await;
    results.into_iter().collect()
}

#[cfg(test)]
#[path = "workflow_startup_test.rs"]
mod workflow_startup_tests;
