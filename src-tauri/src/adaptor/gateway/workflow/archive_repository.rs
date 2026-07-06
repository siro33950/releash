use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::domain::workflow::{
    RunId, WorkflowError, WorkflowRunArchiveRepository, WorkflowRunManualArchiveRecord,
    WORKFLOW_ARCHIVE_REASON_MANUAL,
};

const WORKFLOW_ARCHIVES_FILE: &str = "workflow_run_archives.json";

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunArchiveRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restored_at: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunArchiveIndex {
    #[serde(default)]
    runs: BTreeMap<String, WorkflowRunArchiveRecord>,
}

#[derive(Debug)]
pub(crate) struct WorkflowRunArchiveFileRepository {
    data_dir: PathBuf,
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WorkflowRunArchivePruneResult {
    pub(crate) records_removed: u64,
    pub(crate) reclaimed_bytes: u64,
}

impl WorkflowRunArchiveFileRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            lock: Mutex::new(()),
        }
    }

    fn archive_index_path(&self) -> PathBuf {
        self.data_dir.join(WORKFLOW_ARCHIVES_FILE)
    }

    fn load_index_unlocked(&self) -> Result<WorkflowRunArchiveIndex, WorkflowError> {
        let path = self.archive_index_path();
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).map_err(|e| {
                WorkflowError::external(format!("Failed to parse workflow run archives: {e}"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(WorkflowRunArchiveIndex::default())
            }
            Err(e) => Err(WorkflowError::external(format!(
                "Failed to read workflow run archives: {e}"
            ))),
        }
    }

    fn save_index_unlocked(&self, index: &WorkflowRunArchiveIndex) -> Result<(), WorkflowError> {
        fs::create_dir_all(&self.data_dir).map_err(|e| {
            WorkflowError::external(format!("Failed to create app data directory: {e}"))
        })?;
        let json = serde_json::to_string_pretty(index).map_err(|e| {
            WorkflowError::external(format!("Failed to serialize workflow run archives: {e}"))
        })?;
        atomic_write(&self.archive_index_path(), &json).map_err(|e| {
            WorkflowError::external(format!("Failed to write workflow run archives: {e}"))
        })
    }

    fn update_index(
        &self,
        update: impl FnOnce(&mut WorkflowRunArchiveIndex),
    ) -> Result<(), WorkflowError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WorkflowError::external("workflow run archive lock poisoned"))?;
        let mut index = self.load_index_unlocked()?;
        update(&mut index);
        self.save_index_unlocked(&index)
    }

    pub(crate) fn prune_records(
        &self,
        run_ids: &HashSet<String>,
    ) -> Result<WorkflowRunArchivePruneResult, WorkflowError> {
        if run_ids.is_empty() {
            return Ok(WorkflowRunArchivePruneResult::default());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WorkflowError::external("workflow run archive lock poisoned"))?;
        let path = self.archive_index_path();
        let before = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
            Ok(_) => 0,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(WorkflowError::external(format!(
                    "Failed to read workflow run archives metadata: {error}"
                )));
            }
        };
        let mut index = self.load_index_unlocked()?;
        let mut removed = 0;
        for run_id in run_ids {
            if index.runs.remove(run_id).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(WorkflowRunArchivePruneResult::default());
        }
        self.save_index_unlocked(&index)?;
        let after = fs::symlink_metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(WorkflowRunArchivePruneResult {
            records_removed: removed,
            reclaimed_bytes: before.saturating_sub(after),
        })
    }
}

impl WorkflowRunArchiveRepository for WorkflowRunArchiveFileRepository {
    fn archive_manual(&self, run_id: &RunId, archived_at: f64) -> Result<(), WorkflowError> {
        self.update_index(|index| {
            let record = index.runs.entry(run_id.to_string()).or_default();
            record.archived_at = Some(archived_at);
            record.archive_reason = Some(WORKFLOW_ARCHIVE_REASON_MANUAL.to_string());
        })
    }

    fn restore_manual(&self, run_id: &RunId, restored_at: f64) -> Result<(), WorkflowError> {
        self.update_index(|index| {
            let record = index.runs.entry(run_id.to_string()).or_default();
            record.archived_at = None;
            record.archive_reason = None;
            record.restored_at = Some(restored_at);
        })
    }

