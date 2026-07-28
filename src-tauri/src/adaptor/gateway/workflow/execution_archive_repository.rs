use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::domain::workflow::{
    WorkflowError, WorkflowExecutionArchiveRepository, WorkflowExecutionArchiveSnapshot,
    WorkflowExecutionId, WorkflowExecutionManualArchiveRecord, WORKFLOW_ARCHIVE_REASON_MANUAL,
};

const WORKFLOW_EXECUTION_ARCHIVES_FILE: &str = "workflow_execution_archives.json";

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowExecutionArchiveRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restored_at: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowExecutionArchiveIndex {
    #[serde(default)]
    executions: BTreeMap<String, WorkflowExecutionArchiveRecord>,
}

#[derive(Debug)]
pub(crate) struct WorkflowExecutionArchiveFileRepository {
    data_dir: PathBuf,
    state: Mutex<Result<WorkflowExecutionArchiveState, String>>,
}

#[derive(Debug, Clone)]
struct WorkflowExecutionArchiveState {
    index: WorkflowExecutionArchiveIndex,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct WorkflowExecutionArchivePruneResult {
    pub(crate) records_removed: u64,
    pub(crate) reclaimed_bytes: u64,
}

impl WorkflowExecutionArchiveFileRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let repository = Self {
            data_dir,
            state: Mutex::new(Ok(WorkflowExecutionArchiveState {
                index: WorkflowExecutionArchiveIndex::default(),
            })),
        };
        let initial = repository
            .load_index_unlocked()
            .map(|index| WorkflowExecutionArchiveState { index })
            .map_err(|error| error.to_string());
        *repository.state.lock().expect("archive state poisoned") = initial;
        repository
    }

    fn archive_index_path(&self) -> PathBuf {
        self.data_dir.join(WORKFLOW_EXECUTION_ARCHIVES_FILE)
    }

    fn load_index_unlocked(&self) -> Result<WorkflowExecutionArchiveIndex, WorkflowError> {
        let path = self.archive_index_path();
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).map_err(|e| {
                WorkflowError::external(format!("Failed to parse workflow execution archives: {e}"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(WorkflowExecutionArchiveIndex::default())
            }
            Err(e) => Err(WorkflowError::external(format!(
                "Failed to read workflow execution archives: {e}"
            ))),
        }
    }

    fn save_index_unlocked(
        &self,
        index: &WorkflowExecutionArchiveIndex,
    ) -> Result<(), WorkflowError> {
        fs::create_dir_all(&self.data_dir).map_err(|e| {
            WorkflowError::external(format!("Failed to create app data directory: {e}"))
        })?;
        let json = serde_json::to_string_pretty(index).map_err(|e| {
            WorkflowError::external(format!(
                "Failed to serialize workflow execution archives: {e}"
            ))
        })?;
        atomic_write(&self.archive_index_path(), &json).map_err(|e| {
            WorkflowError::external(format!("Failed to write workflow execution archives: {e}"))
        })
    }

    fn update_index(
        &self,
        update: impl FnOnce(&mut WorkflowExecutionArchiveIndex),
    ) -> Result<(), WorkflowError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkflowError::external("workflow execution archive lock poisoned"))?;
        let current = state
            .as_ref()
            .map_err(|error| WorkflowError::external(error.clone()))?;
        let mut index = current.index.clone();
        update(&mut index);
        self.save_index_unlocked(&index)?;
        *state = Ok(WorkflowExecutionArchiveState { index });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn prune_records(
        &self,
        execution_ids: &HashSet<String>,
    ) -> Result<WorkflowExecutionArchivePruneResult, WorkflowError> {
        if execution_ids.is_empty() {
            return Ok(WorkflowExecutionArchivePruneResult::default());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkflowError::external("workflow execution archive lock poisoned"))?;
        let path = self.archive_index_path();
        let before = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
            Ok(_) => 0,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(WorkflowError::external(format!(
                    "Failed to read workflow execution archives metadata: {error}"
                )));
            }
        };
        let mut index = state
            .as_ref()
            .map_err(|error| WorkflowError::external(error.clone()))?
            .index
            .clone();
        let mut removed = 0;
        for execution_id in execution_ids {
            if index.executions.remove(execution_id).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(WorkflowExecutionArchivePruneResult::default());
        }
        self.save_index_unlocked(&index)?;
        *state = Ok(WorkflowExecutionArchiveState { index });
        let after = fs::symlink_metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(WorkflowExecutionArchivePruneResult {
            records_removed: removed,
            reclaimed_bytes: before.saturating_sub(after),
        })
    }
}

