use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionArchiveOutcome, AgentSessionInitialInstructionOutcome,
    AgentSessionMutationOutcome, AgentSessionProcessExitOutcome, AgentSessionRecoveryResult,
    ManagedPtyPresence,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;
use crate::domain::workflow::AgentSessionActivity;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionUsecaseError {
    NotFound,
    InvalidOperation,
    Conflict,
    ProviderSessionAlreadyOwned { agent_session_id: String },
    Unavailable,
    Corrupt,
}

pub(crate) struct AgentSessionCreateRequest {
    pub(crate) agent_session_id: String,
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) tree_location: crate::domain::agent_session::aggregates::AgentSessionTreeLocation,
    pub(crate) admit_initial_instruction: bool,
}

pub(crate) struct AgentSessionActivityObservation {
    pub(crate) outcome: AgentSessionMutationOutcome,
    pub(crate) worktree_path: String,
}

pub(crate) struct AgentSessionUsecase {
    repository: Arc<dyn AgentSessionRepository>,
    operation_locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl AgentSessionUsecase {
    pub(crate) fn new(repository: Arc<dyn AgentSessionRepository>) -> Self {
        Self {
            repository,
            operation_locks: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn lock_operation(
        &self,
        agent_session_id: &str,
    ) -> Result<OwnedMutexGuard<()>, AgentSessionUsecaseError> {
        let lock = {
            let mut locks = self
                .operation_locks
                .lock()
                .map_err(|_| AgentSessionUsecaseError::Corrupt)?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(agent_session_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(agent_session_id.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        Ok(lock.lock_owned().await)
    }

    pub(crate) async fn create(
        &self,
        agent_session_id: &str,
        workspace: WorkspaceIdentity,
        worktree_path: &str,
        provider: ProviderKind,
        tree_location: crate::domain::agent_session::aggregates::AgentSessionTreeLocation,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionUsecaseError> {
        let session = AgentSession::create(
            agent_session_id,
            workspace,
            worktree_path,
            provider,
            tree_location,
        )
        .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.repository
            .create(session, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn create_with_lifecycle_events(
        &self,
        request: AgentSessionCreateRequest,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionUsecaseError> {
        let mut session = AgentSession::create(
            &request.agent_session_id,
            request.workspace,
            &request.worktree_path,
            request.provider,
            request.tree_location,
        )
        .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        if request.admit_initial_instruction {
            session
                .admit_initial_instruction()
                .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        }
        self.repository
            .create_with_lifecycle_events(session, lifecycle_events, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn find(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionUsecaseError> {
        self.repository
            .find(agent_session_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn associate_provider_session(
        &self,
        agent_session_id: &str,
        provider_session_id: &str,
        transcript_ref: Option<&str>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionUsecaseError> {
        let session = self
            .prepare_provider_session_association(
                agent_session_id,
                provider_session_id,
                transcript_ref,
            )
            .await?;
        if session.session().uncommitted_events().is_empty() {
            return Ok(session);
        }
        self.repository
            .save(session, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn prepare_provider_session_association(
        &self,
        agent_session_id: &str,
        provider_session_id: &str,
        transcript_ref: Option<&str>,
    ) -> Result<VersionedAgentSession, AgentSessionUsecaseError> {
        let mut session = self
            .repository
            .find(agent_session_id)
            .await
            .map_err(map_repository_error)?
            .ok_or(AgentSessionUsecaseError::NotFound)?;
        session
            .session_mut()
            .associate_provider_session(provider_session_id, transcript_ref)
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        Ok(session)
    }

    pub(crate) async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        exit_code: Option<i32>,
        caller_request_id: &str,
    ) -> Result<AgentSessionProcessExitOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .observe_provider_process_exit(exit_code);
        if !session.session().uncommitted_events().is_empty() {
            self.repository
                .save(session, caller_request_id)
                .await
                .map_err(map_repository_error)?;
        }
        Ok(outcome)
    }

    pub(crate) async fn observe_activity(
        &self,
        agent_session_id: &str,
        activity: AgentSessionActivity,
        caller_request_id: &str,
    ) -> Result<AgentSessionActivityObservation, AgentSessionUsecaseError> {
        let mut session = self
            .repository
            .find_for_activity(agent_session_id)
            .await
            .map_err(map_repository_error)?
            .ok_or(AgentSessionUsecaseError::NotFound)?;
        let outcome = session.session_mut().observe_activity(activity);
        let worktree_path = session.session().worktree_path().to_string();
        if outcome == AgentSessionMutationOutcome::Applied {
            self.repository
                .save_activity(session, caller_request_id)
                .await
                .map_err(map_repository_error)?;
        }
        Ok(AgentSessionActivityObservation {
            outcome,
            worktree_path,
        })
    }

    pub(crate) async fn stop_for_terminal_execution_tree_node(
        &self,
        agent_session_id: &str,
        node_execution_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .stop_for_terminal_execution_tree_node(node_execution_id)
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.save_if_changed(session, caller_request_id).await?;
        Ok(outcome)
    }

    pub(crate) async fn complete_resume(
        &self,
        agent_session_id: &str,
        result: AgentSessionRecoveryResult,
        caller_request_id: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .complete_resume(result)
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.save_if_changed(session, caller_request_id).await?;
        Ok(outcome)
    }

    pub(crate) async fn complete_restore(
        &self,
        agent_session_id: &str,
        result: AgentSessionRecoveryResult,
        caller_request_id: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .complete_restore(result)
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.save_if_changed(session, caller_request_id).await?;
        Ok(outcome)
    }

    pub(crate) async fn archive(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionArchiveOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .archive()
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.save_if_changed(session, caller_request_id).await?;
        Ok(outcome)
    }

    pub(crate) async fn delete(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionUsecaseError> {
        let session = self.required(agent_session_id).await?;
        let authorization = session
            .session()
            .authorize_delete()
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.repository
            .remove(session, authorization, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn confirm_archive_fallback_delete(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionUsecaseError> {
        let session = self.required(agent_session_id).await?;
        let authorization = session
            .session()
            .authorize_archive_fallback_delete()
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.repository
            .remove(session, authorization, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn garbage_collect(
        &self,
        agent_session_id: &str,
        pty_presence: ManagedPtyPresence,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionUsecaseError> {
        let session = self.required(agent_session_id).await?;
        let authorization = session
            .session()
            .authorize_gc(pty_presence)
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.repository
            .remove(session, authorization, caller_request_id)
            .await
            .map_err(map_repository_error)
    }

    pub(crate) async fn admit_initial_instruction(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionInitialInstructionOutcome, AgentSessionUsecaseError> {
        let mut session = self.required(agent_session_id).await?;
        let outcome = session
            .session_mut()
            .admit_initial_instruction()
            .map_err(|_| AgentSessionUsecaseError::InvalidOperation)?;
        self.save_if_changed(session, caller_request_id).await?;
        Ok(outcome)
    }

    async fn required(
        &self,
        agent_session_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionUsecaseError> {
        self.find(agent_session_id)
            .await?
            .ok_or(AgentSessionUsecaseError::NotFound)
    }

    async fn save_if_changed(
        &self,
        session: VersionedAgentSession,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionUsecaseError> {
        if session.session().uncommitted_events().is_empty() {
            return Ok(());
        }
        self.repository
            .save(session, caller_request_id)
            .await
            .map(|_| ())
            .map_err(map_repository_error)
    }
}

fn map_repository_error(error: AgentSessionRepositoryError) -> AgentSessionUsecaseError {
    match error {
        AgentSessionRepositoryError::Conflict => AgentSessionUsecaseError::Conflict,
        AgentSessionRepositoryError::ProviderSessionAlreadyOwned { agent_session_id } => {
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned { agent_session_id }
        }
        AgentSessionRepositoryError::InvalidRequest => AgentSessionUsecaseError::InvalidOperation,
        AgentSessionRepositoryError::Corrupt => AgentSessionUsecaseError::Corrupt,
        AgentSessionRepositoryError::Unavailable => AgentSessionUsecaseError::Unavailable,
    }
}
