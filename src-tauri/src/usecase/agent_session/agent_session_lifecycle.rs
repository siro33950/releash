use std::sync::Arc;

use crate::domain::agent_session::aggregates::{
    AgentSessionArchiveOutcome, AgentSessionMutationOutcome, AgentSessionOpenAction,
    AgentSessionProcessExitOutcome, AgentSessionRecoveryResult, ManagedPtyPresence,
};
use crate::domain::agent_session::{
    ProviderAgentLaunchGateway, ProviderAgentTerminalGateway, ProviderAvailabilityReader,
    ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{ProviderLifecycleScope, ProviderLifecycleSlotId};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::usecase::provider_lifecycle::ProviderHookHealthUsecase;
use crate::usecase::provider_lifecycle::{ProviderLifecycleUsecase, ProviderLifecycleUsecaseError};

use super::{AgentSessionChangeNotifier, AgentSessionUsecase, AgentSessionUsecaseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionOpenOutcome {
    Attached,
    Resumed,
    Restored,
    Paused,
    Indeterminate,
    GarbageCollected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionGarbageCollectionOutcome {
    Retained,
    GarbageCollected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionLifecycleUsecaseError {
    NotFound,
    InvalidOperation,
    Conflict,
    StorageUnavailable,
    LaunchUnavailable,
    TerminalUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionLifecycleUsecase {
    sessions: Arc<AgentSessionUsecase>,
    lifecycle: Arc<ProviderLifecycleUsecase>,
    launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
    availability: Arc<dyn ProviderAvailabilityReader>,
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    change_notifier: Arc<dyn AgentSessionChangeNotifier>,
}

impl AgentSessionLifecycleUsecase {
    pub(crate) fn new(
        sessions: Arc<AgentSessionUsecase>,
        lifecycle: Arc<ProviderLifecycleUsecase>,
        launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
        availability: Arc<dyn ProviderAvailabilityReader>,
        terminal: Arc<dyn ProviderAgentTerminalGateway>,
        hook_health: Arc<ProviderHookHealthUsecase>,
        change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    ) -> Self {
        Self {
            sessions,
            lifecycle,
            launch_gateway,
            availability,
            terminal,
            hook_health,
            change_notifier,
        }
    }

    pub(crate) async fn open(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AgentSessionOpenOutcome, AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        self.open_locked(agent_session_id, rows, cols, caller_request_id)
            .await
    }

    async fn open_locked(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AgentSessionOpenOutcome, AgentSessionLifecycleUsecaseError> {
        let session = self.required(agent_session_id).await?;
        let owner = session.session().terminal_surface_owner();
        let presence = self
            .terminal
            .presence(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        match session.session().open_action(presence) {
            AgentSessionOpenAction::Attach => Ok(AgentSessionOpenOutcome::Attached),
            AgentSessionOpenAction::Indeterminate => Ok(AgentSessionOpenOutcome::Indeterminate),
            AgentSessionOpenAction::RemainPaused => Ok(AgentSessionOpenOutcome::Paused),
            AgentSessionOpenAction::Restore => {
                self.restore_locked(agent_session_id, rows, cols, caller_request_id)
                    .await
            }
            AgentSessionOpenAction::GarbageCollect => {
                self.remove_gc(
                    session.session().terminal_surface_owner(),
                    agent_session_id,
                    caller_request_id,
                )
                .await?;
                Ok(AgentSessionOpenOutcome::GarbageCollected)
            }
            AgentSessionOpenAction::Resume => {
                match self
                    .spawn_resume(agent_session_id, rows, cols, caller_request_id)
                    .await
                {
                    Ok(()) => Ok(AgentSessionOpenOutcome::Resumed),
                    Err(error) => {
                        self.sessions
                            .observe_process_exit(
                                agent_session_id,
                                None,
                                &format!("{caller_request_id}.auto-resume-failed"),
                            )
                            .await
                            .map_err(map_session_error)?;
                        log::warn!("automatic AgentSession resume failed: {error:?}");
                        Ok(AgentSessionOpenOutcome::Paused)
                    }
                }
            }
        }
    }

    pub(crate) async fn resume(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AgentSessionOpenOutcome, AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        session
            .session()
            .authorize_resume()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        let owner = session.session().terminal_surface_owner();
        if let Err(error) = self
            .spawn_resume(agent_session_id, rows, cols, caller_request_id)
            .await
        {
            self.sessions
                .complete_resume(
                    agent_session_id,
                    AgentSessionRecoveryResult::Failed,
                    &format!("{caller_request_id}.failed"),
                )
                .await
                .map_err(map_session_error)?;
            return Err(error);
        }
        if let Err(error) = self
            .sessions
            .complete_resume(
                agent_session_id,
                AgentSessionRecoveryResult::Succeeded,
                caller_request_id,
            )
            .await
            .map_err(map_session_error)
        {
            self.rollback_spawned_resume(owner, agent_session_id)
                .await?;
            return Err(error);
        }
        Ok(AgentSessionOpenOutcome::Resumed)
    }

    pub(crate) async fn restore(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AgentSessionOpenOutcome, AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        self.restore_locked(agent_session_id, rows, cols, caller_request_id)
            .await
    }

    async fn restore_locked(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<AgentSessionOpenOutcome, AgentSessionLifecycleUsecaseError> {
        let session = self.required(agent_session_id).await?;
        session
            .session()
            .authorize_restore()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        let owner = session.session().terminal_surface_owner();
        if let Err(error) = self
            .spawn_resume(agent_session_id, rows, cols, caller_request_id)
            .await
        {
            self.sessions
                .complete_restore(
                    agent_session_id,
                    AgentSessionRecoveryResult::Failed,
                    &format!("{caller_request_id}.failed"),
                )
                .await
                .map_err(map_session_error)?;
            return Err(error);
        }
        if let Err(error) = self
            .sessions
            .complete_restore(
                agent_session_id,
                AgentSessionRecoveryResult::Succeeded,
                caller_request_id,
            )
            .await
            .map_err(map_session_error)
        {
            self.rollback_spawned_resume(owner, agent_session_id)
                .await?;
            return Err(error);
        }
        Ok(AgentSessionOpenOutcome::Restored)
    }

    pub(crate) async fn archive(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionArchiveOutcome, AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        let mut candidate = session.session().clone();
        let outcome = candidate
            .archive()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        if outcome != AgentSessionArchiveOutcome::Archived {
            return Ok(outcome);
        }
        self.terminal
            .stop_preserving_checkpoint(&session.session().terminal_surface_owner())
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        self.release_launch_binding(agent_session_id).await?;
        self.launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
        self.sessions
            .archive(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error)
    }

    pub(crate) async fn stop_workflow_owned_preserving_checkpoint(
        &self,
        agent_session_id: &str,
        node_execution_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        session
            .session()
            .authorize_workflow_stop(node_execution_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        self.terminal
            .stop_preserving_checkpoint(&session.session().terminal_surface_owner())
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        let outcome = self
            .sessions
            .stop_workflow_owned(agent_session_id, node_execution_id, caller_request_id)
            .await
            .map_err(map_session_error)?;
        if outcome == AgentSessionMutationOutcome::Applied {
            self.change_notifier
                .agent_session_changed(session.session().worktree_path());
        }
        self.release_launch_binding(agent_session_id).await?;
        self.launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)
    }

    pub(crate) async fn delete(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        let owner = session.session().terminal_surface_owner();
        session
            .session()
            .authorize_delete()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        self.remove_explicit(owner, agent_session_id, caller_request_id)
            .await
    }

    pub(crate) async fn confirm_archive_fallback_delete(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        let owner = session.session().terminal_surface_owner();
        session
            .session()
            .authorize_archive_fallback_delete()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        self.terminal
            .delete(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        self.release_launch_binding(agent_session_id).await?;
        self.launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
        self.sessions
            .confirm_archive_fallback_delete(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error)
    }

    pub(crate) async fn observe_process_exit(
        &self,
        agent_session_id: &str,
        runtime_generation: u64,
        exit_code: Option<i32>,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        if !self
            .terminal
            .is_current_runtime_generation(
                &session.session().terminal_surface_owner(),
                runtime_generation,
            )
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?
        {
            return Ok(());
        }
        let scope = ProviderLifecycleScope::new(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::Corrupt)?;
        match self
            .lifecycle
            .active_launch_id(session.session().provider(), &scope)
            .await
        {
            Ok(Some(launch_id)) => {
                if let Err(error) = self
                    .hook_health
                    .record_unavailable(
                        session.session().provider(),
                        launch_id.as_str(),
                        crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
                        &format!("{caller_request_id}.hook-session-start-missing"),
                    )
                    .await
                {
                    log::warn!(
                        "failed to persist confirmed Provider SessionStart absence: {error:?}"
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("failed to correlate Provider process exit with launch: {error:?}");
            }
        }
        let outcome = self
            .sessions
            .observe_process_exit(agent_session_id, exit_code, caller_request_id)
            .await
            .map_err(map_session_error)?;
        if outcome == AgentSessionProcessExitOutcome::Paused {
            self.change_notifier
                .agent_session_changed(session.session().worktree_path());
        }
        if outcome != AgentSessionProcessExitOutcome::GcRequired {
            self.release_launch_binding(agent_session_id).await?;
            self.launch_gateway
                .cleanup(agent_session_id)
                .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
            return Ok(());
        }
        let session = self.required(agent_session_id).await?;
        let owner = session.session().terminal_surface_owner();
        let presence = self
            .terminal
            .presence(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        if presence != ManagedPtyPresence::ConfirmedAbsent {
            return Err(AgentSessionLifecycleUsecaseError::InvalidOperation);
        }
        self.remove_gc(owner, agent_session_id, caller_request_id)
            .await
    }

    pub(crate) async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError> {
        let _operation = self
            .sessions
            .lock_operation(agent_session_id)
            .await
            .map_err(map_session_error)?;
        let session = self.required(agent_session_id).await?;
        let owner = session.session().terminal_surface_owner();
        let presence = self
            .terminal
            .presence(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        if session.session().authorize_gc(presence).is_err() {
            return Ok(AgentSessionGarbageCollectionOutcome::Retained);
        }
        self.remove_gc(owner, agent_session_id, caller_request_id)
            .await?;
        Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected)
    }

    async fn spawn_resume(
        &self,
        agent_session_id: &str,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let session = self.required(agent_session_id).await?;
        let provider_session_id = session
            .session()
            .provider_session_id_for_recovery()
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        let executable = self
            .availability
            .resolved_executable(session.session().provider())
            .ok_or(AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
        let slot_id = ProviderLifecycleSlotId::new(crate::other::id::unique_simple_id())
            .map_err(|_| AgentSessionLifecycleUsecaseError::Corrupt)?;
        let scope = ProviderLifecycleScope::new(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::InvalidOperation)?;
        self.lifecycle
            .release_scope(&scope)
            .await
            .map_err(map_lifecycle_error)?;
        let armed = self
            .lifecycle
            .arm(slot_id, session.session().provider(), scope)
            .await
            .map_err(map_lifecycle_error)?;
        let prepared = match self.launch_gateway.prepare(
            &armed,
            executable,
            ProviderSessionLaunch::resume(provider_session_id)
                .map_err(|_| AgentSessionLifecycleUsecaseError::Corrupt)?,
            session.session().worktree_path(),
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                self.cleanup_unspawned_resume(agent_session_id).await?;
                return Err(AgentSessionLifecycleUsecaseError::LaunchUnavailable);
            }
        };
        if let Err(error) = self
            .hook_health
            .record_launch_with_warning(
                session.session().provider(),
                armed.slot_id().as_str(),
                None,
                &format!("{caller_request_id}.hook-launch"),
            )
            .await
        {
            log::warn!(
                "failed to persist Provider Hook launch health without blocking AgentSession resume: {error:?}"
            );
        }
        let initial_hook_warning = prepared.initial_hook_warning();
        let owner = session.session().terminal_surface_owner();
        if self
            .terminal
            .spawn(
                owner.clone(),
                session.session().worktree_path(),
                prepared.into_process(),
                rows,
                cols,
            )
            .is_err()
        {
            self.rollback_spawned_resume(owner, agent_session_id)
                .await?;
            return Err(AgentSessionLifecycleUsecaseError::TerminalUnavailable);
        }
        if let Some(warning) = initial_hook_warning {
            if let Err(error) = self
                .hook_health
                .record_unavailable(
                    session.session().provider(),
                    armed.slot_id().as_str(),
                    warning,
                    &format!("{caller_request_id}.hook-initial-warning"),
                )
                .await
            {
                log::warn!(
                    "failed to persist initial Provider Hook warning without blocking AgentSession resume: {error:?}"
                );
            }
        }
        Ok(())
    }

    async fn remove_explicit(
        &self,
        owner: TerminalSurfaceOwner,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        self.terminal
            .delete(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        self.release_launch_binding(agent_session_id).await?;
        self.launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
        self.sessions
            .delete(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error)
    }

    async fn rollback_spawned_resume(
        &self,
        owner: TerminalSurfaceOwner,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let terminal_result = self
            .terminal
            .stop_preserving_checkpoint(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable);
        let lifecycle_result = self.release_launch_binding(agent_session_id).await;
        let launch_result = self
            .launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable);
        terminal_result?;
        lifecycle_result?;
        launch_result
    }

    async fn cleanup_unspawned_resume(
        &self,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let lifecycle_result = self.release_launch_binding(agent_session_id).await;
        let launch_result = self
            .launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable);
        lifecycle_result?;
        launch_result
    }

    async fn remove_gc(
        &self,
        owner: TerminalSurfaceOwner,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        self.terminal
            .delete(&owner)
            .map_err(|_| AgentSessionLifecycleUsecaseError::TerminalUnavailable)?;
        self.release_launch_binding(agent_session_id).await?;
        self.launch_gateway
            .cleanup(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::LaunchUnavailable)?;
        self.sessions
            .garbage_collect(
                agent_session_id,
                ManagedPtyPresence::ConfirmedAbsent,
                caller_request_id,
            )
            .await
            .map_err(map_session_error)
    }

    async fn release_launch_binding(
        &self,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionLifecycleUsecaseError> {
        let scope = ProviderLifecycleScope::new(agent_session_id)
            .map_err(|_| AgentSessionLifecycleUsecaseError::Corrupt)?;
        self.lifecycle
            .release_scope(&scope)
            .await
            .map(|_| ())
            .map_err(map_lifecycle_error)
    }

    async fn required(
        &self,
        agent_session_id: &str,
    ) -> Result<
        crate::domain::agent_session::repository::VersionedAgentSession,
        AgentSessionLifecycleUsecaseError,
    > {
        self.sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(AgentSessionLifecycleUsecaseError::NotFound)
    }
}

fn map_session_error(error: AgentSessionUsecaseError) -> AgentSessionLifecycleUsecaseError {
    match error {
        AgentSessionUsecaseError::NotFound => AgentSessionLifecycleUsecaseError::NotFound,
        AgentSessionUsecaseError::InvalidOperation => {
            AgentSessionLifecycleUsecaseError::InvalidOperation
        }
        AgentSessionUsecaseError::Conflict
        | AgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            AgentSessionLifecycleUsecaseError::Conflict
        }
        AgentSessionUsecaseError::Unavailable => {
            AgentSessionLifecycleUsecaseError::StorageUnavailable
        }
        AgentSessionUsecaseError::Corrupt => AgentSessionLifecycleUsecaseError::Corrupt,
    }
}

fn map_lifecycle_error(error: ProviderLifecycleUsecaseError) -> AgentSessionLifecycleUsecaseError {
    match error {
        ProviderLifecycleUsecaseError::InvalidInput => {
            AgentSessionLifecycleUsecaseError::InvalidOperation
        }
        ProviderLifecycleUsecaseError::StorageUnavailable => {
            AgentSessionLifecycleUsecaseError::StorageUnavailable
        }
        ProviderLifecycleUsecaseError::Corrupt => AgentSessionLifecycleUsecaseError::Corrupt,
    }
}