    fn manual_archive_records(&self) -> Result<Vec<WorkflowRunManualArchiveRecord>, WorkflowError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WorkflowError::external("workflow run archive lock poisoned"))?;
        let index = self.load_index_unlocked()?;
        let mut records = index
            .runs
            .into_iter()
            .filter_map(|(run_id, record)| {
                is_manual_archive_record(&record).then(|| WorkflowRunManualArchiveRecord {
                    run_id,
                    archived_at: record.archived_at.unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        records.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        Ok(records)
    }
}

fn is_manual_archive_record(record: &WorkflowRunArchiveRecord) -> bool {
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
        let repo = WorkflowRunArchiveFileRepository::new(temp.path());

        let index = repo.load_index_unlocked().unwrap();

        assert!(index.runs.is_empty());
        assert!(!repo.archive_index_path().exists());
    }

    #[test]
    fn manual_archive_and_restore_persist_records() {
        let temp = tempfile::tempdir().unwrap();
        let repo = WorkflowRunArchiveFileRepository::new(temp.path());
        let run_id = RunId::new("11111111-1111-4111-8111-111111111111".to_string()).unwrap();

        repo.archive_manual(&run_id, 10.0).unwrap();
        repo.restore_manual(&run_id, 20.0).unwrap();

        let index = repo.load_index_unlocked().unwrap();
        let record = &index.runs["11111111-1111-4111-8111-111111111111"];
        assert_eq!(record.archived_at, None);
        assert_eq!(record.archive_reason, None);
        assert_eq!(record.restored_at, Some(20.0));
        assert!(repo.manual_archive_records().unwrap().is_empty());
    }

    #[test]
    fn lock_preserves_concurrent_manual_archive_updates() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Arc::new(WorkflowRunArchiveFileRepository::new(temp.path()));
        let (loaded_tx, loaded_rx) = mpsc::channel();

        let slow_repo = repo.clone();
        let slow_archive = std::thread::spawn(move || {
            slow_repo
                .update_index(|index| {
                    loaded_tx.send(()).expect("archive load signal");
                    std::thread::sleep(Duration::from_millis(50));
                    let record = index.runs.entry("slow-run".to_string()).or_default();
                    record.archived_at = Some(20.0);
                    record.archive_reason = Some(WORKFLOW_ARCHIVE_REASON_MANUAL.to_string());
                })
                .unwrap();
        });

        loaded_rx.recv().expect("slow archive loaded index");
        let manual_run_id = RunId::new("22222222-2222-4222-8222-222222222222".to_string()).unwrap();
        repo.archive_manual(&manual_run_id, 30.0).unwrap();

        slow_archive.join().unwrap();

        let index = repo.load_index_unlocked().unwrap();
        assert_eq!(
            index.runs["slow-run"].archive_reason.as_deref(),
            Some(WORKFLOW_ARCHIVE_REASON_MANUAL)
        );
        assert_eq!(
            index.runs["22222222-2222-4222-8222-222222222222"]
                .archive_reason
                .as_deref(),
            Some(WORKFLOW_ARCHIVE_REASON_MANUAL)
        );
        let records = repo.manual_archive_records().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["22222222-2222-4222-8222-222222222222", "slow-run"]
        );
    }

    #[test]
    fn prune_records_removes_selected_runs_with_atomic_index_update() {
        let temp = tempfile::tempdir().unwrap();
        let repo = WorkflowRunArchiveFileRepository::new(temp.path());
        let keep_run_id = RunId::new("11111111-1111-4111-8111-111111111111".to_string()).unwrap();
        let prune_run_id = RunId::new("22222222-2222-4222-8222-222222222222".to_string()).unwrap();
        repo.archive_manual(&keep_run_id, 10.0).unwrap();
        repo.archive_manual(&prune_run_id, 20.0).unwrap();

        let result = repo
            .prune_records(&HashSet::from([prune_run_id.to_string()]))
            .unwrap();

        assert_eq!(result.records_removed, 1);
        assert!(result.reclaimed_bytes > 0);
        let index = repo.load_index_unlocked().unwrap();
        assert!(index.runs.contains_key(keep_run_id.as_str()));
        assert!(!index.runs.contains_key(prune_run_id.as_str()));
    }
}
