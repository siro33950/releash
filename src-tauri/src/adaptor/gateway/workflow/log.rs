#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::fs::OpenOptions;
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::io::{BufRead, BufReader};
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::LazyLock;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

#[cfg(test)]
use crate::adaptor::gateway::workflow::event::{
    decode_stored_workflow_event_v1, DecodedStoredWorkflowEventV1, IncompatibleStoredWorkflowEvent,
    StoredWorkflowPayloadSource,
};
use crate::adaptor::gateway::workflow::event::{
    encode_stored_workflow_event_v1, to_domain_event, WorkflowEvent,
};

/// NDJSON persistence adapter for workflow execution events.
///
/// The adapter deliberately performs on-demand reads. Projection and lifecycle
/// decisions belong to the workflow query/use-case layer, not to persistence.
pub struct WorkflowEventLog {
    #[cfg(test)]
    log_dir: PathBuf,
    authority: Option<WorkflowEventAuthority>,
}

#[derive(Clone)]
struct WorkflowEventAuthority {
    repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    installation_id: String,
}

#[cfg(test)]
#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkflowEventLogReadError {
    #[error("failed to read workflow execution log: {0}")]
    Io(String),
    #[error("workflow execution log line {line}: {source}")]
    Incompatible {
        line: usize,
        #[source]
        source: IncompatibleStoredWorkflowEvent,
    },
    #[error(
        "workflow execution log line {line} contains execution_id {actual} instead of {expected}"
    )]
    ExecutionIdentity {
        line: usize,
        actual: String,
        expected: String,
    },
}

