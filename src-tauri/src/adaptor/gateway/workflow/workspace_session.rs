use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::operation::{
    SessionLifecycleAction, SessionLifecycleCommandResult, SessionLifecycleOperationError,
    SessionLifecycleOperationState, SessionLifecycleOperationUsecase, SessionLifecycleRejection,
    SessionLifecycleRequest, LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
};
use crate::usecase::agent_session::session::{SessionState, SessionStore, SessionSummary};
use crate::usecase::workflow::ports::WorkspaceNodeSessionCloseGateway;
use crate::usecase::workflow::{
    WorkspaceSessionGateway, WorkspaceSessionInput, WorkspaceSessionState,
};

pub(crate) struct StoredWorkspaceSessionGateway {
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
}

impl StoredWorkspaceSessionGateway {
    pub(crate) fn new(session_store: Arc<SessionStore>, data_dir: PathBuf) -> Self {
        Self {
            session_store,
            data_dir,
        }
    }

    fn session_input(
        &self,
        session: SessionSummary,
    ) -> Result<WorkspaceSessionInput, WorkflowError> {
        let workflow_execution_id = session
            .workflow_node_context
            .as_ref()
            .map(|context| context.execution_id.clone());
        let session_recovery_reason = self
            .session_store
            .unresolved_recovery_reason(&session.id)
            .map_err(WorkflowError::external)?;
        let execution_recovery_reason = workflow_execution_id
            .as_deref()
            .map(|execution_id| {
                self.session_store
                    .unresolved_recovery_reason(execution_id)
                    .map_err(WorkflowError::external)
            })
            .transpose()?
            .flatten();
        Ok(WorkspaceSessionInput {
            id: session.id,
            worktree_path: session.worktree_path,
            state: workspace_session_state(session.state),
            error_reason: session.error_reason,
            updated_at: session.updated_at,
            first_message: session.first_message,
            workflow_node_session: session.workflow_node_session,
            workflow_execution_id,
            unresolved_recovery_reason: session_recovery_reason.or(execution_recovery_reason),
        })
    }
}

impl WorkspaceSessionGateway for StoredWorkspaceSessionGateway {
    fn list_active_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        self.session_store
            .list_published_sessions(&self.data_dir, worktree_path)
            .map_err(WorkflowError::external)?
            .into_iter()
            .map(|session| self.session_input(session))
            .collect()
    }

    fn list_closed_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        self.session_store
            .list_published_closed_sessions(&self.data_dir, worktree_path)
            .map_err(WorkflowError::external)?
            .into_iter()
            .map(|session| self.session_input(session))
            .collect()
    }
}

#[async_trait]
trait WorkspaceNodeLifecycleRequester: Send + Sync {
    async fn request(
        &self,
        request: SessionLifecycleRequest,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError>;
}

#[async_trait]
impl WorkspaceNodeLifecycleRequester for SessionLifecycleOperationUsecase {
    async fn request(
        &self,
        request: SessionLifecycleRequest,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        SessionLifecycleOperationUsecase::request(self, request).await
    }
}

pub(crate) struct DurableWorkspaceNodeSessionCloseGateway {
    lifecycle: Arc<dyn WorkspaceNodeLifecycleRequester>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
}

impl DurableWorkspaceNodeSessionCloseGateway {
    pub(crate) fn new(
        lifecycle: Arc<SessionLifecycleOperationUsecase>,
        session_store: Arc<SessionStore>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            lifecycle,
            session_store,
            data_dir,
        }
    }

    #[cfg(test)]
    fn with_requester(
        lifecycle: Arc<dyn WorkspaceNodeLifecycleRequester>,
        session_store: Arc<SessionStore>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            lifecycle,
            session_store,
            data_dir,
        }
    }
}

#[async_trait]
impl WorkspaceNodeSessionCloseGateway for DurableWorkspaceNodeSessionCloseGateway {
    async fn close_session(&self, session_id: &str) -> Result<(), WorkflowError> {
        let meta = self
            .session_store
            .get_session_meta(&self.data_dir, session_id)
            .map_err(|error| {
                WorkflowError::external(format!(
                    "Failed to read the SessionNode close target: {error}"
                ))
            })?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("SessionNode target not found: {session_id}"))
            })?;
        let expected_session_revision = i64::try_from(meta.state_revision).map_err(|_| {
            WorkflowError::invalid_state("SessionNode revision exceeds the supported range")
        })?;
        classify_workspace_node_close_result(
            self.lifecycle
                .request(SessionLifecycleRequest {
                    principal: LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                    request_id: workspace_node_close_request_id(session_id, meta.state_revision),
                    session_id: session_id.to_string(),
                    expected_session_revision,
                    action: SessionLifecycleAction::Close,
                })
                .await,
        )
    }
}

fn workspace_node_close_request_id(session_id: &str, session_revision: u64) -> String {
    let digest = Sha256::digest(
        format!("workspace-node-close/v1\0{session_id}\0{session_revision}").as_bytes(),
    );
    format!("workspace-node-close-{}", hex::encode(digest))
}

