//! Bounded reader pool and the closed query implementations.
//!
//! Up to four dedicated reader threads own read-only connections; rusqlite
//! is never called on a tokio task. Every public query is a point / range
//! lookup over a direct index or projection table — never a scan of
//! `events` and never a full-history fold.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::oneshot;

use crate::adaptor::gateway::local_event_store::envelope::{
    DecodedStoredEvent, EventCodecRegistry,
};
use crate::adaptor::gateway::local_event_store::projection_record_codec::decode_session_projection_record_v1;
use crate::common::operation_context::OperationContext;

use crate::domain::local_event::{
    CanonicalRuntimeOwnerView, CommitIdentity, CommittedDomainEvent, DomainEventPage, EventId,
    LoadStreamRequest, LoadedDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, SafeOperationFailure, SessionOperationFailureKind,
    SessionProjectionRecord, SessionProjectionView, StreamSequence, StreamVersion,
};

pub const READER_POOL_SIZE: usize = 4;
pub const READ_QUEUE_MAX_DEPTH: usize = 128;
pub const QUERY_DEADLINE_MS: i64 = 2_000;
pub const MAX_STREAM_PAGE: usize = 200;
pub const STREAM_PAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT: usize = 8_192;
pub struct QueryContext {
    pub registry: Arc<EventCodecRegistry>,
}

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn storage_unavailable(error: &rusqlite::Error) -> LocalEventQueryError {
    use crate::adaptor::gateway::shared::sqlite_failure::{condition, SqliteFailureCondition};
    let correlation = correlation_id();
    log::warn!("local event store read failure [{correlation}]: {error}");
    match condition(error) {
        SqliteFailureCondition::Busy => LocalEventQueryError::QueryBusy,
        SqliteFailureCondition::Corrupt => LocalEventQueryError::Corrupt {
            correlation_id: correlation,
        },
        SqliteFailureCondition::Inaccessible => LocalEventQueryError::StorageAccessRequired {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                crate::adaptor::gateway::shared::sqlite_failure::nature(error),
                "local event store read failed",
                correlation,
            ),
        },
        _ => LocalEventQueryError::Internal {
            correlation_id: correlation,
        },
    }
}

fn corrupt(context: &str) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event store corrupt read [{correlation}]: {context}");
    LocalEventQueryError::Corrupt {
        correlation_id: correlation,
    }
}

fn reader_pool_unavailable(
    message: &'static str,
    nature: crate::domain::failure::TechnicalFailureNature,
) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event reader pool failure [{correlation}]: {message}");
    let failure = SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        nature,
        message,
        correlation,
    );
    if nature == crate::domain::failure::TechnicalFailureNature::Transient {
        LocalEventQueryError::StorageUnavailable { failure }
    } else {
        LocalEventQueryError::StorageAccessRequired { failure }
    }
}

fn session_projection_record(
    raw: String,
    session_id: &str,
    context: &'static str,
) -> Result<SessionProjectionRecord, LocalEventQueryError> {
    decode_session_projection_record_v1(&raw, session_id).map_err(|_| corrupt(context))
}

pub fn run_query(
    connection: &Connection,
    query: &LocalEventQuery,
) -> Result<LocalEventQueryResult, LocalEventQueryError> {
    match query {
        LocalEventQuery::SessionProjectionByIdentity { session_id } => {
            Ok(LocalEventQueryResult::SessionProjectionByIdentity(
                session_projection_by_identity(connection, session_id)?,
            ))
        }
        LocalEventQuery::CanonicalRuntimeOwnerSnapshot { limit } => {
            Ok(LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(
                canonical_runtime_owner_snapshot(connection, *limit)?,
            ))
        }
    }
}