#[cfg(test)]
static LOG_FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
fn log_file_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut locks = LOG_FILE_LOCKS.lock();
    Arc::clone(
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

impl WorkflowEventLog {
    #[cfg(test)]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            log_dir: data_dir.join("workflow_execution_logs"),
            authority: None,
        }
    }

    pub(crate) fn with_authority(
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            #[cfg(test)]
            log_dir: PathBuf::new(),
            authority: Some(WorkflowEventAuthority {
                repository,
                installation_id,
            }),
        }
    }

    #[cfg(test)]
    fn log_path(&self, execution_id: &str) -> PathBuf {
        self.log_dir.join(format!("{execution_id}.ndjson"))
    }

    #[cfg(test)]
    pub fn append(&self, event: &WorkflowEvent) -> Result<(), String> {
        self.append_batch(std::slice::from_ref(event))
    }

    /// Atomically appends one command's event batch.
    ///
    /// Every event in the batch must belong to the same execution. The current
    /// file and the serialized batch are written to a sibling temporary file,
    /// then committed with one rename so a partial batch is never observable.
    #[cfg(test)]
    pub fn append_batch(&self, events: &[WorkflowEvent]) -> Result<(), String> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        let execution_id = first.execution_id();
        validate_execution_id(execution_id)?;
        if let Some(event) = events
            .iter()
            .find(|event| event.execution_id() != execution_id)
        {
            return Err(format!(
                "append_batch requires one execution_id (got {execution_id} and {})",
                event.execution_id()
            ));
        }

        let mut appended = Vec::new();
        for event in events {
            let domain_event = to_domain_event(event)
                .map_err(|error| format!("invalid workflow event semantics: {error}"))?;
            debug_assert_eq!(domain_event.execution_id(), execution_id);
            appended.extend_from_slice(
                &encode_stored_workflow_event_v1(event)
                    .map_err(|error| format!("failed to serialize workflow event: {error}"))?,
            );
            appended.push(b'\n');
        }

        fs::create_dir_all(&self.log_dir).map_err(|error| {
            format!("failed to create workflow execution log directory: {error}")
        })?;
        let path = self.log_path(execution_id);
        let lock = log_file_lock(&path);
        let _guard = lock.lock();
        let existing = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(format!(
                    "failed to read existing workflow execution log: {error}"
                ));
            }
        };
        let temp_path = temporary_log_path(&path)?;
        let write_result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| format!("failed to create temporary workflow log: {error}"))?;
            file.write_all(&existing)
                .map_err(|error| format!("failed to copy workflow log: {error}"))?;
            file.write_all(&appended)
                .map_err(|error| format!("failed to append workflow events: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync workflow event log: {error}"))?;
            fs::rename(&temp_path, &path)
                .map_err(|error| format!("failed to commit workflow event log: {error}"))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        write_result
    }

    #[cfg(test)]
    pub(crate) async fn append_batch_durable_with_mutations(
        &self,
        events: &[WorkflowEvent],
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        self.append_batch_durable_with_mutations_as(
            crate::domain::local_event::CommitOperationKind::Workflow,
            events,
            state_mutations,
        )
        .await
    }

    pub(crate) async fn append_batch_durable_with_mutations_as(
        &self,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        events: &[WorkflowEvent],
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        let Some(first) = events.first() else {
            return if state_mutations.is_empty() {
                Ok(())
            } else {
                Err(
                    "workflow state mutations require an execution event or an explicit projection commit"
                        .to_string(),
                )
            };
        };
        self.commit_execution_batch(
            operation_kind,
            first.execution_id(),
            events,
            state_mutations,
        )
        .await
    }

    /// Commit a workflow projection transition that has no additional domain
    /// event. It still uses the execution stream head and one SQLite batch;
    /// empty-event transitions must not fall back to ExecutionStore JSON.
    pub(crate) async fn commit_projection_durable(
        &self,
        execution_id: &str,
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        self.commit_execution_batch(
            crate::domain::local_event::CommitOperationKind::Workflow,
            execution_id,
            &[],
            state_mutations,
        )
        .await
    }

    async fn commit_execution_batch(
        &self,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        execution_id: &str,
        events: &[WorkflowEvent],
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        let Some(authority) = &self.authority else {
            return Err("workflow SQLite event authority is not configured".to_string());
        };
        validate_execution_id(execution_id)?;
        if events
            .iter()
            .any(|event| event.execution_id() != execution_id)
        {
            return Err("workflow event batch contains multiple executions".to_string());
        }
        let stream_id = crate::domain::local_event::StreamId::workflow(execution_id)
            .map_err(|_| "workflow stream identity is invalid".to_string())?;
        let page = authority
            .repository
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await
            .map_err(|error| format!("workflow SQLite head read failed: {error}"))?;
        if !matches!(
            operation_kind,
            crate::domain::local_event::CommitOperationKind::Workflow
                | crate::domain::local_event::CommitOperationKind::UserMutation
        ) {
            return Err("workflow commit operation kind is invalid".to_string());
        }
        let mut exact = b"workflow_commit_identity_v1".to_vec();
        if operation_kind == crate::domain::local_event::CommitOperationKind::UserMutation {
            // Keep the historic internal-workflow identity stable while giving caller
            // admission commits a distinct, replayable idempotency lane.
            exact.extend_from_slice(b"\0user_mutation");
        }
        exact.extend_from_slice(&(execution_id.len() as u64).to_be_bytes());
        exact.extend_from_slice(execution_id.as_bytes());
        let mut uncommitted = Vec::with_capacity(events.len());
        for event in events {
            let encoded = encode_stored_workflow_event_v1(event)
                .map_err(|error| format!("failed to encode workflow event: {error}"))?;
            exact.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
            exact.extend_from_slice(&encoded);
            let domain = to_domain_event(event)
                .map_err(|error| format!("invalid workflow event semantics: {error}"))?;
            let occurred_at_ms = if event.timestamp().is_finite() && event.timestamp() >= 0.0 {
                (event.timestamp() * 1000.0).round() as i64
            } else {
                return Err("workflow event timestamp is invalid".to_string());
            };
            uncommitted.push(crate::domain::local_event::UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: crate::domain::local_event::LocalDomainEvent::Workflow(domain),
                occurred_at_ms,
            });
        }
        for mutation in &state_mutations {
            let encoded = authority
                .repository
                .canonical_mutation_identity_v1(mutation)?;
            exact.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
            exact.extend_from_slice(&encoded);
        }
        let payload_hash: [u8; 32] = Sha256::digest(&exact).into();
        let identity = format!("workflow-{}", hex::encode(payload_hash));
        let batch = crate::domain::local_event::LocalAtomicBatch {
            commit_id: crate::domain::local_event::CommitIdentity::parse(&identity)
                .map_err(|_| "workflow commit identity is invalid".to_string())?,
            idempotency: crate::domain::local_event::IdempotencyBinding {
                installation_id: authority.installation_id.clone(),
                operation_kind,
                idempotency_key: hex::encode(payload_hash),
                payload_hash,
            },
            expected_heads: vec![crate::domain::local_event::ExpectedStreamHead {
                stream_id,
                expected: page.head,
            }],
            events: uncommitted,
            state_mutations,
        };
        authority
            .repository
            .commit_batch(batch)
            .await
            .map(|_| ())
            .map_err(|error| format!("workflow SQLite event commit failed: {error}"))
    }

    pub(crate) fn commit_projection_durable_blocking(
        &self,
        execution_id: &str,
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create workflow commit runtime: {error}")
                        })?
                        .block_on(self.commit_projection_durable(execution_id, state_mutations))
                })
                .join()
                .map_err(|_| "workflow SQLite projection commit worker panicked".to_string())?
        })
    }

    pub(crate) fn append_batch_durable_with_mutations_blocking_as(
        &self,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        events: &[WorkflowEvent],
        state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create workflow commit runtime: {error}")
                        })?
                        .block_on(self.append_batch_durable_with_mutations_as(
                            operation_kind,
                            events,
                            state_mutations,
                        ))
                })
                .join()
                .map_err(|_| "workflow SQLite event commit worker panicked".to_string())?
        })
    }

    pub(crate) async fn read_log_durable(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEvent>, String> {
        let Some(authority) = &self.authority else {
            #[cfg(test)]
            return self.read_log(execution_id);
            #[cfg(not(test))]
            return Err("workflow SQLite event authority is not configured".to_string());
        };
        validate_execution_id(execution_id)?;
        let stream_id = crate::domain::local_event::StreamId::workflow(execution_id)
            .map_err(|_| "workflow stream identity is invalid".to_string())?;
        let mut after = None;
        let mut result = Vec::new();
        loop {
            let page = authority
                .repository
                .load_stream(crate::domain::local_event::LoadStreamRequest {
                    stream_id: stream_id.clone(),
                    after,
                    limit: 200,
                })
                .await
                .map_err(|error| format!("workflow SQLite event read failed: {error}"))?;
            for event in page.events {
                match event.event {
                    crate::domain::local_event::LoadedDomainEvent::Known(event) => match *event {
                        crate::domain::local_event::LocalDomainEvent::Workflow(event) => result
                            .push(
                                crate::adaptor::gateway::workflow::event::from_domain_event(&event)
                                    .map_err(|error| {
                                        format!("workflow SQLite event conversion failed: {error}")
                                    })?,
                            ),
                        _ => {
                            return Err("workflow SQLite stream contains a foreign domain event"
                                .to_string());
                        }
                    },
                    crate::domain::local_event::LoadedDomainEvent::Unknown { .. } => {
                        return Err(
                            "workflow SQLite event has an unsupported required version".to_string()
                        );
                    }
                }
            }
            let Some(next) = page.next_after else {
                break;
            };
            after = Some(next);
        }
        Ok(result)
    }

    /// Synchronous query-port bridge for the canonical SQLite stream.
    ///
    /// Workflow query traits predate the async local-event authority. Run the
    /// bounded replay on a dedicated current-thread runtime so callers never
    /// fall back to the legacy NDJSON source merely because their port is
    /// synchronous.
    pub(crate) fn read_log_durable_blocking(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEvent>, String> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create workflow read runtime: {error}")
                        })?
                        .block_on(self.read_log_durable(execution_id))
                })
                .join()
                .map_err(|_| "workflow SQLite read worker panicked".to_string())?
        })
    }

    /// Reads and validates one execution log on demand.
    #[cfg(test)]
    pub fn read_log(&self, execution_id: &str) -> Result<Vec<WorkflowEvent>, String> {
        self.read_log_records(execution_id)
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| {
                        let _preserved = record.preserved_additive_payload;
                        record.event
                    })
                    .collect()
            })
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) fn read_log_records(
        &self,
        execution_id: &str,
    ) -> Result<Vec<DecodedStoredWorkflowEventV1>, WorkflowEventLogReadError> {
        validate_execution_id(execution_id).map_err(WorkflowEventLogReadError::Io)?;
        let path = self.log_path(execution_id);
        let content = match fs::read(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(WorkflowEventLogReadError::Io(error.to_string())),
        };
        let source_id = path.to_string_lossy().into_owned();
        let text = std::str::from_utf8(&content)
            .map_err(|error| WorkflowEventLogReadError::Io(error.to_string()))?;
        text.lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                let line_number = index + 1;
                let record = decode_stored_workflow_event_v1(
                    line.as_bytes(),
                    1,
                    StoredWorkflowPayloadSource {
                        source_id: source_id.clone(),
                        record_ordinal: u64::try_from(index).unwrap_or(u64::MAX),
                    },
                )
                .map_err(|source| WorkflowEventLogReadError::Incompatible {
                    line: line_number,
                    source,
                })?;
                if record.event.execution_id() != execution_id {
                    return Err(WorkflowEventLogReadError::ExecutionIdentity {
                        line: line_number,
                        actual: record.event.execution_id().to_string(),
                        expected: execution_id.to_string(),
                    });
                }
                Ok(record)
            })
            .collect()
    }

    /// Streams one execution log and maps each line without first retaining the
    /// complete NDJSON document. Callers can deserialize a payload-stripped
    /// event mirror while preserving the persistence adapter's UUID and
    /// per-line execution ownership checks.
    #[cfg(test)]
    pub(crate) fn read_log_mapped<T, Parse, EventExecutionId>(
        &self,
        execution_id: &str,
        mut parse: Parse,
        event_execution_id: EventExecutionId,
    ) -> Result<Vec<T>, String>
    where
        Parse: FnMut(&str) -> Result<T, String>,
        EventExecutionId: for<'event> Fn(&'event T) -> &'event str,
    {
        validate_execution_id(execution_id)?;
        let path = self.log_path(execution_id);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!("failed to read workflow execution log: {error}"));
            }
        };

        let mut events = Vec::new();
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| {
                format!(
                    "failed to read workflow execution log line {}: {error}",
                    line_index + 1
                )
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let event = parse(&line).map_err(|error| {
                format!(
                    "failed to parse workflow execution log line {}: {error}",
                    line_index + 1
                )
            })?;
            let actual_execution_id = event_execution_id(&event);
            if actual_execution_id != execution_id {
                return Err(format!(
                    "workflow execution log line {} contains execution_id {actual_execution_id} instead of {execution_id}",
                    line_index + 1
                ));
            }
            events.push(event);
        }
        Ok(events)
    }

    /// Reads only the requested event window without retaining the complete log.
    #[cfg(test)]
    pub fn read_log_page(
        &self,
        execution_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<WorkflowEvent>, String> {
        validate_execution_id(execution_id)?;
        let path = self.log_path(execution_id);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!("failed to read workflow execution log: {error}"));
            }
        };

        let mut events = Vec::new();
        let mut event_index = 0;
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| {
                format!(
                    "failed to read workflow execution log line {}: {error}",
                    line_index + 1
                )
            })?;
            if line.trim().is_empty() {
                continue;
            }
            if event_index < offset {
                event_index += 1;
                continue;
            }
            if events.len() == limit {
                break;
            }
            let event = decode_stored_workflow_event_v1(
                line.as_bytes(),
                1,
                StoredWorkflowPayloadSource {
                    source_id: path.to_string_lossy().into_owned(),
                    record_ordinal: u64::try_from(event_index).unwrap_or(u64::MAX),
                },
            )
            .map_err(|error| {
                format!(
                    "failed to parse workflow execution log line {}: {error}",
                    line_index + 1
                )
            })?
            .event;
            if event.execution_id() != execution_id {
                return Err(format!(
                    "workflow execution log line {} contains execution_id {} instead of {execution_id}",
                    line_index + 1,
                    event.execution_id()
                ));
            }
            events.push(event);
            event_index += 1;
        }
        Ok(events)
    }
}