fn classify_workspace_node_close_result(
    result: Result<SessionLifecycleCommandResult, SessionLifecycleOperationError>,
) -> Result<(), WorkflowError> {
    match result {
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }) => Ok(()),
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::ReconciliationRequired { failure },
            ..
        })
        | Ok(SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::Failed {
            failure,
        }))
        | Err(SessionLifecycleOperationError::StorageUnavailable { failure }) => {
            Err(WorkflowError::external(format!(
                "SessionNode close requires reconciliation: {failure}"
            )))
        }
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Accepted,
            ..
        }) => Err(WorkflowError::external(
            "SessionNode close has not reached a durable terminal result",
        )),
        Ok(SessionLifecycleCommandResult::OutcomeUnknown { .. }) => Err(WorkflowError::external(
            "SessionNode close outcome is unknown; retry the same close action",
        )),
        Ok(SessionLifecycleCommandResult::Rejected(rejection)) => Err(
            WorkflowError::invalid_state(format!("SessionNode close was rejected: {rejection:?}")),
        ),
        Err(error) => Err(WorkflowError::external(format!(
            "SessionNode close operation failed: {error:?}"
        ))),
    }
}

fn workspace_session_state(state: SessionState) -> WorkspaceSessionState {
    match state {
        SessionState::Active => WorkspaceSessionState::Active,
        SessionState::Idle => WorkspaceSessionState::Idle,
        SessionState::Done => WorkspaceSessionState::Done,
        SessionState::Error => WorkspaceSessionState::Error,
        SessionState::Closed => WorkspaceSessionState::Closed,
        SessionState::Archived => WorkspaceSessionState::Archived,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
    use parking_lot::Mutex;

    struct RecordingLifecycleRequester {
        requests: Mutex<Vec<SessionLifecycleRequest>>,
        result: Mutex<Result<SessionLifecycleCommandResult, SessionLifecycleOperationError>>,
    }

    #[async_trait]
    impl WorkspaceNodeLifecycleRequester for RecordingLifecycleRequester {
        async fn request(
            &self,
            request: SessionLifecycleRequest,
        ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
            self.requests.lock().push(request);
            self.result.lock().clone()
        }
    }

    fn completed_result(session_id: &str) -> SessionLifecycleCommandResult {
        SessionLifecycleCommandResult::Accepted {
            receipt: crate::usecase::agent_session::operation::SessionLifecycleReceipt {
                operation_id: "operation-1".to_string(),
                session_id: session_id.to_string(),
                action: SessionLifecycleAction::Close,
                first_accepted_revision: 0,
            },
            state: SessionLifecycleOperationState::Completed,
        }
    }

    #[tokio::test]
    async fn workspace_node_close_uses_the_durable_lifecycle_with_a_stable_target_revision() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let state_revision = session_store
            .get_session_meta(data_dir.path(), &session.id)
            .unwrap()
            .unwrap()
            .state_revision;
        let requester = Arc::new(RecordingLifecycleRequester {
            requests: Mutex::new(Vec::new()),
            result: Mutex::new(Ok(completed_result(&session.id))),
        });
        let gateway = DurableWorkspaceNodeSessionCloseGateway::with_requester(
            requester.clone(),
            session_store,
            data_dir.path().to_path_buf(),
        );

        gateway.close_session(&session.id).await.unwrap();
        gateway.close_session(&session.id).await.unwrap();

        let requests = requester.requests.lock();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(
            requests[0].principal,
            LOCAL_INSTALLATION_OPERATION_PRINCIPAL
        );
        assert_eq!(requests[0].session_id, session.id);
        assert_eq!(
            requests[0].expected_session_revision,
            i64::try_from(state_revision).unwrap()
        );
        assert_eq!(requests[0].action, SessionLifecycleAction::Close);
        assert!(requests[0].request_id.starts_with("workspace-node-close-"));
        assert!(requests[0].request_id.len() <= 128);
        crate::usecase::agent_session::operation::validate_operation_identity(
            &requests[0].request_id,
        )
        .unwrap();
    }

    #[test]
    fn workspace_node_close_succeeds_only_after_durable_completion() {
        let receipt = crate::usecase::agent_session::operation::SessionLifecycleReceipt {
            operation_id: "operation-1".to_string(),
            session_id: "session-1".to_string(),
            action: SessionLifecycleAction::Close,
            first_accepted_revision: 1,
        };
        assert!(classify_workspace_node_close_result(Ok(
            SessionLifecycleCommandResult::Accepted {
                receipt: receipt.clone(),
                state: SessionLifecycleOperationState::Accepted,
            }
        ))
        .is_err());
        assert!(classify_workspace_node_close_result(Ok(
            SessionLifecycleCommandResult::Accepted {
                receipt,
                state: SessionLifecycleOperationState::ReconciliationRequired {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "close outcome unknown",
                        "close-test",
                    ),
                },
            }
        ))
        .is_err());
        assert!(classify_workspace_node_close_result(Ok(
            SessionLifecycleCommandResult::OutcomeUnknown {
                request_id: "workspace-node-close-test".to_string(),
            }
        ))
        .is_err());
        assert!(
            classify_workspace_node_close_result(Ok(SessionLifecycleCommandResult::Rejected(
                SessionLifecycleRejection::InvalidState
            )))
            .is_err()
        );
    }
}