impl WorkflowExecutionArchiveRepository for WorkflowExecutionArchiveFileRepository {
    fn archive_manual(
        &self,
        execution_id: &WorkflowExecutionId,
        archived_at: f64,
    ) -> Result<(), WorkflowError> {
        self.update_index(|index| {
            let record = index
                .executions
                .entry(execution_id.to_string())
                .or_default();
            record.archived_at = Some(archived_at);
            record.archive_reason = Some(WORKFLOW_ARCHIVE_REASON_MANUAL.to_string());
        })
    }

    fn restore_manual(
        &self,
        execution_id: &WorkflowExecutionId,
        restored_at: f64,
    ) -> Result<(), WorkflowError> {
        self.update_index(|index| {
            let record = index
                .executions
                .entry(execution_id.to_string())
                .or_default();
            record.archived_at = None;
            record.archive_reason = None;
            record.restored_at = Some(restored_at);
        })
    }

    fn manual_archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<WorkflowExecutionArchiveSnapshot, WorkflowError> {
        let state = self
            .state
            .lock()
            .map_err(|_| WorkflowError::external("workflow execution archive lock poisoned"))?;
        let state = state
            .as_ref()
            .map_err(|error| WorkflowError::external(error.clone()))?;
        let mut records = execution_ids
            .iter()
            .filter_map(|execution_id| {
                state
                    .index
                    .executions
                    .get(execution_id)
                    .filter(|record| is_manual_archive_record(record))
                    .map(|record| WorkflowExecutionManualArchiveRecord {
                        execution_id: execution_id.clone(),
                        archived_at: record
                            .archived_at
                            .expect("manual archive predicate requires archived_at"),
                    })
            })
            .collect::<Vec<_>>();
        records.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));
        records.dedup_by(|left, right| left.execution_id == right.execution_id);
        Ok(WorkflowExecutionArchiveSnapshot { records })
    }
}

