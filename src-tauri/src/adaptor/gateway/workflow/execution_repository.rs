use std::collections::HashMap;
#[cfg(test)]
use std::fs::{self, OpenOptions};
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use crate::adaptor::gateway::workflow::execution_store;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
#[cfg(test)]
use crate::adaptor::gateway::workflow::pending_command::PendingCommandStore;
#[cfg(test)]
use crate::domain::workflow::WorkflowExecutionRecord;
use crate::domain::workflow::{
    ExecutionListFilter, WorkflowError, WorkflowExecutionId, WorkflowExecutionRepository,
    WorkflowExecutionSummary,
};

#[cfg(test)]
use super::mapper;

#[cfg(test)]
const EXECUTIONS_SUBDIR: &str = "workflow_executions";

#[derive(Debug, Clone)]
pub(crate) struct WorkflowExecutionFileRepository {
    data_dir: PathBuf,
}

impl WorkflowExecutionFileRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub(crate) fn scan_gc_metadata(&self) -> execution_store::WorkflowExecutionMetadataScan {
        execution_store::scan_valid_execution_metadata(&self.data_dir)
    }

    pub(crate) fn read_gc_metadata(
        &self,
        execution_id: &str,
    ) -> Result<Option<execution_store::WorkflowExecutionMetadata>, String> {
        execution_store::read_valid_execution_metadata(&self.data_dir, execution_id)
    }

    #[cfg(test)]
    pub(crate) fn gc_delete_paths(&self, execution_id: &str) -> Vec<PathBuf> {
        let mut paths = vec![execution_store::workflow_execution_metadata_path(
            &self.data_dir,
            execution_id,
        )];
        paths.extend(WorkflowEventLog::new(&self.data_dir).gc_delete_paths(execution_id));
        paths.extend(workflow_artifact_paths(&self.data_dir, execution_id));
        paths.extend(
            PendingCommandStore::new(&self.data_dir).gc_delete_paths_for_execution(execution_id),
        );
        paths
    }

    pub(crate) fn gc_delete_paths_with_pending_index(
        &self,
        execution_id: &str,
        pending_paths_by_execution: &HashMap<String, Vec<PathBuf>>,
    ) -> Vec<PathBuf> {
        let mut paths = vec![execution_store::workflow_execution_metadata_path(
            &self.data_dir,
            execution_id,
        )];
        paths.extend(WorkflowEventLog::new(&self.data_dir).gc_delete_paths(execution_id));
        paths.extend(workflow_artifact_paths(&self.data_dir, execution_id));
        if let Some(pending_paths) = pending_paths_by_execution.get(execution_id) {
            paths.extend(pending_paths.iter().cloned());
        }
        paths
    }

    #[cfg(test)]
    fn executions_dir(&self) -> PathBuf {
        self.data_dir.join(EXECUTIONS_SUBDIR)
    }

    #[cfg(test)]
    fn execution_file_path(&self, execution_id: &WorkflowExecutionId) -> PathBuf {
        self.executions_dir().join(format!("{execution_id}.json"))
    }

    #[cfg(test)]
    fn persist(&self, execution: &WorkflowExecutionRecord) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution.execution_id.clone())?;
        fs::create_dir_all(self.executions_dir()).map_err(|e| {
            WorkflowError::external(format!("failed to create workflow_executions dir: {e}"))
        })?;
        let legacy = mapper::workflow_execution_record_to_metadata(execution);
        let json = serde_json::to_string_pretty(&legacy).map_err(|e| {
            WorkflowError::external(format!(
                "failed to serialize workflow execution metadata: {e}"
            ))
        })?;
        atomic_write(&self.execution_file_path(&execution_id), &json).map_err(|e| {
            WorkflowError::external(format!("failed to write workflow execution metadata: {e}"))
        })
    }
}

