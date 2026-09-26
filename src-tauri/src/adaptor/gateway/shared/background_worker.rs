use super::background_io;
use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
use crate::infrastructure::process::background_worker::{BackgroundWorker, FRAME_PREFIX};
use crate::infrastructure::terminal::terminal_emulator::{
    NativeTerminalCheckpoint, NativeTerminalCheckpointRecord, TerminalCheckpointFileStore,
};
use crate::usecase::work_queue::WorkFailure;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    WatchStart(PathBuf),
    WatchPoll,
    RepositoryScan(String),
    CheckpointAppend {
        store: TerminalCheckpointFileStore,
        key: String,
        base: Option<NativeTerminalCheckpoint>,
        records: Vec<NativeTerminalCheckpointRecord>,
    },
    CheckpointCompact {
        store: TerminalCheckpointFileStore,
        key: String,
        checkpoint: NativeTerminalCheckpoint,
    },
    #[cfg(test)]
    Block(PathBuf),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Failure")]
enum FailureWire {
    Business(#[serde(with = "BusinessFailureWire")] BusinessFailure),
    Technical(#[serde(with = "TechnicalFailureNatureWire")] TechnicalFailureNature),
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "BusinessFailure")]
enum BusinessFailureWire {
    VersionConflict,
    Other,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "TechnicalFailureNature")]
enum TechnicalFailureNatureWire {
    Transient,
    TimedOut,
    Cancelled,
    Other,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WorkerFailure {
    #[serde(with = "FailureWire")]
    kind: Failure,
    message: String,
}
impl From<WorkFailure> for WorkerFailure {
    fn from(error: WorkFailure) -> Self {
        Self {
            kind: error.kind,
            message: error.message,
        }
    }
}
impl From<WorkerFailure> for WorkFailure {
    fn from(error: WorkerFailure) -> Self {
        Self {
            kind: error.kind,
            message: error.message,
        }
    }
}

pub(crate) async fn request<T: serde::de::DeserializeOwned>(
    worker: &mut BackgroundWorker,
    request: &Request,
) -> Result<T, WorkFailure> {
    #[cfg(test)]
    let blocked = BLOCKED_REQUEST
        .try_with(|blocked| {
            *blocked.child.lock().unwrap() = Some(worker.child());
            Request::Block(blocked.path.clone())
        })
        .ok();
    #[cfg(test)]
    let request = blocked.as_ref().unwrap_or(request);
    let bytes = serde_json::to_vec(request).map_err(encoding_failure)?;
    let response = worker
        .request(&bytes)
        .await
        .map_err(background_io::failure)?;
    let response: Result<T, WorkerFailure> =
        serde_json::from_str(&response).map_err(encoding_failure)?;
    response.map_err(Into::into)
}

pub(crate) async fn execute<T: serde::de::DeserializeOwned>(
    request: &Request,
) -> Result<T, WorkFailure> {
    let mut worker = BackgroundWorker::start(Arc::new(|| {})).map_err(background_io::failure)?;
    let result = self::request(&mut worker, request).await;
    worker.stop().await.map_err(background_io::failure)?;
    result
}

fn encoding_failure(error: serde_json::Error) -> WorkFailure {
    WorkFailure {
        kind: Failure::Technical(TechnicalFailureNature::Other),
        message: error.to_string(),
    }
}

pub(crate) fn serve(
    scanner: &dyn crate::usecase::repository_state::scanner::RepositoryScanner,
) -> Result<(), String> {
    let emit = |frame: &str| -> std::io::Result<()> {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{FRAME_PREFIX}{frame}")?;
        stdout.flush()
    };
    let mut watcher = None;
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        let request: Request = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        let result = (|| -> Result<serde_json::Value, WorkFailure> {
            match request {
                Request::WatchStart(dir) => {
                    watcher = Some(
                        crate::infrastructure::comment::watcher::ReviewCommentsWatcher::start(
                            dir,
                            Arc::new(move || {
                                if let Err(error) = emit("changed") {
                                    eprintln!("background watch notification failed: {error}");
                                }
                            }),
                        )
                        .map_err(background_io::failure)?,
                    );
                    Ok(serde_json::Value::Null)
                }
                Request::WatchPoll => {
                    watcher
                        .as_mut()
                        .ok_or_else(|| WorkFailure {
                            kind: Failure::Business(BusinessFailure::Other),
                            message: "watcher is not started".into(),
                        })?
                        .poll()
                        .map_err(background_io::failure)?;
                    Ok(serde_json::Value::Null)
                }
                Request::RepositoryScan(path) => serde_json::to_value(
                    scanner
                        .scan(&path)
                        .map_err(|error| WorkFailure::from_error(&error))?,
                )
                .map_err(encoding_failure),
                Request::CheckpointAppend {
                    store,
                    key,
                    base,
                    records,
                } => {
                    if let Some(base) = base {
                        store
                            .replace_base(&key, &base)
                            .map_err(checkpoint_failure)?;
                    }
                    store
                        .append_records(&key, &records)
                        .map_err(checkpoint_failure)?;
                    Ok(store.journal_len(&key).map_err(checkpoint_failure)?.into())
                }
                Request::CheckpointCompact {
                    store,
                    key,
                    checkpoint,
                } => {
                    store
                        .replace_base(&key, &checkpoint)
                        .map_err(checkpoint_failure)?;
                    Ok(serde_json::Value::Null)
                }
                #[cfg(test)]
                Request::Block(path) => {
                    let file = std::fs::File::create(path).map_err(background_io::failure)?;
                    fs2::FileExt::lock_exclusive(&file).map_err(background_io::failure)?;
                    emit("changed").map_err(background_io::failure)?;
                    loop {
                        std::thread::park();
                    }
                }
            }
        })();
        let result = result.map_err(WorkerFailure::from);
        emit(&serde_json::to_string(&result).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn checkpoint_failure(
    error: crate::infrastructure::terminal::terminal_emulator::CheckpointWriteError,
) -> WorkFailure {
    match error {
        crate::infrastructure::terminal::terminal_emulator::CheckpointWriteError::Io(error) => {
            background_io::failure(error)
        }
        crate::infrastructure::terminal::terminal_emulator::CheckpointWriteError::Encode(error) => {
            encoding_failure(error)
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct BlockedRequest {
    path: PathBuf,
    child: Arc<std::sync::Mutex<Option<crate::infrastructure::process::attempt::SharedChild>>>,
}
#[cfg(test)]
tokio::task_local! { static BLOCKED_REQUEST: BlockedRequest; }

#[cfg(test)]
#[path = "background_worker_test.rs"]
pub(crate) mod background_worker_tests;
