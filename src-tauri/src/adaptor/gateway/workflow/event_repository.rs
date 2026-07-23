use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::workflow::{WorkflowError, WorkflowExecutionId, WorkflowPageRequest};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};

use super::mapper;

#[derive(Clone)]
pub(crate) struct WorkflowEventLogRepository {
    source: WorkflowEventReadSource,
}

#[derive(Clone)]
enum WorkflowEventReadSource {
    #[cfg(test)]
    Legacy(PathBuf),
    Canonical {
        data_dir: PathBuf,
        repository: Arc<dyn LocalEventTransactionRepository>,
        generation_id: String,
    },
    PhaseAware {
        data_dir: PathBuf,
        repository: Arc<dyn LocalEventTransactionRepository>,
        generation_id: String,
        canonical_reads_active: Arc<dyn Fn() -> bool + Send + Sync>,
    },
}

impl WorkflowEventLogRepository {
    #[cfg(test)]
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            source: WorkflowEventReadSource::Legacy(data_dir.into()),
        }
    }

    pub(crate) fn with_authority(
        data_dir: impl Into<PathBuf>,
        repository: Arc<dyn LocalEventTransactionRepository>,
        generation_id: String,
    ) -> Self {
        Self {
            source: WorkflowEventReadSource::Canonical {
                data_dir: data_dir.into(),
                repository,
                generation_id,
            },
        }
    }

    pub(crate) fn with_phase_aware_authority(
        data_dir: impl Into<PathBuf>,
        repository: Arc<dyn LocalEventTransactionRepository>,
        generation_id: String,
        canonical_reads_active: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            source: WorkflowEventReadSource::PhaseAware {
                data_dir: data_dir.into(),
                repository,
                generation_id,
                canonical_reads_active,
            },
        }
    }

    fn log(&self) -> WorkflowEventLog {
        match &self.source {
            #[cfg(test)]
            WorkflowEventReadSource::Legacy(data_dir) => WorkflowEventLog::new(data_dir),
            WorkflowEventReadSource::Canonical {
                data_dir,
                repository,
                generation_id,
            } => WorkflowEventLog::with_authority(
                data_dir.as_path(),
                repository.clone(),
                generation_id.clone(),
            ),
            WorkflowEventReadSource::PhaseAware {
                data_dir,
                repository,
                generation_id,
                canonical_reads_active,
            } => {
                if canonical_reads_active() {
                    WorkflowEventLog::with_authority(
                        data_dir.as_path(),
                        repository.clone(),
                        generation_id.clone(),
                    )
                } else {
                    WorkflowEventLog::new(data_dir)
                }
            }
        }
    }

    fn read_events(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<crate::adaptor::gateway::workflow::event::WorkflowEvent>, String> {
        match &self.source {
            WorkflowEventReadSource::PhaseAware {
                canonical_reads_active,
                ..
            } if !canonical_reads_active() => self.log().read_log(execution_id.as_str()),
            #[cfg(test)]
            WorkflowEventReadSource::Legacy(_) => self.log().read_log(execution_id.as_str()),
            _ => self.log().read_log_durable_blocking(execution_id.as_str()),
        }
    }
}