pub fn load_stream_page(
    connection: &Connection,
    context: &QueryContext,
    request: &LoadStreamRequest,
) -> Result<DomainEventPage, LocalEventQueryError> {
    if request.limit == 0 {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let limit = request.limit.min(MAX_STREAM_PAGE);
    let after = request.after.map(|sequence| sequence.value()).unwrap_or(0);
    let head: Option<i64> = connection
        .query_row(
            "SELECT head FROM stream_heads WHERE stream_id = ?1",
            params![request.stream_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let head = StreamVersion::new(head.unwrap_or(0)).map_err(|_| corrupt("stream head"))?;

    let mut statement = connection
        .prepare(
            "SELECT event_id, commit_id, stream_sequence, global_sequence, event_type,
                    payload_version, occurred_at, payload
             FROM events
             WHERE stream_id = ?1 AND stream_sequence > ?2
             ORDER BY stream_sequence
             LIMIT ?3",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(
            params![request.stream_id.as_str(), after, limit as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                ))
            },
        )
        .map_err(|error| storage_unavailable(&error))?;

    let mut events = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let (
            event_id,
            commit_id,
            stream_sequence,
            global_sequence,
            event_type,
            payload_version,
            occurred_at,
            payload,
        ) = row.map_err(|error| storage_unavailable(&error))?;
        if !events.is_empty() && bytes + payload.len() > STREAM_PAGE_MAX_BYTES {
            break;
        }
        bytes += payload.len();
        let decoded = context
            .registry
            .decode(&event_type, payload_version, &payload)
            .map_err(|_| LocalEventQueryError::IncompatibleStoredEvent {
                correlation_id: correlation_id(),
            })?;
        let event = match decoded {
            DecodedStoredEvent::Known(event) => LoadedDomainEvent::Known(event),
            DecodedStoredEvent::Unknown => LoadedDomainEvent::Unknown {
                event_type: event_type.clone(),
                payload_version,
            },
        };
        events.push(CommittedDomainEvent {
            event_id: EventId::parse(&event_id).map_err(|_| corrupt("stored event id"))?,
            commit_id: CommitIdentity::parse(&commit_id)
                .map_err(|_| corrupt("stored commit id"))?,
            stream_id: request.stream_id.clone(),
            stream_sequence: StreamSequence::new(stream_sequence)
                .map_err(|_| corrupt("stored stream sequence"))?,
            global_sequence: crate::domain::local_event::GlobalSequence::new(global_sequence)
                .map_err(|_| corrupt("stored global sequence"))?,
            occurred_at_ms: occurred_at
                .parse()
                .map_err(|_| corrupt("stored occurred_at"))?,
            event,
        });
    }
    let next_after = events
        .last()
        .filter(|event| event.stream_sequence.value() < head.value())
        .map(|event| event.stream_sequence);
    Ok(DomainEventPage {
        events,
        head,
        next_after,
    })
}

fn session_projection_by_identity(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<SessionProjectionView>, LocalEventQueryError> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT projection, revision FROM session_projection WHERE session_id = ?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(|(projection, revision)| {
        Ok(SessionProjectionView {
            session_id: session_id.to_string(),
            projection: session_projection_record(projection, session_id, "session projection")?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("session projection revision"))?,
        })
    })
    .transpose()
}

pub fn canonical_runtime_owner_snapshot(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<CanonicalRuntimeOwnerView>, LocalEventQueryError> {
    use crate::adaptor::gateway::workflow::fact_log::records_from_tree_rows;
    use crate::domain::workflow::services::fact_replay;
    use crate::domain::workflow::{ExecutionTreeLaunch, NodeFact};

    if limit == 0 || limit > MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    // 統一 Node 事実ログの木ごとの fold で生存 owner を導出する。
    let roots = super::node_events::list_tree_roots(connection, "started")
        .map_err(|error| storage_unavailable(&error))?;
    let mut owners = Vec::new();
    for root in roots {
        let rows = super::node_events::read_tree(connection, &root.tree_id)
            .map_err(|error| storage_unavailable(&error))?;
        let records = records_from_tree_rows(&rows).map_err(|error| corrupt(&error))?;
        let Some(NodeFact::Started(started)) = records.first().map(|record| &record.fact) else {
            continue;
        };
        match &started.root {
            Some(tree_root) if tree_root.launched_as == ExecutionTreeLaunch::Session => {
                let Some(session_id) = records.iter().find_map(|record| match &record.fact {
                    NodeFact::SessionAttached(attached)
                        if record.meta.node_execution_id == root.node_execution_id =>
                    {
                        Some(attached.session_id.as_str())
                    }
                    _ => None,
                }) else {
                    continue;
                };
                let view = fact_replay::derive_session_facts(
                    &records,
                    &root.node_execution_id,
                    session_id,
                );
                if view.is_open() {
                    owners.push(CanonicalRuntimeOwnerView::AgentSession {
                        worktree_path: tree_root.worktree_path.clone(),
                        active: true,
                    });
                }
            }
            Some(tree_root) if tree_root.launched_as == ExecutionTreeLaunch::Workflow => {
                let folded = fact_replay::fold_execution_tree(&root.tree_id, &records)
                    .map_err(|error| corrupt(&error))?
                    .ok_or_else(|| corrupt("runtime owner tree fold missing"))?;
                let read_model = fact_replay::derive_read_model(&folded);
                if read_model.status.is_active() {
                    owners.push(CanonicalRuntimeOwnerView::ActiveWorkflow {
                        worktree_path: tree_root.worktree_path.clone(),
                    });
                }
            }
            Some(_) | None => {}
        }
        if owners.len() > limit {
            return Err(LocalEventQueryError::ResponseTooLarge);
        }
    }
    Ok(owners)
}

type ReadTask = Box<dyn FnOnce(&Connection) + Send>;

pub struct ReadJob {
    context: OperationContext,
    task: ReadTask,
}

struct ReadQueueState {
    jobs: VecDeque<ReadJob>,
    closed: bool,
}

/// Shared bounded job queue feeding the dedicated reader threads.
pub struct ReaderPool {
    state: Mutex<ReadQueueState>,
    available: Condvar,
    #[cfg(any(test, feature = "test-support"))]
    running_workers: std::sync::atomic::AtomicUsize,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) next_failure: Mutex<Option<super::test_helpers::ReadFailure>>,
}