fn validate_execution_id(execution_id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(execution_id)
        .map(|_| ())
        .map_err(|_| "workflow execution log execution_id must be UUID".to_string())
}

#[cfg(test)]
fn temporary_log_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "failed to derive workflow log file name".to_string())?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error while creating workflow log: {error}"))?
        .as_nanos();
    Ok(path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::TokenUsage;

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000002";

    fn event(execution_id: &str, timestamp: f64) -> WorkflowEvent {
        WorkflowEvent::ExecutionCompleted {
            execution_id: execution_id.to_string(),
            total_token_usage: TokenUsage::default(),
            timestamp,
        }
    }

    #[test]
    fn append_batch_round_trips_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        log.append_batch(&[event(EXECUTION_ID, 1.0), event(EXECUTION_ID, 2.0)])
            .unwrap();

        let events = log.read_log(EXECUTION_ID).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].timestamp(), 1.0);
        assert_eq!(events[1].timestamp(), 2.0);
        assert!(temp
            .path()
            .join("workflow_execution_logs")
            .join(format!("{EXECUTION_ID}.ndjson"))
            .is_file());
    }

    #[test]
    fn production_read_append_reload_preserves_additive_raw_and_source_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        fs::create_dir_all(&log.log_dir).unwrap();
        let path = log.log_path(EXECUTION_ID);
        let raw = format!(
            "{{\"event\":\"execution_completed\",\"execution_id\":\"{EXECUTION_ID}\",\"total_token_usage\":{{\"inputTokens\":1,\"outputTokens\":2,\"futureNested\":true}},\"timestamp\":1.0,\"futureTop\":{{\"x\":1}}}}\n"
        );
        fs::write(&path, raw.as_bytes()).unwrap();

        let records = log.read_log_records(EXECUTION_ID).unwrap();
        let preserved = records[0].preserved_additive_payload.as_ref().unwrap();
        assert_eq!(preserved.raw_bytes, raw.trim_end().as_bytes());
        assert_eq!(preserved.source.source_id, path.to_string_lossy());
        assert_eq!(preserved.source.record_ordinal, 0);

        log.append(&event(EXECUTION_ID, 2.0)).unwrap();
        let rewritten = fs::read(&path).unwrap();
        assert!(rewritten.starts_with(raw.as_bytes()));
        let reloaded = WorkflowEventLog::new(temp.path())
            .read_log_records(EXECUTION_ID)
            .unwrap();
        assert_eq!(
            reloaded[0]
                .preserved_additive_payload
                .as_ref()
                .unwrap()
                .raw_bytes,
            raw.trim_end().as_bytes()
        );
    }

    #[test]
    fn production_reader_returns_typed_incompatibility_for_unknown_required_event() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        fs::create_dir_all(&log.log_dir).unwrap();
        fs::write(
            log.log_path(EXECUTION_ID),
            format!("{{\"event\":\"future_required\",\"execution_id\":\"{EXECUTION_ID}\"}}\n"),
        )
        .unwrap();
        assert!(matches!(
            log.read_log_records(EXECUTION_ID),
            Err(WorkflowEventLogReadError::Incompatible { .. })
        ));
    }

    #[test]
    fn read_log_page_returns_only_the_requested_event_window() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        log.append_batch(&[
            event(EXECUTION_ID, 1.0),
            event(EXECUTION_ID, 2.0),
            event(EXECUTION_ID, 3.0),
        ])
        .unwrap();

        let events = log.read_log_page(EXECUTION_ID, 1, 1).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timestamp(), 2.0);
    }

    #[test]
    fn mixed_execution_batch_is_rejected_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        assert!(log
            .append_batch(&[event(EXECUTION_ID, 1.0), event(OTHER_EXECUTION_ID, 2.0),])
            .is_err());
        assert!(log.read_log(EXECUTION_ID).unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_event_batch_never_reports_uncommitted_state_mutations_as_success() {
        let temp = tempfile::tempdir().unwrap();
        let log = WorkflowEventLog::new(temp.path());
        let mutation = crate::domain::local_event::LocalStateMutation::SessionProjectionRemoval(
            crate::domain::local_event::SessionProjectionRemovalMutation {
                session_id: "session-1".to_string(),
                expected: crate::domain::local_event::RevisionGuard::Absent,
            },
        );

        let error = log
            .append_batch_durable_with_mutations(&[], vec![mutation])
            .await
            .unwrap_err();

        assert!(error.contains("explicit projection commit"));
    }

    #[test]
    fn legacy_log_directory_is_not_read() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_dir = temp.path().join("workflow_logs");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join(format!("{EXECUTION_ID}.ndjson")),
            r#"{"event":"run_completed","run_id":"legacy","timestamp":1}"#,
        )
        .unwrap();

        assert!(WorkflowEventLog::new(temp.path())
            .read_log(EXECUTION_ID)
            .unwrap()
            .is_empty());
    }
}
