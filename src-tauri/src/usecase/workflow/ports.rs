use crate::domain::workflow::{RunId, WorkflowDefinition, WorkflowError, WorkflowStateSnapshot};

use super::command::{
    AbortRunCommand, ApprovalCommand, ResolvedStartRunCommand, SubmitOutputCommand,
};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEventDraft {
    pub run_id: String,
    pub event_kind: String,
    pub timestamp: f64,
    pub payload: serde_json::Value,
}

pub trait WorkflowEventRepository: Send + Sync {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError>;
    #[cfg(test)]
    fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError>;
    fn read(&self, run_id: &RunId) -> Result<Vec<WorkflowEventDraft>, WorkflowError>;
}

pub trait WorkflowStateProjectionRepository: Send + Sync {
    fn get_state(&self, run_id: &RunId) -> Result<Option<WorkflowStateSnapshot>, WorkflowError>;
}

pub trait WorkflowStepDetailProjectionRepository: Send + Sync {
    fn get_step_detail(
        &self,
        run_id: &RunId,
        node_name: &str,
        run_index: Option<u32>,
    ) -> Result<Option<serde_json::Value>, WorkflowError>;
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

#[derive(Debug, Clone, PartialEq)]
pub struct PendingWorkflowCommand {
    pub command_id: String,
    pub run_id: String,
    pub requested_at: f64,
    pub payload: serde_json::Value,
}

pub trait PendingWorkflowCommandRepository: Send + Sync {
    fn write_pending(&self, command: PendingWorkflowCommand) -> Result<(), WorkflowError>;
    #[cfg(test)]
    fn list_pending(&self) -> Result<Vec<PendingWorkflowCommand>, WorkflowError>;
    #[cfg(test)]
    fn mark_processed(&self, command_id: &str) -> Result<(), WorkflowError>;
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
pub trait WorkflowStartRunGateway: Send + Sync {
    async fn resolve_start_run_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError>;
    async fn resolve_start_run_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<WorkflowDefinition, WorkflowError>;
    async fn start_resolved_run(
        &self,
        command: ResolvedStartRunCommand,
    ) -> Result<String, WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowAbortRunGateway: Send + Sync {
    async fn abort_run(&self, command: AbortRunCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowApprovalGateway: Send + Sync {
    async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowSubmitOutputGateway: Send + Sync {
    async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowPendingRuntimeCommandGateway: Send + Sync {
    async fn dispatch_pending_command(
        &self,
        command: PendingRuntimeCommand,
    ) -> PendingRuntimeCommandOutcome;
}

#[async_trait::async_trait]
pub trait WorkflowTurnCompleteGateway: Send + Sync {
    async fn is_session_running(&self, chat_session_id: &str) -> bool;
    async fn pickup_pending_submit_outputs(&self);
    async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteCommand,
    ) -> Result<(), WorkflowError>;
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
    async fn get_state_by_run_id(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError>;
    async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowRuntimeShutdownGateway: Send + Sync {
    async fn shutdown_active_commands(&self);
}

#[async_trait::async_trait]
pub trait WorkflowApprovalChatGateway: Send + Sync {
    async fn resolve_approval_chat_target(
        &self,
        run_id: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError>;
    async fn validate_approval_chat_instruction(
        &self,
        chat_session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowError>;
}

pub trait WorkflowRuntimeCommandGateway:
    WorkflowStartRunGateway
    + WorkflowAbortRunGateway
    + WorkflowApprovalGateway
    + WorkflowSubmitOutputGateway
    + WorkflowPendingRuntimeCommandGateway
    + WorkflowTurnCompleteGateway
    + WorkflowStallObservedGateway
    + WorkflowRuntimeStateGateway
    + WorkflowRuntimeShutdownGateway
    + WorkflowApprovalChatGateway
{
}

impl<T> WorkflowRuntimeCommandGateway for T where
    T: WorkflowStartRunGateway
        + WorkflowAbortRunGateway
        + WorkflowApprovalGateway
        + WorkflowSubmitOutputGateway
        + WorkflowPendingRuntimeCommandGateway
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

#[derive(Debug, Clone, PartialEq)]
pub struct PendingRuntimeCommand {
    pub run_id: String,
    pub request_id: String,
    pub requested_at: f64,
    pub payload: PendingRuntimeCommandPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingRuntimeCommandPayload {
    Approve {
        node_name: String,
        comment: Option<String>,
    },
    Abort {
        node_name: Option<String>,
    },
    SubmitOutput {
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRuntimeCommandOutcome {
    Accepted,
    RejectedFinal(String),
    RetryableFailure(String),
}
