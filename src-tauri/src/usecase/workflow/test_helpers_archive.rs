use crate::domain::workflow::{ExecutionTreeArchiveRepository, WorkflowError};
pub struct NoopArchiveRepository;

#[async_trait::async_trait]
impl ExecutionTreeArchiveRepository for NoopArchiveRepository {
    async fn location(
        &self,
        id: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveCandidate, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveCandidate {
            execution_id: id.into(),
            worktree_path: "/tmp/wt".into(),
            workspace_identity: "/tmp/wt".into(),
            repository_root: None,
        })
    }
    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError> {
        Ok(crate::domain::repository::normalize_repo_path(path))
    }
    async fn record_repository_root(&self, _: &str, _: &str, _: f64) -> Result<(), WorkflowError> {
        unreachable!()
    }
    async fn candidate_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn legacy_session_archive_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }

    async fn worktree_target_page(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn target(
        &self,
        _: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveTarget, WorkflowError> {
        unreachable!()
    }
    fn legacy_archives(
        &self,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }
    fn finish_legacy_migration(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _archived_at: f64,
        _reason: &str,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn restore(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive_snapshot_for(
        &self,
        _execution_ids: &[String],
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveSnapshot, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveSnapshot {
            records: Vec::new(),
        })
    }
}
