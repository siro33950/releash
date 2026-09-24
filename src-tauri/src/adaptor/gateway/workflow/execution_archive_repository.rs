use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::fact_log::{self, FactLogReadBackend};
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::workflow::{
    ExecutionTreeArchiveCandidate, ExecutionTreeArchiveRecord, ExecutionTreeArchiveRepository,
    ExecutionTreeArchiveSnapshot, ExecutionTreeArchiveTarget, ExecutionTreeId, NodeFact,
    WorkflowError,
};

pub(crate) struct ExecutionTreeArchiveFactRepository {
    backend: FactLogReadBackend,
    legacy_path: Option<PathBuf>,
}

impl ExecutionTreeArchiveFactRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            backend: FactLogReadBackend::Live(store),
            legacy_path: Some(data_dir.into().join("workflow_execution_archives.json")),
        }
    }

    pub(crate) fn from_backend(backend: FactLogReadBackend) -> Self {
        Self {
            backend,
            legacy_path: None,
        }
    }

    async fn read_candidate_page(
        &self,
        after: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError> {
        let after = after.unwrap_or("").to_string();
        self.backend.run_indexed(move |connection| {
            let mut statement = connection.prepare(
                "SELECT tree_id, json_extract(detail, '$.root.worktreePath'),
                        json_extract(detail, '$.root.workspaceIdentity'),
                        COALESCE(json_extract(detail, '$.root.repositoryRoot'),
                          (SELECT json_extract(owner.detail, '$.repositoryRoot')
                           FROM node_events owner WHERE owner.tree_id = node_events.tree_id
                             AND (owner.event_type = 'isolated_worktree_created'
                               OR (owner.parent_id IS NULL AND owner.event_type = 'repository_root_observed'))
                           ORDER BY owner.seq LIMIT 1))
                 FROM node_events WHERE tree_id > ?1 AND parent_id IS NULL AND event_type = 'started'
                   AND seq = (SELECT MIN(root.seq) FROM node_events root WHERE root.tree_id = node_events.tree_id)
                   AND (?2 OR COALESCE((SELECT event_type FROM node_events archive
                     WHERE archive.tree_id = node_events.tree_id AND archive.parent_id IS NULL
                       AND archive.event_type IN ('archive_requested', 'restore_requested')
                     ORDER BY seq DESC LIMIT 1), '') != 'archive_requested')
                 ORDER BY tree_id LIMIT 128"
            ).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))?;
            statement.query_map(rusqlite::params![after, include_archived], |row| Ok(ExecutionTreeArchiveCandidate {
                execution_id: row.get(0)?, worktree_path: row.get(1)?, workspace_identity: row.get(2)?, repository_root: row.get(3)?,
            })).and_then(|rows| rows.collect()).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
        }).await.map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))
    }

    async fn append(
        &self,
        execution_id: &str,
        fact: NodeFact,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        let FactLogReadBackend::Live(store) = &self.backend else {
            return Err(WorkflowError::external("archive repository is read only"));
        };
        let tree_id = execution_id.to_string();
        let row = self
            .backend
            .run_indexed(move |connection| {
                crate::adaptor::gateway::local_event_store::node_events::first_row_of_tree(
                    connection, &tree_id,
                )
                .map_err(|error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                })
            })
            .await
            .map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.to_string()))?;
        let root = fact_log::record_from_row(&row)
            .map_err(WorkflowError::external)?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.to_string()))?;
        fact_log::append_single_fact(store, &root.meta, &fact, (timestamp * 1000.0) as i64).await
    }
}

