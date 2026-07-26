#[cfg(test)]
use std::fs::{self, OpenOptions};
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::execution_store;
use crate::domain::local_event::{
    LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository,
    SessionProjectionRecord, WorkflowExecutionProjectionRecord,
};
#[cfg(test)]
use crate::domain::workflow::WorkflowExecutionRecord;
use crate::domain::workflow::{
    ExecutionListFilter, WorkflowError, WorkflowExecutionId, WorkflowExecutionRepository,
    WorkflowExecutionSummary, WorkflowPageRequest,
};

#[cfg(test)]
use super::mapper;

#[cfg(test)]
const EXECUTIONS_SUBDIR: &str = "workflow_executions";

#[derive(Clone)]
pub(crate) struct WorkflowExecutionFileRepository {
    source: WorkflowExecutionReadSource,
}

#[derive(Clone)]
enum WorkflowExecutionReadSource {
    #[cfg(test)]
    Legacy(PathBuf),
    Canonical(Arc<dyn LocalEventTransactionRepository>),
}

impl WorkflowExecutionFileRepository {
    #[cfg(test)]
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            source: WorkflowExecutionReadSource::Legacy(data_dir.into()),
        }
    }

    pub(crate) fn with_authority(repository: Arc<dyn LocalEventTransactionRepository>) -> Self {
        Self {
            source: WorkflowExecutionReadSource::Canonical(repository),
        }
    }

    #[cfg(test)]
    fn legacy_data_dir(&self) -> &Path {
        match &self.source {
            WorkflowExecutionReadSource::Legacy(data_dir) => data_dir,
            WorkflowExecutionReadSource::Canonical(_) => {
                panic!("legacy workflow metadata operation used with canonical authority")
            }
        }
    }

    fn query_canonical(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, WorkflowError> {
        #[cfg(not(test))]
        let WorkflowExecutionReadSource::Canonical(repository) = &self.source;
        #[cfg(test)]
        let repository = match &self.source {
            WorkflowExecutionReadSource::Canonical(repository) => repository,
            WorkflowExecutionReadSource::Legacy(_) => {
                return Err(WorkflowError::external(
                    "canonical workflow projection authority is not active",
                ));
            }
        };
        let repository = repository.clone();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            WorkflowError::external(format!(
                                "failed to create workflow projection read runtime: {error}"
                            ))
                        })?
                        .block_on(repository.query(request))
                        .map_err(|error| {
                            WorkflowError::external(format!(
                                "workflow SQLite projection read failed: {error}"
                            ))
                        })
                })
                .join()
                .map_err(|_| {
                    WorkflowError::external("workflow SQLite projection read worker panicked")
                })?
        })
    }

    fn canonical_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<execution_store::WorkflowExecutionMetadata>, WorkflowError> {
        let result = self.query_canonical(LocalEventQuery::SessionProjectionByIdentity {
            session_id: workflow_projection_key(execution_id.as_str()),
        })?;
        let LocalEventQueryResult::SessionProjectionByIdentity(projection) = result else {
            return Err(WorkflowError::external(
                "workflow SQLite projection returned the wrong result type",
            ));
        };
        projection
            .map(|projection| {
                decode_canonical_workflow_projection(&projection.projection, execution_id.as_str())
            })
            .transpose()
            .map(Option::flatten)
            .map_err(WorkflowError::external)
    }

    fn canonical_executions(
        &self,
    ) -> Result<Vec<execution_store::WorkflowExecutionMetadata>, WorkflowError> {
        let mut after_session_id = None;
        let mut executions = Vec::new();
        loop {
            let result = self.query_canonical(LocalEventQuery::SessionProjectionPage {
                limit: 200,
                after_session_id: after_session_id.clone(),
            })?;
            let LocalEventQueryResult::SessionProjectionPage(page) = result else {
                return Err(WorkflowError::external(
                    "workflow SQLite projection page returned the wrong result type",
                ));
            };
            let page_len = page.len();
            for projection in page {
                after_session_id = Some(projection.session_id.clone());
                let Some(execution_id) = projection.session_id.strip_prefix("workflow:") else {
                    continue;
                };
                let execution_id =
                    WorkflowExecutionId::new(execution_id.to_string()).map_err(|_| {
                        WorkflowError::external(
                            "workflow SQLite projection namespace contains an invalid identity",
                        )
                    })?;
                if let Some(execution) = decode_canonical_workflow_projection(
                    &projection.projection,
                    execution_id.as_str(),
                )
                .map_err(WorkflowError::external)?
                {
                    executions.push(execution);
                }
            }
            if page_len < 200 {
                break;
            }
        }
        Ok(executions)
    }

    #[cfg(test)]
    fn executions_dir(&self) -> PathBuf {
        self.legacy_data_dir().join(EXECUTIONS_SUBDIR)
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
        let metadata = mapper::workflow_execution_record_to_metadata(execution);
        let json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            WorkflowError::external(format!(
                "failed to serialize workflow execution metadata: {e}"
            ))
        })?;
        atomic_write(&self.execution_file_path(&execution_id), &json).map_err(|e| {
            WorkflowError::external(format!("failed to write workflow execution metadata: {e}"))
        })
    }
}

