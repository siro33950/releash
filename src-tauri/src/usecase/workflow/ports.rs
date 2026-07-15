use crate::domain::workflow::{
    WorkflowDefinition, WorkflowError, WorkflowExecution, WorkflowExecutionId, WorkflowPageRequest,
    WorkflowRuntimeSnapshot,
};

use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand, SubmitOutputCommand,
};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEventDraft {
    pub execution_id: String,
    pub event_kind: String,
    pub timestamp: f64,
    pub payload: serde_json::Value,
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
pub trait WorkflowApprovalGateway: Send + Sync {
    async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
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

#[async_trait::async_trait]
pub trait WorkflowRuntimeShutdownGateway: Send + Sync {
    async fn shutdown_active_commands(&self);
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