#[async_trait::async_trait]
impl ExecutionTreeArchiveRepository for ExecutionTreeArchiveFactRepository {
    async fn location(
        &self,
        execution_id: &str,
    ) -> Result<ExecutionTreeArchiveCandidate, WorkflowError> {
        let id = execution_id.to_string();
        let row = self
            .backend
            .run_indexed(move |connection| {
                crate::adaptor::gateway::local_event_store::node_events::first_row_of_tree(
                    connection, &id,
                )
                .map_err(|error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                })
            })
            .await
            .map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.into()))?;
        let mut root = super::stored_definition::read_tree_header(&row.detail)
            .map_err(WorkflowError::CorruptStoredState)?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.into()))?;
        if root.repository_root.is_none() {
            let id = execution_id.to_string();
            root.repository_root = self.backend.run_indexed(move |connection| {
                connection.query_row(
                    "SELECT (SELECT json_extract(detail, '$.repositoryRoot') FROM node_events
                     WHERE tree_id = ?1 AND parent_id IS NULL AND event_type = 'repository_root_observed'
                     ORDER BY seq LIMIT 1)",
                    [id], |row| row.get(0),
                ).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
            }).await.map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))?;
        }
        Ok(ExecutionTreeArchiveCandidate {
            execution_id: row.tree_id,
            worktree_path: root.worktree_path,
            workspace_identity: root.workspace_identity,
            repository_root: root.repository_root,
        })
    }

    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError> {
        Ok(archive_path_key(path)?.to_string_lossy().into_owned())
    }

    async fn worktree_target_page(
        &self,
        worktree_path: &str,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError> {
        let worktree_path = archive_path_key(worktree_path)?;
        let mut after = after.map(str::to_string);
        loop {
            let page = self.read_candidate_page(after.as_deref(), true).await?;
            let Some(last) = page.last() else {
                return Ok(Vec::new());
            };
            after = Some(last.execution_id.clone());
            let targets: Vec<_> = page
                .into_iter()
                .filter(|candidate| {
                    [&candidate.worktree_path, &candidate.workspace_identity]
                        .into_iter()
                        .any(|path| archive_path_key(path).is_ok_and(|path| path == worktree_path))
                })
                .collect();
            if !targets.is_empty() {
                return Ok(targets);
            }
        }
    }

    async fn candidate_page(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError> {
        let mut page = self.read_candidate_page(after, false).await?;
        for candidate in &mut page {
            if candidate.repository_root.is_none() {
                candidate.repository_root =
                    [&candidate.workspace_identity, &candidate.worktree_path]
                        .into_iter()
                        .find_map(|path| {
                            super::super::repository::worktree::recorded_main_repo_path(path)
                        });
            }
        }
        Ok(page)
    }

    async fn record_repository_root(
        &self,
        execution_id: &str,
        repository_root: &str,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        match self.location(execution_id).await?.repository_root {
            Some(root) if root != repository_root => Err(WorkflowError::CorruptStoredState(
                format!("tree {execution_id} has conflicting repository roots"),
            )),
            Some(_) => Ok(()),
            None => {
                self.append(
                    execution_id,
                    NodeFact::RepositoryRootObserved(repository_root.into()),
                    timestamp,
                )
                .await
            }
        }
    }

    async fn legacy_session_archive_page(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveRecord>, WorkflowError> {
        let after = after.unwrap_or("").to_string();
        let ids = self.backend.run_indexed(move |connection| {
            let mut statement = connection.prepare(
                "SELECT tree_id FROM node_events archive
                 WHERE tree_id > ?1 AND parent_id IS NULL AND event_type = 'archive_requested'
                   AND json_extract(detail, '$.archivedAt') IS NULL
                   AND seq = (SELECT MAX(seq) FROM node_events latest WHERE latest.tree_id = archive.tree_id
                       AND latest.parent_id IS NULL AND latest.event_type IN ('archive_requested', 'restore_requested'))
                 ORDER BY tree_id LIMIT 128"
            ).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))?;
            statement.query_map([after], |row| row.get::<_, String>(0)).and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                .map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
        }).await.map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))?;
        Ok(self.archive_snapshot_for(&ids).await?.records)
    }

    async fn target(
        &self,
        execution_id: &str,
    ) -> Result<ExecutionTreeArchiveTarget, WorkflowError> {
        let folded = fact_log::fold_tree_from(&self.backend, execution_id)
            .await
            .map_err(WorkflowError::from)?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.to_string()))?;
        let status =
            crate::domain::workflow::services::fact_replay::derive_read_model(&folded).status;
        Ok(ExecutionTreeArchiveTarget {
            execution_id: execution_id.to_string(),
            worktree_path: folded.root.worktree_path,
            workspace_identity: folded.root.workspace_identity,
            repository_root: folded.root.repository_root,
            status,
        })
    }

    async fn archive(
        &self,
        execution_id: &ExecutionTreeId,
        archived_at: f64,
        reason: &str,
    ) -> Result<(), WorkflowError> {
        let mut tree = fact_log::fold_tree_from(&self.backend, execution_id.as_str())
            .await
            .map_err(WorkflowError::from)?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.to_string()))?;
        if let Some(fact) = tree.aggregate.archive(archived_at, reason)? {
            return self.append(execution_id.as_str(), fact, archived_at).await;
        }
        let id = execution_id.to_string();
        let legacy = self.backend.run_indexed(move |connection| {
            connection.query_row(
                "SELECT detail, timestamp FROM node_events WHERE tree_id = ?1 AND parent_id IS NULL
                 AND event_type IN ('archive_requested', 'restore_requested') ORDER BY seq DESC LIMIT 1",
                [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            ).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
        }).await.map_err(|error| WorkflowError::from(fact_log::FactReadError::Query(error)))?;
        let detail: serde_json::Value = serde_json::from_str(&legacy.0)
            .map_err(|error| WorkflowError::CorruptStoredState(error.to_string()))?;
        if detail
            .get("archivedAt")
            .is_none_or(serde_json::Value::is_null)
        {
            let fact = fact_log::decode_stored_fact("archive_requested", &legacy.0, legacy.1)
                .map_err(WorkflowError::CorruptStoredState)?
                .ok_or_else(|| WorkflowError::CorruptStoredState("archive fact missing".into()))?;
            return self
                .append(execution_id.as_str(), fact, legacy.1 as f64 / 1000.0)
                .await;
        }
        Ok(())
    }

    async fn restore(
        &self,
        execution_id: &ExecutionTreeId,
        restored_at: f64,
    ) -> Result<(), WorkflowError> {
        let mut tree = fact_log::fold_tree_from(&self.backend, execution_id.as_str())
            .await
            .map_err(WorkflowError::from)?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.to_string()))?;
        if let Some(fact) = tree.aggregate.restore_archive() {
            self.append(execution_id.as_str(), fact, restored_at)
                .await?;
        }
        Ok(())
    }

    async fn archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<ExecutionTreeArchiveSnapshot, WorkflowError> {
        let facts = fact_log::read_tree_archive_records_for(&self.backend, execution_ids)
            .await
            .map_err(WorkflowError::from)?;
        let mut records = facts
            .iter()
            .filter_map(|fact| {
                crate::domain::workflow::services::fact_replay::derive_tree_archive(
                    std::slice::from_ref(fact),
                )
            })
            .collect::<Vec<_>>();
        records.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));
        records.dedup_by(|a, b| a.execution_id == b.execution_id);
        Ok(ExecutionTreeArchiveSnapshot { records })
    }

    fn legacy_archives(&self) -> Result<Vec<ExecutionTreeArchiveRecord>, WorkflowError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LegacyRecord {
            archived_at: Option<f64>,
            archive_reason: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct LegacyIndex {
            #[serde(default)]
            executions: BTreeMap<String, LegacyRecord>,
        }
        let Some(path) = &self.legacy_path else {
            return Ok(Vec::new());
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(WorkflowError::external(error.to_string())),
        };
        let index: LegacyIndex = serde_json::from_slice(&bytes)
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        Ok(index
            .executions
            .into_iter()
            .filter_map(|(execution_id, record)| {
                record
                    .archived_at
                    .map(|archived_at| ExecutionTreeArchiveRecord {
                        execution_id,
                        archived_at,
                        archive_reason: record
                            .archive_reason
                            .unwrap_or_else(|| "manual".to_string()),
                    })
            })
            .collect())
    }

    fn finish_legacy_migration(&self) -> Result<(), WorkflowError> {
        let Some(path) = &self.legacy_path else {
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(WorkflowError::external(error.to_string())),
        }
    }
}

fn archive_path_key(path: &str) -> Result<PathBuf, WorkflowError> {
    crate::adaptor::gateway::repository::worktree_operation::worktree_identity(path).map_err(
        |error| {
            WorkflowError::external(format!(
                "archive worktree path {path} could not be resolved: {error}"
            ))
        },
    )
}

#[cfg(test)]
#[path = "execution_archive_repository_test.rs"]
mod execution_archive_repository_tests;