fn workflow_artifact_paths(data_dir: &std::path::Path, execution_id: &str) -> Vec<PathBuf> {
    let workflow_dir = data_dir.join("workflow");
    [
        workflow_dir.join(execution_id),
        workflow_dir.join(format!("{execution_id}.json")),
        workflow_dir.join(format!("{execution_id}.ndjson")),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

impl WorkflowExecutionRepository for WorkflowExecutionFileRepository {
    #[cfg(test)]
    fn register_active(&self, execution: WorkflowExecutionRecord) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution.execution_id.clone())?;
        if execution.status.is_terminal() {
            return Err(WorkflowError::invalid_state(
                "register_active requires non-terminal status",
            ));
        }
        if let Some(existing) =
            self.resolve_active_execution_by_worktree(&execution.worktree_path)?
        {
            if existing != execution_id {
                return Err(WorkflowError::invalid_state(format!(
                    "worktree {} already has active execution {}",
                    execution.worktree_path, existing
                )));
            }
        }
        self.persist(&execution)
    }

    #[cfg(test)]
    fn complete_execution(
        &self,
        execution_id: &WorkflowExecutionId,
        completed: WorkflowExecutionRecord,
    ) -> Result<(), WorkflowError> {
        if completed.execution_id != execution_id.as_str() {
            return Err(WorkflowError::validation(format!(
                "completed execution_id {} does not match {execution_id}",
                completed.execution_id
            )));
        }
        if !completed.status.is_terminal() {
            return Err(WorkflowError::invalid_state(
                "complete_execution requires terminal status",
            ));
        }
        if self.get_execution(execution_id)?.is_none() {
            return Err(WorkflowError::NotFound(execution_id.to_string()));
        }
        self.persist(&completed)
    }

    fn list_executions(
        &self,
        filter: ExecutionListFilter,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        let executions = execution_store::iter_valid_execution_metadata(&self.data_dir);
        Ok(execution_store::project_executions_to_summaries(
            executions, &filter,
        ))
    }

    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self
            .list_executions(ExecutionListFilter {
                status: None,
                worktree_path: None,
            })?
            .into_iter()
            .find(|execution| execution.execution_id == execution_id.as_str()))
    }

    #[cfg(test)]
    fn resolve_active_execution_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowExecutionId>, WorkflowError> {
        self.list_executions(ExecutionListFilter {
            status: Some(crate::domain::workflow::ExecutionStatusFilter::Active),
            worktree_path: Some(worktree_path.to_string()),
        })?
        .into_iter()
        .next()
        .map(|execution| WorkflowExecutionId::new(execution.execution_id))
        .transpose()
    }

    fn resolve_worktree_by_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<String>, WorkflowError> {
        Ok(self
            .get_execution(execution_id)?
            .map(|execution| execution.worktree_path))
    }
}

#[cfg(test)]
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
        })?;
    let tmp = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
    use tempfile::TempDir;

    fn execution(
        execution_id: &str,
        worktree_path: &str,
        status: ExecutionStatus,
    ) -> WorkflowExecutionRecord {
        WorkflowExecutionRecord {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: worktree_path.to_string(),
            current_node: Some("node".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: status.is_terminal().then_some(2.0),
            error_reason: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    fn write_legacy_run_metadata(data_dir: &Path, execution: &WorkflowExecutionRecord) {
        let legacy_dir = data_dir.join("workflow_runs");
        fs::create_dir_all(&legacy_dir).unwrap();
        let metadata = mapper::workflow_execution_record_to_metadata(execution);
        let json = serde_json::to_string_pretty(&metadata).unwrap();
        fs::write(
            legacy_dir.join(format!("{}.json", execution.execution_id)),
            json,
        )
        .unwrap();
    }

    #[test]
    fn persists_execution_metadata_shape() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowExecutionFileRepository::new(tmp.path());
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").unwrap();

        repo.register_active(execution(
            execution_id.as_str(),
            "/repo",
            ExecutionStatus::Running,
        ))
        .unwrap();

        let path = tmp
            .path()
            .join("workflow_executions")
            .join(format!("{execution_id}.json"));
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["executionId"], execution_id.as_str());
        assert_eq!(json["status"], "running");
        assert_eq!(json["createdFrom"], "desktop_ui");
    }

    #[test]
    fn active_resolution_uses_persisted_metadata() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowExecutionFileRepository::new(tmp.path());
        let active = WorkflowExecutionId::new("00000000-0000-4000-8000-000000000002").unwrap();
        let done = WorkflowExecutionId::new("00000000-0000-4000-8000-000000000003").unwrap();
        repo.register_active(execution(
            active.as_str(),
            "/repo/a",
            ExecutionStatus::Running,
        ))
        .unwrap();
        repo.register_active(execution(
            done.as_str(),
            "/repo/b",
            ExecutionStatus::Running,
        ))
        .unwrap();
        repo.complete_execution(
            &done,
            execution(done.as_str(), "/repo/b", ExecutionStatus::Completed),
        )
        .unwrap();

        assert_eq!(
            repo.resolve_active_execution_by_worktree("/repo/a")
                .unwrap()
                .as_ref(),
            Some(&active)
        );
        assert_eq!(
            repo.resolve_active_execution_by_worktree("/repo/b")
                .unwrap(),
            None
        );
        assert_eq!(
            repo.resolve_worktree_by_execution(&done)
                .unwrap()
                .as_deref(),
            Some("/repo/b")
        );
    }

    #[test]
    fn legacy_workflow_runs_metadata_is_not_read() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowExecutionFileRepository::new(tmp.path());
        let legacy = WorkflowExecutionId::new("00000000-0000-4000-8000-000000000004").unwrap();
        write_legacy_run_metadata(
            tmp.path(),
            &execution(legacy.as_str(), "/repo/legacy", ExecutionStatus::Running),
        );

        assert!(repo
            .list_executions(ExecutionListFilter::default())
            .unwrap()
            .is_empty());
        assert_eq!(repo.get_execution(&legacy).unwrap(), None);
        assert_eq!(repo.resolve_worktree_by_execution(&legacy).unwrap(), None);
    }
}