impl ReaderPool {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ReadQueueState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            available: Condvar::new(),
            #[cfg(any(test, feature = "test-support"))]
            running_workers: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-support"))]
            next_failure: Mutex::new(None),
        })
    }

    /// Submit a query; `QueryBusy` when the bounded queue is full.
    pub async fn submit<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        crate::common::operation_context::timeout(
            std::time::Duration::from_millis(QUERY_DEADLINE_MS as u64),
            self.submit_scoped(run),
        )
        .await?
    }

    async fn submit_scoped<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        #[cfg(any(test, feature = "test-support"))]
        let failure = self.next_failure.lock().unwrap().take();
        let (reply, receiver) = oneshot::channel();
        let context = crate::common::operation_context::current();
        {
            let mut state = self.state.lock().expect("reader queue poisoned");
            if state.closed {
                return Err(reader_pool_unavailable(
                    "local event store reader pool is closed",
                    crate::domain::failure::TechnicalFailureNature::Other,
                ));
            }
            if state.jobs.len() >= READ_QUEUE_MAX_DEPTH {
                return Err(LocalEventQueryError::QueryBusy);
            }
            state.jobs.push_back(ReadJob {
                context: context.clone(),
                task: Box::new(move |connection| {
                    let job_context = crate::common::operation_context::current();
                    #[cfg(any(test, feature = "test-support"))]
                    let run = |connection: &Connection| match failure {
                        Some(failure) => failure.run(connection, run),
                        None => run(connection),
                    };
                    let result = crate::common::operation_context::checked(|| {
                        let progress_context = job_context.clone();
                        connection
                            .progress_handler(
                                1,
                                Some(move || {
                                    progress_context.check(std::time::Instant::now()).is_err()
                                }),
                            )
                            .map_err(|error| storage_unavailable(&error))?;
                        let interrupt = connection.get_interrupt_handle();
                        let (done, stopped) = std::sync::mpsc::channel();
                        let result = std::thread::scope(|scope| {
                            let context = &job_context;
                            scope.spawn(move || loop {
                                match stopped.recv_timeout(std::time::Duration::from_millis(1)) {
                                    Ok(())
                                    | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                        if context.check(std::time::Instant::now()).is_err() {
                                            interrupt.interrupt();
                                        }
                                    }
                                }
                            });
                            let result = run(connection);
                            drop(done);
                            result
                        });
                        connection
                            .progress_handler(0, None::<fn() -> bool>)
                            .map_err(|error| storage_unavailable(&error))?;
                        result
                    })
                    .map_err(LocalEventQueryError::from)
                    .and_then(std::convert::identity);
                    let _ = reply.send(result);
                }),
            });
        }
        self.available.notify_one();
        receiver.await.map_err(|_| {
            reader_pool_unavailable(
                "local event store reader reply lost",
                crate::domain::failure::TechnicalFailureNature::Transient,
            )
        })?
    }

    pub fn pop_blocking(&self) -> Option<ReadJob> {
        let mut state = self.state.lock().expect("reader queue poisoned");
        loop {
            if let Some(job) = state.jobs.pop_front() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state = self.available.wait(state).expect("reader queue poisoned");
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().expect("reader queue poisoned");
        state.closed = true;
        state.jobs.clear();
        drop(state);
        self.available.notify_all();
    }

    /// Worker loop for one dedicated reader thread.
    pub fn run_worker(
        self: &Arc<Self>,
        connection: crate::infrastructure::local_event_store_connection::ManagedConnection,
    ) {
        #[cfg(any(test, feature = "test-support"))]
        self.running_workers
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        while let Some(job) = self.pop_blocking() {
            crate::common::operation_context::sync_scope(job.context, || (job.task)(&connection));
        }
        #[cfg(any(test, feature = "test-support"))]
        self.running_workers
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

// --- Snapshot-stable recovery pager ---

#[cfg(test)]
#[path = "reader_test.rs"]
mod reader_tests;

#[cfg(feature = "test-support")]
impl ReadJob {
    pub fn test_into_parts(self) -> (OperationContext, ReadTask) {
        (self.context, self.task)
    }
}
