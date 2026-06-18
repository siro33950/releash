use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::adaptor::gateway::workflow::run as legacy_run;
use crate::domain::workflow::{
    RunId, RunListFilter, WorkflowError, WorkflowRunRecord, WorkflowRunRepository,
    WorkflowRunSummary,
};

use super::mapper;

const RUNS_SUBDIR: &str = "workflow_runs";

#[derive(Debug, Clone)]
pub(crate) struct WorkflowRunFileRepository {
    data_dir: PathBuf,
}

impl WorkflowRunFileRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    fn runs_dir(&self) -> PathBuf {
        self.data_dir.join(RUNS_SUBDIR)
    }

    fn run_file_path(&self, run_id: &RunId) -> PathBuf {
        self.runs_dir().join(format!("{run_id}.json"))
    }

    fn persist(&self, run: &WorkflowRunRecord) -> Result<(), WorkflowError> {
        let run_id = RunId::new(run.run_id.clone())?;
        fs::create_dir_all(self.runs_dir()).map_err(|e| {
            WorkflowError::external(format!("failed to create workflow_runs dir: {e}"))
        })?;
        let legacy = mapper::domain_run_record_to_legacy(run);
        let json = serde_json::to_string_pretty(&legacy).map_err(|e| {
            WorkflowError::external(format!("failed to serialize workflow run metadata: {e}"))
        })?;
        atomic_write(&self.run_file_path(&run_id), &json).map_err(|e| {
            WorkflowError::external(format!("failed to write workflow run metadata: {e}"))
        })
    }
}

impl WorkflowRunRepository for WorkflowRunFileRepository {
    fn register_active(&self, run: WorkflowRunRecord) -> Result<(), WorkflowError> {
        let run_id = RunId::new(run.run_id.clone())?;
        if run.status.is_terminal() {
            return Err(WorkflowError::invalid_state(
                "register_active requires non-terminal status",
            ));
        }
        if let Some(existing) = self.resolve_active_run_by_worktree(&run.worktree_path)? {
            if existing != run_id {
                return Err(WorkflowError::AlreadyActive(format!(
                    "worktree {} already has active run {}",
                    run.worktree_path, existing
                )));
            }
        }
        self.persist(&run)
    }

    fn complete_run(
        &self,
        run_id: &RunId,
        completed: WorkflowRunRecord,
    ) -> Result<(), WorkflowError> {
        if completed.run_id != run_id.as_str() {
            return Err(WorkflowError::validation(format!(
                "completed run_id {} does not match {run_id}",
                completed.run_id
            )));
        }
        if !completed.status.is_terminal() {
            return Err(WorkflowError::invalid_state(
                "complete_run requires terminal status",
            ));
        }
        if self.get_run(run_id)?.is_none() {
            return Err(WorkflowError::NotFound(run_id.to_string()));
        }
        self.persist(&completed)
    }

    fn cancel_reservation(&self, run_id: &RunId) -> Result<(), WorkflowError> {
        let path = self.run_file_path(run_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WorkflowError::external(format!(
                "failed to remove workflow run metadata: {e}"
            ))),
        }
    }

    fn list_runs(&self, filter: RunListFilter) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
        let legacy_filter = mapper::domain_run_filter_to_legacy(filter);
        let runs = legacy_run::iter_valid_run_metadata(&self.data_dir);
        Ok(legacy_run::project_runs_to_summaries(runs, &legacy_filter)
            .into_iter()
            .map(mapper::legacy_run_summary_to_domain)
            .collect())
    }

    fn get_run(&self, run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
        Ok(self
            .list_runs(RunListFilter {
                status: None,
                worktree_path: None,
            })?
            .into_iter()
            .find(|run| run.run_id == run_id.as_str()))
    }

    fn resolve_active_run_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<RunId>, WorkflowError> {
        self.list_runs(RunListFilter {
            status: Some(crate::domain::workflow::RunStatusFilter::Active),
            worktree_path: Some(worktree_path.to_string()),
        })?
        .into_iter()
        .next()
        .map(|run| RunId::new(run.run_id))
        .transpose()
    }

    fn resolve_worktree_by_run(&self, run_id: &RunId) -> Result<Option<String>, WorkflowError> {
        Ok(self.get_run(run_id)?.map(|run| run.worktree_path))
    }
}

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
    use crate::domain::workflow::{RunStatus, TriggerSource};
    use tempfile::TempDir;

    fn run(run_id: &str, worktree_path: &str, status: RunStatus) -> WorkflowRunRecord {
        WorkflowRunRecord {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: Some("task".to_string()),
            status,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: status.is_terminal().then_some(2.0),
            error_reason: None,
        }
    }

    #[test]
    fn persists_existing_run_metadata_shape() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowRunFileRepository::new(tmp.path());
        let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();

        repo.register_active(run(run_id.as_str(), "/repo", RunStatus::Running))
            .unwrap();

        let path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{run_id}.json"));
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["runId"], run_id.as_str());
        assert_eq!(json["status"], "running");
        assert_eq!(json["triggerSource"], "desktop_ui");
    }

    #[test]
    fn active_resolution_uses_persisted_metadata() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowRunFileRepository::new(tmp.path());
        let active = RunId::new("00000000-0000-4000-8000-000000000002").unwrap();
        let done = RunId::new("00000000-0000-4000-8000-000000000003").unwrap();
        repo.register_active(run(active.as_str(), "/repo/a", RunStatus::Running))
            .unwrap();
        repo.register_active(run(done.as_str(), "/repo/b", RunStatus::Running))
            .unwrap();
        repo.complete_run(&done, run(done.as_str(), "/repo/b", RunStatus::Completed))
            .unwrap();

        assert_eq!(
            repo.resolve_active_run_by_worktree("/repo/a")
                .unwrap()
                .as_ref(),
            Some(&active)
        );
        assert_eq!(
            repo.resolve_active_run_by_worktree("/repo/b").unwrap(),
            None
        );
        assert_eq!(
            repo.resolve_worktree_by_run(&done).unwrap().as_deref(),
            Some("/repo/b")
        );
    }
}
