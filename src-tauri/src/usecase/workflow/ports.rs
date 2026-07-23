use crate::domain::workflow::{
    WorkflowDefinition, WorkflowError, WorkflowExecution, WorkflowExecutionId, WorkflowPageRequest,
    WorkflowRuntimeSnapshot,
};

use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand, ResumeExecutionCommand,
    StopExecutionCommand, SubmitOutputCommand,
};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEventDraft {
    pub execution_id: String,
    pub event_kind: String,
    pub timestamp: f64,
    pub payload: serde_json::Value,
}

/// One-read event-log projection used by Workspace UI queries.
///
/// `definition` is the immutable snapshot persisted by `ExecutionStarted`.  The
/// optional shape keeps existing in-memory/fake repositories source-compatible;
/// production repositories should override `get_execution_with_definition`.
/// `execution.node_executions` preserves `NodeStarted` append order; Workspace
/// presentation projections rely on that order and must not timestamp-sort it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionProjection {
    pub execution: WorkflowExecution,
    pub definition: Option<WorkflowDefinition>,
}

pub trait WorkflowEventRepository: Send + Sync {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError>;
    #[cfg(test)]
    fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError>;
    fn read(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError>;
    fn read_page(
        &self,
        execution_id: &WorkflowExecutionId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read(execution_id).map(|events| {
            events
                .into_iter()
                .skip(page.offset)
                .take(page.limit)
                .collect()
        })
    }
}

pub trait WorkflowExecutionProjectionRepository: Send + Sync {
    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecution>, WorkflowError>;

