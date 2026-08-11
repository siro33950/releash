use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::repository::normalize_repo_path;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
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
        origin: AgentSessionOrigin,
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
    Tombstoned {
        reason: AgentSessionRemovalAuthorization,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionOrigin {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionOriginError {
    EmptyWorkflowExecutionId,
    EmptyNodeExecutionId,
}

impl AgentSessionOrigin {
    pub(crate) fn workflow_node(
        workflow_execution_id: impl Into<String>,
        node_execution_id: impl Into<String>,
    ) -> Result<Self, AgentSessionOriginError> {
        let workflow_execution_id = workflow_execution_id.into();
        if workflow_execution_id.trim().is_empty() {
            return Err(AgentSessionOriginError::EmptyWorkflowExecutionId);
        }
        let node_execution_id = node_execution_id.into();
        if node_execution_id.trim().is_empty() {
            return Err(AgentSessionOriginError::EmptyNodeExecutionId);
        }
        Ok(Self::WorkflowNode {
            workflow_execution_id,
            node_execution_id,
        })
    }

    pub(crate) fn is_standalone(&self) -> bool {
        matches!(self, Self::Standalone)
    }

    pub(crate) fn workflow_execution_id(&self) -> Option<&str> {
        match self {
            Self::Standalone => None,
            Self::WorkflowNode {
                workflow_execution_id,
                ..
            } => Some(workflow_execution_id),
        }
    }

    pub(crate) fn node_execution_id(&self) -> Option<&str> {
        match self {
            Self::Standalone => None,
            Self::WorkflowNode {
                node_execution_id, ..
            } => Some(node_execution_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionCreationError {
    Identity,
    Workspace,
    Worktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRehydrateError {
    EmptyStream,
    FirstEventNotCreated,
    DuplicateCreated,
    InvalidEventSequence,
    Tombstoned(AgentSessionRemovalAuthorization),
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
    Standalone,
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
    NotArchived,
    NotPaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRemovalAuthorization {
    ArchiveFallbackDelete,
    ExplicitDelete,
    GarbageCollection,
    WorkflowLaunchRollback,
}

impl AgentSessionRemovalAuthorization {
    pub(crate) fn tombstone_event(self) -> AgentSessionLifecycleEvent {
        AgentSessionLifecycleEvent::Tombstoned { reason: self }
    }
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
}

impl AgentSessionOperations {
    pub(crate) fn for_state(origin: &AgentSessionOrigin, lifecycle: AgentSessionLifecycle) -> Self {
        let standalone = origin.is_standalone();
        Self {
            can_archive: standalone && lifecycle != AgentSessionLifecycle::Archived,
            can_restore: standalone && lifecycle == AgentSessionLifecycle::Archived,
            can_delete: standalone && lifecycle == AgentSessionLifecycle::Archived,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionRemovalError {
    Standalone,
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
    origin: AgentSessionOrigin,
    lifecycle: AgentSessionLifecycle,
    provider_session_id: Option<String>,
    transcript_ref: Option<String>,
    initial_instruction_admitted: bool,
    last_exit_abnormal: bool,
    uncommitted_events: Vec<AgentSessionLifecycleEvent>,
}

impl AgentSession {
    pub(crate) fn rehydrate(
        events: &[AgentSessionLifecycleEvent],
    ) -> Result<Self, AgentSessionRehydrateError> {
        let (first, remaining) = events
            .split_first()
            .ok_or(AgentSessionRehydrateError::EmptyStream)?;
        let AgentSessionLifecycleEvent::Created {
            id,
            workspace,
            worktree_path,
            provider,
            origin,
        } = first
        else {
            return Err(AgentSessionRehydrateError::FirstEventNotCreated);
        };
        let mut session = Self::create(
            id.clone(),
            workspace.clone(),
            worktree_path.clone(),
            *provider,
            origin.clone(),
        )
        .map_err(|_| AgentSessionRehydrateError::InvalidEventSequence)?;
        session.take_uncommitted_events();
        for event in remaining {
            match event {
                AgentSessionLifecycleEvent::Created { .. } => {
                    return Err(AgentSessionRehydrateError::DuplicateCreated);
                }
                AgentSessionLifecycleEvent::ProviderSessionAssociated {
                    provider_session_id,
                    transcript_ref,
                } => {
                    let outcome = session
                        .associate_provider_session(
                            provider_session_id.clone(),
                            transcript_ref.as_deref(),
                        )
                        .map_err(|_| AgentSessionRehydrateError::InvalidEventSequence)?;
                    if outcome == AgentSessionMutationOutcome::AlreadyApplied {
                        return Err(AgentSessionRehydrateError::InvalidEventSequence);
                    }
                }
                AgentSessionLifecycleEvent::LifecycleChanged {
                    lifecycle,
                    last_exit_abnormal,
                } => {
                    let transition_allowed = matches!(
                        (session.lifecycle, *lifecycle),
                        (
                            AgentSessionLifecycle::Open,
                            AgentSessionLifecycle::Paused | AgentSessionLifecycle::Archived
                        ) | (
                            AgentSessionLifecycle::Paused,
                            AgentSessionLifecycle::Open | AgentSessionLifecycle::Archived
                        ) | (AgentSessionLifecycle::Archived, AgentSessionLifecycle::Open)
                    );
                    if !transition_allowed {
                        return Err(AgentSessionRehydrateError::InvalidEventSequence);
                    }
                    if *lifecycle == AgentSessionLifecycle::Archived
                        && (session.provider_session_id.is_none()
                            || !session.origin.is_standalone())
                    {
                        return Err(AgentSessionRehydrateError::InvalidEventSequence);
                    }
                    if *lifecycle == AgentSessionLifecycle::Paused
                        && session.provider_session_id.is_none()
                    {
                        return Err(AgentSessionRehydrateError::InvalidEventSequence);
                    }
                    session.lifecycle = *lifecycle;
                    session.last_exit_abnormal = *last_exit_abnormal;
                }
                AgentSessionLifecycleEvent::InitialInstructionAdmitted => {
                    let outcome = session
                        .admit_initial_instruction()
                        .map_err(|_| AgentSessionRehydrateError::InvalidEventSequence)?;
                    if outcome == AgentSessionInitialInstructionOutcome::AlreadyAdmitted {
                        return Err(AgentSessionRehydrateError::InvalidEventSequence);
                    }
                }
                AgentSessionLifecycleEvent::Tombstoned { reason } => {
                    return Err(AgentSessionRehydrateError::Tombstoned(*reason));
                }
            }
            session.take_uncommitted_events();
        }
        Ok(session)
    }

    pub(crate) fn create(
        id: impl Into<String>,
        workspace: WorkspaceIdentity,
        worktree_path: impl Into<String>,
        provider: ProviderKind,
        origin: AgentSessionOrigin,
    ) -> Result<Self, AgentSessionCreationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(AgentSessionCreationError::Identity);
        }
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
            origin: origin.clone(),
        };
        Ok(Self {
            id,
            workspace,
            worktree_path,
            provider,
            origin,
            lifecycle: AgentSessionLifecycle::Open,
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
            last_exit_abnormal: false,
            uncommitted_events: vec![created],
        })
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

    pub(crate) fn origin(&self) -> &AgentSessionOrigin {
        &self.origin
    }

    pub(crate) fn lifecycle(&self) -> AgentSessionLifecycle {
        self.lifecycle
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

    pub(crate) fn last_exit_abnormal(&self) -> bool {
        self.last_exit_abnormal
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
        if self.provider_session_id.is_none() {
            return AgentSessionProcessExitOutcome::GcRequired;
        }
        if self.lifecycle == AgentSessionLifecycle::Paused {
            return AgentSessionProcessExitOutcome::AlreadyPaused;
        }
        self.lifecycle = AgentSessionLifecycle::Paused;
        self.last_exit_abnormal = exit_code != Some(0);
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
        if !self.origin.is_standalone() {
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
        if self.lifecycle != AgentSessionLifecycle::Archived {
            return Err(AgentSessionRecoveryError::NotArchived);
        }
        Ok(())
    }

    pub(crate) fn authorize_resume(&self) -> Result<(), AgentSessionRecoveryError> {
        if self.lifecycle != AgentSessionLifecycle::Paused {
            return Err(AgentSessionRecoveryError::NotPaused);
        }
        Ok(())
    }

    pub(crate) fn admit_initial_instruction(
        &mut self,
    ) -> Result<AgentSessionInitialInstructionOutcome, AgentSessionInitialInstructionError> {
        if self.origin.is_standalone() {
            return Err(AgentSessionInitialInstructionError::Standalone);
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
        if !self.origin.is_standalone() {
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
        if !self.origin.is_standalone() {
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
        if self.provider_session_id.is_some() {
            return Err(AgentSessionRemovalError::ProviderSessionKnown);
        }
        Ok(AgentSessionRemovalAuthorization::GarbageCollection)
    }

    pub(crate) fn authorize_workflow_launch_rollback(
        &self,
    ) -> Result<AgentSessionRemovalAuthorization, AgentSessionRemovalError> {
        if self.origin.is_standalone() {
            return Err(AgentSessionRemovalError::Standalone);
        }
        Ok(AgentSessionRemovalAuthorization::WorkflowLaunchRollback)
    }
}
