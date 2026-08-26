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
    fn source_format(
        &self,
        _file_stem: &str,
    ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
        Ok(crate::domain::workflow::WorkflowSourceFormat::Yaml)
    }
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

/// workflow diagnostics の診断対象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowDiagnosticsTarget {
    /// 適用済み Workflow の config directory。実際の path は gateway 実装が所有する。
    AppliedConfigDirectory,
    /// 呼び出し時に指定された workflow source directory。Facet base は gateway が解決する。
    Directory(std::path::PathBuf),
}

impl WorkflowDiagnosticsTarget {
    /// 入口（Tauri command / local API）が受け取る optional な directory 文字列を対象へ写す。
    /// 絶対 path であることだけを検証する。空文字列と空白のみの文字列も相対 path として
    /// 弾かれる。前後の空白は path の一部として保持する（空白を含む directory 名は正当で
    /// あり、trim すると実在する対象を診断できなくなる）。対象 directory の実在検査は
    /// filesystem I/O を所有する gateway が行う。
    pub fn from_optional_directory(dir: Option<String>) -> Result<Self, WorkflowError> {
        let Some(dir) = dir else {
            return Ok(Self::AppliedConfigDirectory);
        };
        let path = std::path::PathBuf::from(&dir);
        if !path.is_absolute() {
            return Err(WorkflowError::Validation(format!(
                "diagnostics target directory must be an absolute path: {dir}"
            )));
        }
        Ok(Self::Directory(path))
    }
}

pub trait WorkflowDiagnosticsGateway: Send + Sync {
    fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<serde_json::Value, WorkflowError>;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_診断対象_directory省略時は適用済みconfigになる() {
        // Given
        let directory = None;

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(directory).unwrap();

        // Then
        assert_eq!(target, WorkflowDiagnosticsTarget::AppliedConfigDirectory);
    }

    #[test]
    fn test_診断対象_空directoryを拒否する() {
        // Given
        let directories = [String::new(), "   ".to_string()];

        // When
        let targets = directories
            .map(|directory| WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)));

        // Then
        assert!(targets
            .into_iter()
            .all(|target| matches!(target, Err(WorkflowError::Validation(_)))));
    }

    #[test]
    fn test_診断対象_相対directoryを拒否する() {
        // Given
        let directory = "workflows".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory));

        // Then
        assert!(matches!(target, Err(WorkflowError::Validation(_))));
    }

    #[test]
    fn test_診断対象_先頭空白付き相対directoryを拒否する() {
        // Given
        let directory = " workflows".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory));

        // Then
        assert!(matches!(target, Err(WorkflowError::Validation(_))));
    }

    #[test]
    fn test_診断対象_絶対directoryを受理する() {
        // Given
        let directory = "/tmp/x".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)).unwrap();

        // Then
        assert_eq!(
            target,
            WorkflowDiagnosticsTarget::Directory(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn test_診断対象_絶対directoryの末尾空白を保持する() {
        // Given
        let directory = "/tmp/workflows ".to_string();

        // When
        let target = WorkflowDiagnosticsTarget::from_optional_directory(Some(directory)).unwrap();

        // Then
        assert_eq!(
            target,
            WorkflowDiagnosticsTarget::Directory(PathBuf::from("/tmp/workflows "))
        );
    }
}