    fn get_execution_with_definition(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionProjection>, WorkflowError> {
        self.get_execution(execution_id).map(|execution| {
            execution.map(|execution| WorkflowExecutionProjection {
                execution,
                definition: None,
            })
        })
    }

    /// Returns the execution shape needed to build a Workspace tree summary.
    ///
    /// Production persistence adapters should override this method so large
    /// request, command, and Artifact bodies are not materialized while
    /// replaying the append-only log. The default keeps lightweight fakes and
    /// alternate adapters source-compatible.
    fn get_workspace_execution_with_definition(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionProjection>, WorkflowError> {
        self.get_execution_with_definition(execution_id)
    }
}

pub trait WorkflowDefinitionSourceGateway: Send + Sync {
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError>;
    fn save_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError>;
    fn save_source_with_diagnostics(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        self.save_source(source, original_name)
            .map_err(WorkflowSourceSaveError::Workflow)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowSourceSaveError {
    Diagnostics(Vec<serde_json::Value>),
    Workflow(WorkflowError),
}

pub trait ExternalEditorGateway: Send + Sync {
    fn open_workflow(&self, name: &str) -> Result<(), WorkflowError>;
    fn open_facet(&self, kind: &str, key: &str) -> Result<(), WorkflowError>;
}

pub trait WorkflowDiagnosticsGateway: Send + Sync {
    fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError>;
}

pub trait WorkflowConfigPathGateway: Send + Sync {
    fn automation_config_dir(&self) -> Result<String, WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowStartExecutionGateway: Send + Sync {
    async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError>;
    async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowError>;
    async fn start_resolved_execution(
        &self,
        command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowAbortExecutionGateway: Send + Sync {
    async fn abort_execution(&self, command: AbortExecutionCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowStopExecutionGateway: Send + Sync {
    async fn stop_execution(&self, command: StopExecutionCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowResumeExecutionGateway: Send + Sync {
    async fn resume_execution(&self, command: ResumeExecutionCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowApprovalGateway: Send + Sync {
    async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub(crate) trait WorkspaceNodeSessionCloseGateway: Send + Sync {
    async fn close_session(&self, session_id: &str) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowSubmitOutputGateway: Send + Sync {
    async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowTurnCompleteGateway: Send + Sync {
    async fn is_session_running(&self, chat_session_id: &str) -> bool;
    async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteCommand,
    ) -> Result<(), WorkflowError>;

    async fn recover_turn_complete(
        &self,
        _command: WorkflowTurnCompleteRecoveryCommand,
    ) -> Result<WorkflowTurnCompleteRecoveryOutcome, WorkflowError> {
        Err(WorkflowError::external(
            "durable workflow turn-completion recovery is unsupported",
        ))
    }
}

#[async_trait::async_trait]
pub trait WorkflowStallObservedGateway: Send + Sync {
    async fn observe_stall(
        &self,
        command: WorkflowStallObservedCommand,
    ) -> Result<(), WorkflowError>;

    async fn clear_stall(&self, command: WorkflowStallClearedCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowRuntimeStateGateway: Send + Sync {
    /// Explicit startup recovery hook. Construction must never invoke this:
    /// composition calls it once only after verified local-store cutover and
    /// normal mutation admission.
    async fn recover_startup(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError>;
    async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError>;
}

/// Startup recovery admission is owned outside the workflow runtime. The
/// workflow usecase may observe this gate but cannot open it.
#[cfg(test)]
pub trait WorkflowStartupRecoveryAdmission: Send + Sync {
    fn normal_mutation_admitted(&self) -> bool;
    fn migration_blocked(&self) -> bool;
}

#[async_trait::async_trait]
pub trait WorkflowRuntimeShutdownGateway: Send + Sync {
    async fn shutdown_active_commands(&self);

    async fn shutdown_execution_commands(&self, execution_id: &str) {
        let _ = execution_id;
        self.shutdown_active_commands().await;
    }

    async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String>;

    async fn execute_shutdown_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> WorkflowShutdownEffectReadback {
        let _ = (operation_id, effect_identity, owner_revision);
        self.shutdown_execution_commands(execution_id).await;
        WorkflowShutdownEffectReadback::Ambiguous
    }

    async fn read_shutdown_effect(
        &self,
        _operation_id: &str,
        _effect_identity: &str,
        _owner_revision: i64,
        _execution_id: &str,
    ) -> WorkflowShutdownEffectReadback {
        WorkflowShutdownEffectReadback::Ambiguous
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowShutdownEffectReadback {
    Completed,
    ConfirmedNotStarted,
    Ambiguous,
}

#[async_trait::async_trait]
pub trait WorkflowApprovalChatGateway: Send + Sync {
    async fn resolve_approval_chat_target(
        &self,
        execution_id: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError>;
    async fn validate_approval_chat_instruction(
        &self,
        chat_session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowError>;
}

pub trait WorkflowRuntimeCommandGateway:
    WorkflowStartExecutionGateway
    + WorkflowAbortExecutionGateway
    + WorkflowStopExecutionGateway
    + WorkflowResumeExecutionGateway
    + WorkflowApprovalGateway
    + WorkflowSubmitOutputGateway
    + WorkflowTurnCompleteGateway
    + WorkflowStallObservedGateway
    + WorkflowRuntimeStateGateway
    + WorkflowRuntimeShutdownGateway
    + WorkflowApprovalChatGateway
{
}

impl<T> WorkflowRuntimeCommandGateway for T where
    T: WorkflowStartExecutionGateway
        + WorkflowAbortExecutionGateway
        + WorkflowStopExecutionGateway
        + WorkflowResumeExecutionGateway
        + WorkflowApprovalGateway
        + WorkflowSubmitOutputGateway
        + WorkflowTurnCompleteGateway
        + WorkflowStallObservedGateway
        + WorkflowRuntimeStateGateway
        + WorkflowRuntimeShutdownGateway
        + WorkflowApprovalChatGateway
{
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalChatTarget {
    pub chat_session_id: String,
    pub worktree_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnFailureSignal {
    ModelRefusal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnCompleteCommand {
    pub chat_session_id: String,
    pub exit_code: i64,
    pub final_text_parts: Vec<String>,
    pub failure_signal: Option<WorkflowTurnFailureSignal>,
    pub token_usage: Option<WorkflowTurnTokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnCompleteNotification {
    pub chat_session_id: String,
    pub exit_code: i64,
    pub final_text_parts: Vec<String>,
    pub failure_signal: Option<WorkflowTurnFailureSignal>,
    pub token_usage: Option<WorkflowTurnTokenUsage>,
    pub interrupted: bool,
}

/// Canonical ownership coordinates captured in the workflow-owned chat
/// session before its terminal turn is committed. Startup replay uses these
/// coordinates to reject a notification aimed at a different execution,
/// node attempt, or fanout child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnCompleteRecoveryCommand {
    pub notification: WorkflowTurnCompleteNotification,
    pub turn_id: u64,
    pub execution_id: String,
    pub node_execution_id: String,
    pub workflow_name: String,
    pub node_name: String,
    pub attempt: u32,
    pub parent_node_name: Option<String>,
    pub parent_attempt: Option<u32>,
    pub order: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnCompleteRecoveryOutcome {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStallObservedNotification {
    pub chat_session_id: String,
    pub turn_phase: String,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStallClearedNotification {
    pub chat_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStallObservedCommand {
    pub chat_session_id: String,
    pub turn_phase: String,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStallClearedCommand {
    pub chat_session_id: String,
}
