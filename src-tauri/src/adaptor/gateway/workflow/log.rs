use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;

/// NDJSON persistence adapter for workflow execution events.
///
/// The adapter deliberately performs on-demand reads. Projection and lifecycle
/// decisions belong to the workflow query/use-case layer, not to persistence.
pub struct WorkflowEventLog {
    log_dir: PathBuf,
}

static LOG_FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn log_file_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut locks = LOG_FILE_LOCKS.lock();
    Arc::clone(
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

impl WorkflowEventLog {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            log_dir: data_dir.join("workflow_execution_logs"),
        }
    }

    fn log_path(&self, execution_id: &str) -> PathBuf {
        self.log_dir.join(format!("{execution_id}.ndjson"))
    }

    pub(crate) fn gc_delete_paths(&self, execution_id: &str) -> Vec<PathBuf> {
        vec![self.log_path(execution_id)]
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
            serde_json::to_writer(&mut appended, event)
                .map_err(|error| format!("failed to serialize workflow event: {error}"))?;
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

    /// Reads and validates one execution log on demand.
    pub fn read_log(&self, execution_id: &str) -> Result<Vec<WorkflowEvent>, String> {
        validate_execution_id(execution_id)?;
        let path = self.log_path(execution_id);
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!("failed to read workflow execution log: {error}"));
            }
        };

        content
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                let event: WorkflowEvent = serde_json::from_str(line).map_err(|error| {
                    format!(
                        "failed to parse workflow execution log line {}: {error}",
                        index + 1
                    )
                })?;
                if event.execution_id() != execution_id {
                    return Err(format!(
                        "workflow execution log line {} contains execution_id {} instead of {execution_id}",
                        index + 1,
                        event.execution_id()
                    ));
                }
                Ok(event)
            })
            .collect()
    }

    /// Streams one execution log and maps each line without first retaining the
    /// complete NDJSON document. Callers can deserialize a payload-stripped
    /// event mirror while preserving the persistence adapter's UUID and
    /// per-line execution ownership checks.
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
            let event: WorkflowEvent = serde_json::from_str(&line).map_err(|error| {
                format!(
                    "failed to parse workflow execution log line {}: {error}",
                    line_index + 1
                )
            })?;
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
    use crate::adaptor::gateway::workflow::event::TokenUsage;

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