fn workflow_projection_key(execution_id: &str) -> String {
    format!("workflow:{execution_id}")
}

fn decode_canonical_workflow_projection(
    projection: &SessionProjectionRecord,
    expected_execution_id: &str,
) -> Result<Option<execution_store::WorkflowExecutionMetadata>, String> {
    match projection {
        SessionProjectionRecord::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(
            execution,
        )) if execution.execution_id == expected_execution_id => Ok(Some(
            execution_store::workflow_execution_metadata(execution),
        )),
        SessionProjectionRecord::WorkflowExecution(
            WorkflowExecutionProjectionRecord::Deleted { execution_id },
        ) if execution_id == expected_execution_id => Ok(None),
        _ => Err("workflow SQLite projection invariant failed".to_string()),
    }
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
        #[cfg(test)]
        if let WorkflowExecutionReadSource::Legacy(data_dir) = &self.source {
            let scan = execution_store::scan_valid_execution_metadata(data_dir);
            if !scan.is_complete {
                return Err(WorkflowError::external(
                    "workflow projection source is incomplete",
                ));
            }
            return Ok(execution_store::project_executions_to_summaries(
                scan.executions,
                &filter,
            ));
        }
        let executions = self.canonical_executions()?;
        Ok(execution_store::project_executions_to_summaries(
            executions, &filter,
        ))
    }

    fn list_executions_page(
        &self,
        filter: ExecutionListFilter,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        #[cfg(test)]
        if let WorkflowExecutionReadSource::Legacy(data_dir) = &self.source {
            return Ok(execution_store::project_valid_execution_metadata_page(
                data_dir,
                &filter,
                page.offset,
                page.limit,
            ));
        }
        Ok(self
            .list_executions(filter)?
            .into_iter()
            .skip(page.offset)
            .take(page.limit)
            .collect())
    }

    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        #[cfg(test)]
        if let WorkflowExecutionReadSource::Legacy(data_dir) = &self.source {
            return execution_store::read_valid_execution_metadata(data_dir, execution_id.as_str())
                .map(|execution| {
                    execution.map(|metadata| WorkflowExecutionSummary::from(&metadata))
                })
                .map_err(WorkflowError::external);
        }
        self.canonical_execution(execution_id)
            .map(|execution| execution.map(|metadata| WorkflowExecutionSummary::from(&metadata)))
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
            interruption_reason: None,
            resume_from_node: None,
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
    fn execution_page_projects_only_the_requested_sorted_window() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowExecutionFileRepository::new(tmp.path());
        for (seed, completed_at) in [(10, 10.0), (11, 30.0), (12, 20.0)] {
            let id = format!("00000000-0000-4000-8000-{seed:012}");
            let mut record = execution(&id, "/repo", ExecutionStatus::Completed);
            record.updated_at = completed_at;
            record.completed_at = Some(completed_at);
            repo.persist(&record).unwrap();
        }

        let page = repo
            .list_executions_page(
                ExecutionListFilter::default(),
                WorkflowPageRequest::new(1, 1),
            )
            .unwrap();

        assert_eq!(page.len(), 1);
        assert_eq!(page[0].execution_id, "00000000-0000-4000-8000-000000000012");
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