fn is_manual_archive_record(record: &WorkflowExecutionArchiveRecord) -> bool {
    record.archived_at.is_some()
        && record.archive_reason.as_deref() == Some(WORKFLOW_ARCHIVE_REASON_MANUAL)
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
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn missing_archive_index_loads_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let repo = WorkflowExecutionArchiveFileRepository::new(temp.path());

        let index = repo.load_index_unlocked().unwrap();

        assert!(index.executions.is_empty());
        assert!(!repo.archive_index_path().exists());
    }

    #[test]
    fn manual_archive_and_restore_persist_records() {
        let temp = tempfile::tempdir().unwrap();
        let repo = WorkflowExecutionArchiveFileRepository::new(temp.path());
        let execution_id =
            WorkflowExecutionId::new("11111111-1111-4111-8111-111111111111".to_string()).unwrap();

        repo.archive_manual(&execution_id, 10.0).unwrap();
        repo.restore_manual(&execution_id, 20.0).unwrap();

        let index = repo.load_index_unlocked().unwrap();
        let record = &index.executions["11111111-1111-4111-8111-111111111111"];
        assert_eq!(record.archived_at, None);
        assert_eq!(record.archive_reason, None);
        assert_eq!(record.restored_at, Some(20.0));
        assert!(repo
            .manual_archive_snapshot_for(&[execution_id.to_string()])
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn b004_archive_queries_use_the_process_local_index_without_file_rereads() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(WORKFLOW_EXECUTION_ARCHIVES_FILE),
            r#"{"executions":{"11111111-1111-4111-8111-111111111111":{"archivedAt":10.0,"archiveReason":"manual"}}}"#,
        )
        .unwrap();
        let repo = WorkflowExecutionArchiveFileRepository::new(temp.path());
        let first = repo
            .manual_archive_snapshot_for(&["11111111-1111-4111-8111-111111111111".to_string()])
            .unwrap();

        std::fs::write(
            temp.path().join(WORKFLOW_EXECUTION_ARCHIVES_FILE),
            b"not valid JSON",
        )
        .unwrap();
        for _ in 0..20 {
            assert_eq!(
                repo.manual_archive_snapshot_for(&[
                    "11111111-1111-4111-8111-111111111111".to_string()
                ])
                .unwrap(),
                first
            );
        }
    }

    #[test]
    fn lock_preserves_concurrent_manual_archive_updates() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Arc::new(WorkflowExecutionArchiveFileRepository::new(temp.path()));
        let (loaded_tx, loaded_rx) = mpsc::channel();

        let slow_repo = repo.clone();
        let slow_archive = std::thread::spawn(move || {
            slow_repo
                .update_index(|index| {
                    loaded_tx.send(()).expect("archive load signal");
                    std::thread::sleep(Duration::from_millis(50));
                    let record = index
                        .executions
                        .entry("slow-execution".to_string())
                        .or_default();
                    record.archived_at = Some(20.0);
                    record.archive_reason = Some(WORKFLOW_ARCHIVE_REASON_MANUAL.to_string());
                })
                .unwrap();
        });

        loaded_rx.recv().expect("slow archive loaded index");
        let manual_execution_id =
            WorkflowExecutionId::new("22222222-2222-4222-8222-222222222222".to_string()).unwrap();
        repo.archive_manual(&manual_execution_id, 30.0).unwrap();

        slow_archive.join().unwrap();

        let index = repo.load_index_unlocked().unwrap();
        assert_eq!(
            index.executions["slow-execution"].archive_reason.as_deref(),
            Some(WORKFLOW_ARCHIVE_REASON_MANUAL)
        );
        assert_eq!(
            index.executions["22222222-2222-4222-8222-222222222222"]
                .archive_reason
                .as_deref(),
            Some(WORKFLOW_ARCHIVE_REASON_MANUAL)
        );
        let records = repo
            .manual_archive_snapshot_for(&[
                "22222222-2222-4222-8222-222222222222".to_string(),
                "slow-execution".to_string(),
            ])
            .unwrap()
            .records;
        assert_eq!(
            records
                .iter()
                .map(|record| record.execution_id.as_str())
                .collect::<Vec<_>>(),
            vec!["22222222-2222-4222-8222-222222222222", "slow-execution"]
        );
    }

    #[test]
    fn prune_records_removes_selected_executions_with_atomic_index_update() {
        let temp = tempfile::tempdir().unwrap();
        let repo = WorkflowExecutionArchiveFileRepository::new(temp.path());
        let keep_execution_id =
            WorkflowExecutionId::new("11111111-1111-4111-8111-111111111111".to_string()).unwrap();
        let prune_execution_id =
            WorkflowExecutionId::new("22222222-2222-4222-8222-222222222222".to_string()).unwrap();
        repo.archive_manual(&keep_execution_id, 10.0).unwrap();
        repo.archive_manual(&prune_execution_id, 20.0).unwrap();

        let result = repo
            .prune_records(&HashSet::from([prune_execution_id.to_string()]))
            .unwrap();

        assert_eq!(result.records_removed, 1);
        assert!(result.reclaimed_bytes > 0);
        let index = repo.load_index_unlocked().unwrap();
        assert!(index.executions.contains_key(keep_execution_id.as_str()));
        assert!(!index.executions.contains_key(prune_execution_id.as_str()));
    }
}
