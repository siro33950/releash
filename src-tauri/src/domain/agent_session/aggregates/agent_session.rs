use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::repository::normalize_repo_path;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workflow::ExecutionTreeLaunch;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionLifecycle {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionLifecycleEvent {
    Created {
        id: String,
        workspace: WorkspaceIdentity,
        worktree_path: String,
        provider: ProviderKind,
        tree_location: AgentSessionTreeLocation,
    },
    ProviderSessionAssociated {
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    LifecycleChanged {
        lifecycle: AgentSessionLifecycle,
        last_exit_abnormal: bool,
    },
    InitialInstructionAdmitted,
}

/// AgentSession が属する実行木と NodeExecution の必須の所在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionTreeLocation {
    tree_id: String,
    node_execution_id: String,
    launched_as: ExecutionTreeLaunch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionTreeLocationError {
    EmptyTreeId,
    EmptyNodeExecutionId,
    SessionTreeRootIdentityMismatch,
}

impl AgentSessionTreeLocation {
    pub(crate) fn session_tree_root(
        agent_session_id: impl Into<String>,
    ) -> Result<Self, AgentSessionTreeLocationError> {
        let agent_session_id = agent_session_id.into();
        let agent_session_id = agent_session_id.trim();
        if agent_session_id.is_empty() {
            return Err(AgentSessionTreeLocationError::EmptyTreeId);
        }
        Ok(Self {
            tree_id: agent_session_id.to_string(),
            node_execution_id: agent_session_id.to_string(),
            launched_as: ExecutionTreeLaunch::Session,
        })
    }

    pub(crate) fn for_agent_session(
        tree_id: impl Into<String>,
        node_execution_id: impl Into<String>,
        launched_as: ExecutionTreeLaunch,
        agent_session_id: &str,
    ) -> Result<Self, AgentSessionTreeLocationError> {
        let tree_id = tree_id.into();
        let node_execution_id = node_execution_id.into();
        Self::validate_session_root_identity(
            &tree_id,
            &node_execution_id,
            launched_as,
            agent_session_id,
        )?;
        Self::new(tree_id, node_execution_id, launched_as)
    }

    pub(crate) fn workflow_node(
        tree_id: impl Into<String>,
        node_execution_id: impl Into<String>,
    ) -> Result<Self, AgentSessionTreeLocationError> {
        Self::new(tree_id, node_execution_id, ExecutionTreeLaunch::Workflow)
    }

    fn new(
        tree_id: impl Into<String>,
        node_execution_id: impl Into<String>,
        launched_as: ExecutionTreeLaunch,
    ) -> Result<Self, AgentSessionTreeLocationError> {
        let tree_id = tree_id.into();
        let tree_id = tree_id.trim();
        if tree_id.is_empty() {
            return Err(AgentSessionTreeLocationError::EmptyTreeId);
        }
        let node_execution_id = node_execution_id.into();
        let node_execution_id = node_execution_id.trim();
        if node_execution_id.is_empty() {
            return Err(AgentSessionTreeLocationError::EmptyNodeExecutionId);
        }
        Ok(Self {
            tree_id: tree_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            launched_as,
        })
    }

    fn validate_agent_session_identity(
        &self,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionTreeLocationError> {
        Self::validate_session_root_identity(
            &self.tree_id,
            &self.node_execution_id,
            self.launched_as,
            agent_session_id,
        )
    }

    fn validate_session_root_identity(
        tree_id: &str,
        node_execution_id: &str,
        launched_as: ExecutionTreeLaunch,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionTreeLocationError> {
        if launched_as == ExecutionTreeLaunch::Session
            && (tree_id != agent_session_id || node_execution_id != agent_session_id)
        {
            return Err(AgentSessionTreeLocationError::SessionTreeRootIdentityMismatch);
        }
        Ok(())
    }

    pub(crate) fn tree_id(&self) -> &str {
        &self.tree_id
    }

    pub(crate) fn node_execution_id(&self) -> &str {
        &self.node_execution_id
    }

    pub(crate) fn launched_as(&self) -> ExecutionTreeLaunch {
        self.launched_as
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionCreationError {
    Identity,
    Workspace,
    Worktree,
    SessionTreeRootIdentityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionMutationOutcome {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRecoveryResult {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionInitialInstructionOutcome {
    Admitted,
    AlreadyAdmitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionInitialInstructionError {
    NotWorkflowOwned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionAssociationError {
    EmptyProviderSessionId,
    EmptyTranscriptReference,
    ProviderSessionMismatch,
    TranscriptMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionProcessExitOutcome {
    Paused,
    AlreadyPaused,
    AlreadyArchived,
    GcRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionArchiveOutcome {
    Archived,
    AlreadyArchived,
    DeleteConfirmationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionArchiveError {
    WorkflowOwned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRecoveryError {
    WorkflowOwned,
    NotArchived,
    NotPaused,
    ProviderSessionUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionExecutionTreeNodeStopError {
    NodeExecutionMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRemovalAuthorization {
    ArchiveFallbackDelete,
    ExplicitDelete,
    GarbageCollection,
    WorkflowLaunchRollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedPtyPresence {
    ConfirmedAbsent,
    Live,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionOpenAction {
    Attach,
    Resume,
    Restore,
    RemainPaused,
    Indeterminate,
    GarbageCollect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentSessionOperations {
    pub(crate) can_archive: bool,
    pub(crate) can_restore: bool,
    pub(crate) can_delete: bool,
    pub(crate) can_resume: bool,
}

pub(crate) fn derive_agent_session_operations(
    launched_as: ExecutionTreeLaunch,
    archived: bool,
    exited: bool,
    provider_session_known: bool,
) -> AgentSessionOperations {
    let user_owned = launched_as == ExecutionTreeLaunch::Session;
    let paused = !archived && exited;
    AgentSessionOperations {
        can_archive: user_owned && !archived,
        can_restore: user_owned && archived,
        can_delete: user_owned && archived,
        can_resume: paused && provider_session_known,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRemovalError {
    NotWorkflowOwned,
    WorkflowOwned,
    NotArchived,
    ProviderSessionKnown,
    PtyNotConfirmedAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSession {
    id: String,
    workspace: WorkspaceIdentity,
    worktree_path: String,
    provider: ProviderKind,
    tree_location: AgentSessionTreeLocation,
    lifecycle: AgentSessionLifecycle,
    provider_session_id: Option<String>,
    transcript_ref: Option<String>,
    initial_instruction_admitted: bool,
    last_exit_abnormal: bool,
    last_exit_code: Option<i32>,
    uncommitted_events: Vec<AgentSessionLifecycleEvent>,
}

impl AgentSession {
    pub(crate) fn create(
        id: impl Into<String>,
        workspace: WorkspaceIdentity,
        worktree_path: impl Into<String>,
        provider: ProviderKind,
        tree_location: AgentSessionTreeLocation,
    ) -> Result<Self, AgentSessionCreationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(AgentSessionCreationError::Identity);
        }
        tree_location
            .validate_agent_session_identity(&id)
            .map_err(|_| AgentSessionCreationError::SessionTreeRootIdentityMismatch)?;
        if workspace.as_str().trim().is_empty() {
            return Err(AgentSessionCreationError::Workspace);
        }
        let worktree_path = worktree_path.into();
        if worktree_path.trim().is_empty() {
            return Err(AgentSessionCreationError::Worktree);
        }
        let worktree_path = normalize_repo_path(&worktree_path);
        let created = AgentSessionLifecycleEvent::Created {
            id: id.clone(),
            workspace: workspace.clone(),
            worktree_path: worktree_path.clone(),
            provider,
            tree_location: tree_location.clone(),
        };
        Ok(Self {
            id,
            workspace,
            worktree_path,
            provider,
            tree_location,
            lifecycle: AgentSessionLifecycle::Open,
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
            last_exit_abnormal: false,
            last_exit_code: None,
            uncommitted_events: vec![created],
        })
    }

    /// 事実ログから導出済みの lifecycle と異常終了状態だけを復元する。
    /// provider session 参照など、別の事実から復元する属性は変更しない。
    pub(crate) fn restore_derived_lifecycle(
        &mut self,
        lifecycle: AgentSessionLifecycle,
        last_exit_abnormal: bool,
    ) {
        debug_assert!(self.uncommitted_events.is_empty());
        self.lifecycle = lifecycle;
        self.last_exit_abnormal = last_exit_abnormal;
        self.last_exit_code = None;
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn workspace(&self) -> &WorkspaceIdentity {
        &self.workspace
    }

    pub(crate) fn worktree_path(&self) -> &str {
        &self.worktree_path
    }

    pub(crate) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(crate) fn tree_location(&self) -> &AgentSessionTreeLocation {
        &self.tree_location
    }

    pub(crate) fn lifecycle(&self) -> AgentSessionLifecycle {
        self.lifecycle
    }

    #[cfg(test)]
    pub(crate) fn operations(&self) -> AgentSessionOperations {
        derive_agent_session_operations(
            self.tree_location.launched_as,
            self.lifecycle == AgentSessionLifecycle::Archived,
            self.lifecycle == AgentSessionLifecycle::Paused,
            self.provider_session_id.is_some(),
        )
    }

    pub(crate) fn uncommitted_events(&self) -> &[AgentSessionLifecycleEvent] {
        &self.uncommitted_events
    }

    pub(crate) fn take_uncommitted_events(&mut self) -> Vec<AgentSessionLifecycleEvent> {
        std::mem::take(&mut self.uncommitted_events)
    }

    pub(crate) fn terminal_surface_owner(&self) -> TerminalSurfaceOwner {
        TerminalSurfaceOwner::session_from_validated(self.workspace.clone(), &self.id)
    }

    pub(crate) fn associate_provider_session(
        &mut self,
        provider_session_id: impl Into<String>,
        transcript_ref: Option<&str>,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionAssociationError> {
        let provider_session_id = provider_session_id.into();
        if provider_session_id.trim().is_empty() {
            return Err(AgentSessionAssociationError::EmptyProviderSessionId);
        }
        if transcript_ref.is_some_and(|reference| reference.trim().is_empty()) {
            return Err(AgentSessionAssociationError::EmptyTranscriptReference);
        }
        if self
            .provider_session_id
            .as_deref()
            .is_some_and(|current| current != provider_session_id)
        {
            return Err(AgentSessionAssociationError::ProviderSessionMismatch);
        }
        if matches!(
            (self.transcript_ref.as_deref(), transcript_ref),
            (Some(current), Some(candidate)) if current != candidate
        ) {
            return Err(AgentSessionAssociationError::TranscriptMismatch);
        }
        if self.provider_session_id.as_deref() == Some(provider_session_id.as_str())
            && (transcript_ref.is_none() || self.transcript_ref.as_deref() == transcript_ref)
        {
            return Ok(AgentSessionMutationOutcome::AlreadyApplied);
        }
        let transcript_ref = transcript_ref.map(str::to_string);
        self.provider_session_id = Some(provider_session_id.clone());
        self.transcript_ref = transcript_ref.clone();
        self.uncommitted_events
            .push(AgentSessionLifecycleEvent::ProviderSessionAssociated {
                provider_session_id,
                transcript_ref,
            });
        Ok(AgentSessionMutationOutcome::Applied)
    }

    pub(crate) fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    pub(crate) fn transcript_ref(&self) -> Option<&str> {
        self.transcript_ref.as_deref()
    }

    pub(crate) fn initial_instruction_admitted(&self) -> bool {
        self.initial_instruction_admitted
    }

    #[cfg(test)]
    pub(crate) fn last_exit_abnormal(&self) -> bool {
        self.last_exit_abnormal
    }

    pub(crate) fn last_exit_code(&self) -> Option<i32> {
        self.last_exit_code
    }

    pub(crate) fn open_action(&self, pty_presence: ManagedPtyPresence) -> AgentSessionOpenAction {
        match pty_presence {
            ManagedPtyPresence::Live => AgentSessionOpenAction::Attach,
            ManagedPtyPresence::Unknown => AgentSessionOpenAction::Indeterminate,
            ManagedPtyPresence::ConfirmedAbsent => match self.lifecycle {
                AgentSessionLifecycle::Paused => AgentSessionOpenAction::RemainPaused,
                AgentSessionLifecycle::Archived => AgentSessionOpenAction::Restore,
                AgentSessionLifecycle::Open if self.provider_session_id.is_some() => {
                    AgentSessionOpenAction::Resume
                }
                AgentSessionLifecycle::Open
                    if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow =>
                {
                    AgentSessionOpenAction::RemainPaused
                }
                AgentSessionLifecycle::Open => AgentSessionOpenAction::GarbageCollect,
            },
        }
    }

    pub(crate) fn observe_provider_process_exit(
        &mut self,
        exit_code: Option<i32>,
    ) -> AgentSessionProcessExitOutcome {
        if self.lifecycle == AgentSessionLifecycle::Archived {
            return AgentSessionProcessExitOutcome::AlreadyArchived;
        }
        if self.provider_session_id.is_none()
            && self.tree_location.launched_as == ExecutionTreeLaunch::Session
        {
            return AgentSessionProcessExitOutcome::GcRequired;
        }
        if self.lifecycle == AgentSessionLifecycle::Paused {
            return AgentSessionProcessExitOutcome::AlreadyPaused;
        }
        self.lifecycle = AgentSessionLifecycle::Paused;
        self.last_exit_abnormal = exit_code != Some(0);
        self.last_exit_code = exit_code;
        self.uncommitted_events
            .push(AgentSessionLifecycleEvent::LifecycleChanged {
                lifecycle: AgentSessionLifecycle::Paused,
                last_exit_abnormal: self.last_exit_abnormal,
            });
        AgentSessionProcessExitOutcome::Paused
    }

    pub(crate) fn archive(
        &mut self,
    ) -> Result<AgentSessionArchiveOutcome, AgentSessionArchiveError> {
        if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionArchiveError::WorkflowOwned);
        }
        if self.lifecycle == AgentSessionLifecycle::Archived {
            return Ok(AgentSessionArchiveOutcome::AlreadyArchived);
        }
        if self.provider_session_id.is_none() {
            return Ok(AgentSessionArchiveOutcome::DeleteConfirmationRequired);
        }
        self.lifecycle = AgentSessionLifecycle::Archived;
        self.uncommitted_events
            .push(AgentSessionLifecycleEvent::LifecycleChanged {
                lifecycle: AgentSessionLifecycle::Archived,
                last_exit_abnormal: self.last_exit_abnormal,
            });
        Ok(AgentSessionArchiveOutcome::Archived)
    }

    pub(crate) fn complete_restore(
        &mut self,
        result: AgentSessionRecoveryResult,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionRecoveryError> {
        self.authorize_restore()?;
        match result {
            AgentSessionRecoveryResult::Succeeded => {
                self.lifecycle = AgentSessionLifecycle::Open;
                self.last_exit_abnormal = false;
                self.last_exit_code = None;
                self.uncommitted_events
                    .push(AgentSessionLifecycleEvent::LifecycleChanged {
                        lifecycle: AgentSessionLifecycle::Open,
                        last_exit_abnormal: false,
                    });
                Ok(AgentSessionMutationOutcome::Applied)
            }
            AgentSessionRecoveryResult::Failed => Ok(AgentSessionMutationOutcome::AlreadyApplied),
        }
    }

    pub(crate) fn complete_resume(
        &mut self,
        result: AgentSessionRecoveryResult,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionRecoveryError> {
        self.authorize_resume()?;
        match result {
            AgentSessionRecoveryResult::Succeeded => {
                self.lifecycle = AgentSessionLifecycle::Open;
                self.last_exit_abnormal = false;
                self.last_exit_code = None;
                self.uncommitted_events
                    .push(AgentSessionLifecycleEvent::LifecycleChanged {
                        lifecycle: AgentSessionLifecycle::Open,
                        last_exit_abnormal: false,
                    });
                Ok(AgentSessionMutationOutcome::Applied)
            }
            AgentSessionRecoveryResult::Failed => Ok(AgentSessionMutationOutcome::AlreadyApplied),
        }
    }

    pub(crate) fn authorize_restore(&self) -> Result<(), AgentSessionRecoveryError> {
        if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionRecoveryError::WorkflowOwned);
        }
        if self.lifecycle != AgentSessionLifecycle::Archived {
            return Err(AgentSessionRecoveryError::NotArchived);
        }
        Ok(())
    }

    pub(crate) fn authorize_resume(&self) -> Result<(), AgentSessionRecoveryError> {
        if self.lifecycle != AgentSessionLifecycle::Paused {
            return Err(AgentSessionRecoveryError::NotPaused);
        }
        self.provider_session_id_for_recovery().map(|_| ())
    }

    pub(crate) fn provider_session_id_for_recovery(
        &self,
    ) -> Result<&str, AgentSessionRecoveryError> {
        self.provider_session_id
            .as_deref()
            .ok_or(AgentSessionRecoveryError::ProviderSessionUnknown)
    }

    pub(crate) fn admit_initial_instruction(
        &mut self,
    ) -> Result<AgentSessionInitialInstructionOutcome, AgentSessionInitialInstructionError> {
        if self.tree_location.launched_as != ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionInitialInstructionError::NotWorkflowOwned);
        }
        if self.initial_instruction_admitted {
            return Ok(AgentSessionInitialInstructionOutcome::AlreadyAdmitted);
        }
        self.initial_instruction_admitted = true;
        self.uncommitted_events
            .push(AgentSessionLifecycleEvent::InitialInstructionAdmitted);
        Ok(AgentSessionInitialInstructionOutcome::Admitted)
    }

    pub(crate) fn authorize_delete(
        &self,
    ) -> Result<AgentSessionRemovalAuthorization, AgentSessionRemovalError> {
        if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionRemovalError::WorkflowOwned);
        }
        if self.lifecycle != AgentSessionLifecycle::Archived {
            return Err(AgentSessionRemovalError::NotArchived);
        }
        Ok(AgentSessionRemovalAuthorization::ExplicitDelete)
    }

    pub(crate) fn authorize_archive_fallback_delete(
        &self,
    ) -> Result<AgentSessionRemovalAuthorization, AgentSessionRemovalError> {
        if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionRemovalError::WorkflowOwned);
        }
        if self.provider_session_id.is_some() {
            return Err(AgentSessionRemovalError::ProviderSessionKnown);
        }
        Ok(AgentSessionRemovalAuthorization::ArchiveFallbackDelete)
    }

    pub(crate) fn authorize_gc(
        &self,
        pty_presence: ManagedPtyPresence,
    ) -> Result<AgentSessionRemovalAuthorization, AgentSessionRemovalError> {
        if pty_presence != ManagedPtyPresence::ConfirmedAbsent {
            return Err(AgentSessionRemovalError::PtyNotConfirmedAbsent);
        }
        if self.tree_location.launched_as == ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionRemovalError::WorkflowOwned);
        }
        if self.provider_session_id.is_some() {
            return Err(AgentSessionRemovalError::ProviderSessionKnown);
        }
        Ok(AgentSessionRemovalAuthorization::GarbageCollection)
    }

    pub(crate) fn authorize_execution_tree_node_stop(
        &self,
        node_execution_id: &str,
    ) -> Result<(), AgentSessionExecutionTreeNodeStopError> {
        if self.tree_location.node_execution_id != node_execution_id {
            return Err(AgentSessionExecutionTreeNodeStopError::NodeExecutionMismatch);
        }
        Ok(())
    }

    pub(crate) fn stop_for_terminal_execution_tree_node(
        &mut self,
        node_execution_id: &str,
    ) -> Result<AgentSessionMutationOutcome, AgentSessionExecutionTreeNodeStopError> {
        self.authorize_execution_tree_node_stop(node_execution_id)?;
        if self.lifecycle != AgentSessionLifecycle::Open {
            return Ok(AgentSessionMutationOutcome::AlreadyApplied);
        }
        self.lifecycle = AgentSessionLifecycle::Paused;
        self.last_exit_abnormal = false;
        self.last_exit_code = None;
        self.uncommitted_events
            .push(AgentSessionLifecycleEvent::LifecycleChanged {
                lifecycle: AgentSessionLifecycle::Paused,
                last_exit_abnormal: false,
            });
        Ok(AgentSessionMutationOutcome::Applied)
    }

    pub(crate) fn authorize_workflow_launch_rollback(
        &self,
    ) -> Result<AgentSessionRemovalAuthorization, AgentSessionRemovalError> {
        if self.tree_location.launched_as != ExecutionTreeLaunch::Workflow {
            return Err(AgentSessionRemovalError::NotWorkflowOwned);
        }
        Ok(AgentSessionRemovalAuthorization::WorkflowLaunchRollback)
    }
}
