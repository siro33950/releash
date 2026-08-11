#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;
use crate::domain::workflow::{
    WorkflowDefinition, WorkflowError, WorkflowExecution, WorkflowExecutionId, WorkflowPageRequest,
};

use super::command::{
    AbortExecutionCommand, ResolvedStartExecutionCommand, ResumeExecutionCommand,
    StopExecutionCommand,
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
pub trait WorkflowStopExecutionGateway: Send + Sync {
    async fn stop_execution(&self, command: StopExecutionCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowResumeExecutionGateway: Send + Sync {
    async fn resume_execution(&self, command: ResumeExecutionCommand) -> Result<(), WorkflowError>;
}

#[async_trait::async_trait]
pub trait WorkflowRuntimeStateGateway: Send + Sync {
    /// Explicit startup recovery hook. Construction must never invoke this:
    /// composition calls it once only after the fixed local store is verified and
    /// normal mutation admission.
    async fn recover_startup(&self) -> Result<(), WorkflowError>;

    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError>;
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

pub trait WorkflowRuntimeCommandGateway:
    WorkflowStartExecutionGateway
    + WorkflowAbortExecutionGateway
    + WorkflowStopExecutionGateway
    + WorkflowResumeExecutionGateway
    + crate::usecase::workflow::control_plane::WorkflowControlPlaneGateway
    + WorkflowRuntimeStateGateway
    + WorkflowRuntimeShutdownGateway
{
}

impl<T> WorkflowRuntimeCommandGateway for T where
    T: WorkflowStartExecutionGateway
        + WorkflowAbortExecutionGateway
        + WorkflowStopExecutionGateway
        + WorkflowResumeExecutionGateway
        + crate::usecase::workflow::control_plane::WorkflowControlPlaneGateway
        + WorkflowRuntimeStateGateway
        + WorkflowRuntimeShutdownGateway
{
}