impl WorkflowEventRepository for WorkflowEventLogRepository {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        self.append_batch(std::slice::from_ref(event))
    }

    #[cfg(test)]
    fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError> {
        let workflow_events: Vec<_> = events
            .iter()
            .map(mapper::event_draft_to_event)
            .collect::<Result<_, _>>()?;
        self.log()
            .append_batch(&workflow_events)
            .map_err(WorkflowError::external)
    }

    fn read(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_events(execution_id)
            .map_err(WorkflowError::external)?
            .iter()
            .map(mapper::workflow_event_to_domain_draft)
            .collect()
    }

    fn read_page(
        &self,
        execution_id: &WorkflowExecutionId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_events(execution_id)
            .map_err(WorkflowError::external)?
            .into_iter()
            .skip(page.offset)
            .take(page.limit)
            .map(|event| mapper::workflow_event_to_domain_draft(&event))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use tempfile::TempDir;

    fn started_event(
        execution_id: &WorkflowExecutionId,
        workflow_name: &str,
        timestamp: f64,
    ) -> WorkflowEventDraft {
        WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp,
            payload: serde_json::json!({
                "workflow_name": workflow_name,
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "",
                "permission_mode": "ask",
                "definition": {
                    "name": workflow_name,
                    "description": "",
                    "nodes": []
                }
            }),
        }
    }

    async fn open_ready_store(data_dir: &std::path::Path) -> Arc<LocalEventStore> {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !store.normal_admission_ready() && !store.cutover_ready() {
                assert!(!store.migration_blocked());
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fresh SQLite authority must become ready");
        if store.cutover_ready() {
            assert!(store.open_normal_admission_after_authority_install());
        }
        store
    }

    #[test]
    fn append_preserves_canonical_execution_event_tag() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").unwrap();

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "ship it",
                "permission_mode": "ask",
                "definition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "node",
                        "session": { "gate": "auto" }
                    }]
                }
            }),
        })
        .unwrap();

        let content = std::fs::read_to_string(
            tmp.path()
                .join("workflow_execution_logs")
                .join(format!("{execution_id}.ndjson")),
        )
        .unwrap();
        assert!(content.contains("\"event\":\"execution_started\""));

        let events = repo.read(&execution_id).unwrap();
        assert_eq!(events[0].event_kind, "execution_started");
        assert_eq!(events[0].payload["workflow_name"], "wf");
        assert_eq!(events[0].payload["request"], "ship it");
    }

    #[test]
    fn read_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000002").unwrap();

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "",
                "permission_mode": "ask",
                "definition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "node",
                        "session": { "gate": "auto" }
                    }]
                }
            }),
        })
        .unwrap();

        let first = repo.read(&execution_id).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event_kind, "execution_started");
        assert_eq!(first[0].payload["workflow_name"], "wf");

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_aborted".to_string(),
            timestamp: 2.0,
            payload: serde_json::json!({"aborted_node": null}),
        })
        .unwrap();

        let second = repo.read(&execution_id).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0], first[0]);
        assert_eq!(second[1].event_kind, "execution_aborted");
        assert_eq!(second[1].timestamp, 2.0);
        assert_eq!(second[1].payload["aborted_node"], serde_json::Value::Null);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn phase_switch_reads_sqlite_without_touching_or_falling_back_to_legacy() {
        let sqlite = TempDir::new().unwrap();
        let legacy = TempDir::new().unwrap();
        let store = open_ready_store(sqlite.path()).await;
        let authority: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let generation_id = store.generation_id().to_string();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000003").unwrap();

        let canonical_draft = started_event(&execution_id, "canonical", 2.0);
        WorkflowEventLog::with_authority(legacy.path(), authority.clone(), generation_id.clone())
            .append_batch_durable(&[mapper::event_draft_to_event(&canonical_draft).unwrap()])
            .await
            .unwrap();

        WorkflowEventLogRepository::new(legacy.path())
            .append(&started_event(&execution_id, "legacy", 1.0))
            .unwrap();

        let canonical_reads_active = Arc::new(AtomicBool::new(false));
        let phase_flag = canonical_reads_active.clone();
        let phase_aware = WorkflowEventLogRepository::with_phase_aware_authority(
            legacy.path(),
            authority.clone(),
            generation_id.clone(),
            Arc::new(move || phase_flag.load(Ordering::Acquire)),
        );
        assert_eq!(
            phase_aware.read(&execution_id).unwrap()[0].payload["workflow_name"],
            "legacy"
        );

        canonical_reads_active.store(true, Ordering::Release);
        let legacy_path = legacy
            .path()
            .join("workflow_execution_logs")
            .join(format!("{execution_id}.ndjson"));
        std::fs::write(&legacy_path, b"not valid workflow json\n").unwrap();

        let events = phase_aware.read(&execution_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["workflow_name"], "canonical");

        let canonical =
            WorkflowEventLogRepository::with_authority(legacy.path(), authority, generation_id);
        assert_eq!(
            canonical.read(&execution_id).unwrap()[0].payload["workflow_name"],
            "canonical"
        );
        assert_eq!(
            std::fs::read(&legacy_path).unwrap(),
            b"not valid workflow json\n"
        );
    }
}
