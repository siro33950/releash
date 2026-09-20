//! Bounded reader pool and the closed query implementations.
//!
//! Up to four dedicated reader threads own read-only connections; rusqlite
//! is never called on a tokio task. Every public query is a point / range
//! lookup over a direct index or projection table — never a scan of
//! `events` and never a full-history fold.

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Condvar, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::oneshot;

use crate::adaptor::gateway::local_event_store::clock::StoreClock;
use crate::adaptor::gateway::local_event_store::envelope::{
    DecodedStoredEvent, EventCodecRegistry,
};
use crate::adaptor::gateway::local_event_store::projection_record_codec::decode_session_projection_record_v1;

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

pub(crate) fn storage_unavailable(error: &rusqlite::Error) -> LocalEventQueryError {
    // Concurrent-commit contention surfaces as SQLITE_BUSY / SQLITE_LOCKED
    // after the 250 ms busy timeout; that is `QueryBusy`, not a storage
    // failure (B-069).
    if let rusqlite::Error::SqliteFailure(inner, _) = error {
        if matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ) {
            return LocalEventQueryError::QueryBusy;
        }
    }
    let correlation = correlation_id();
    log::warn!("local event store read failure [{correlation}]: {error}");
    LocalEventQueryError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "local event store read failed",
            correlation,
        ),
    }
}

fn corrupt(context: &str) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event store corrupt read [{correlation}]: {context}");
    LocalEventQueryError::Corrupt {
        correlation_id: correlation,
    }
}

fn reader_pool_unavailable(message: &'static str, retryable: bool) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event reader pool failure [{correlation}]: {message}");
    LocalEventQueryError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            retryable,
            message,
            correlation,
        ),
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

fn canonical_runtime_owner_snapshot(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<CanonicalRuntimeOwnerView>, LocalEventQueryError> {
    use crate::adaptor::gateway::workflow::fact_log::record_from_row;
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
        let records = super::node_events::read_tree(connection, &root.tree_id)
            .map_err(|error| storage_unavailable(&error))?
            .iter()
            .filter_map(|row| record_from_row(row).transpose())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| LocalEventQueryError::InvalidRequest)?;
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
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?
                    .ok_or(LocalEventQueryError::InvalidRequest)?;
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

type ReadTask = Box<dyn FnOnce(&Connection, bool) + Send>;

struct ReadJob {
    deadline_ms: i64,
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
    clock: Arc<dyn StoreClock>,
    #[cfg(test)]
    running_workers: std::sync::atomic::AtomicUsize,
}

impl ReaderPool {
    pub fn new(clock: Arc<dyn StoreClock>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ReadQueueState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            available: Condvar::new(),
            clock,
            #[cfg(test)]
            running_workers: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Submit a query; `QueryBusy` when the bounded queue is full.
    pub fn submit<T, F>(
        &self,
        run: F,
    ) -> Result<oneshot::Receiver<Result<T, LocalEventQueryError>>, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let (reply, receiver) = oneshot::channel();
        let deadline_ms = self.clock.now_ms() + QUERY_DEADLINE_MS;
        let mut state = self.state.lock().expect("reader queue poisoned");
        if state.closed {
            return Err(reader_pool_unavailable(
                "local event store reader pool is closed",
                false,
            ));
        }
        if state.jobs.len() >= READ_QUEUE_MAX_DEPTH {
            return Err(LocalEventQueryError::QueryBusy);
        }
        state.jobs.push_back(ReadJob {
            deadline_ms,
            task: Box::new(move |connection, deadline_exceeded| {
                if deadline_exceeded {
                    let _ = reply.send(Err(LocalEventQueryError::DeadlineExceeded));
                    return;
                }
                let _ = reply.send(run(connection));
            }),
        });
        drop(state);
        self.available.notify_one();
        Ok(receiver)
    }

    /// Synchronous facade over the same fixed reader workers.
    ///
    /// This exists for established synchronous application ports. It does not
    /// create a thread, runtime, or SQLite connection per call.
    pub fn submit_blocking<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let (reply, receiver) = mpsc::sync_channel(1);
        let deadline_ms = self.clock.now_ms() + QUERY_DEADLINE_MS;
        let mut state = self.state.lock().expect("reader queue poisoned");
        if state.closed {
            return Err(reader_pool_unavailable(
                "local event store reader pool is closed",
                false,
            ));
        }
        if state.jobs.len() >= READ_QUEUE_MAX_DEPTH {
            return Err(LocalEventQueryError::QueryBusy);
        }
        state.jobs.push_back(ReadJob {
            deadline_ms,
            task: Box::new(move |connection, deadline_exceeded| {
                let result = if deadline_exceeded {
                    Err(LocalEventQueryError::DeadlineExceeded)
                } else {
                    run(connection)
                };
                let _ = reply.send(result);
            }),
        });
        drop(state);
        self.available.notify_one();
        receiver
            .recv()
            .map_err(|_| reader_pool_unavailable("local event store reader reply lost", true))?
    }

    fn pop_blocking(&self) -> Option<ReadJob> {
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
    pub fn run_worker(self: &Arc<Self>, connection: Connection) {
        #[cfg(test)]
        self.running_workers
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        while let Some(job) = self.pop_blocking() {
            let deadline_exceeded = self.clock.now_ms() > job.deadline_ms;
            (job.task)(&connection, deadline_exceeded);
        }
        #[cfg(test)]
        self.running_workers
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

// --- Snapshot-stable recovery pager ---

#[cfg(test)]
mod canonical_runtime_owner_snapshot_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
    use crate::adaptor::gateway::local_event_store::schema::{
        initialize_schema, InitialStoreMetadata,
    };
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionTreeLaunch, NodeCompletion, NodeDefinition, NodeFact, NodeKind,
        SessionAttachedFact, SessionExecutionTreeRootFacts, SessionSpec, StartedFact, TreeRootFact,
        WorkflowDefinition,
    };

    fn connection_with_node_events() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        initialize_schema(
            &connection,
            &InitialStoreMetadata {
                installation_id: "00000000-0000-4000-8000-000000000001",
                created_at_ms: 1,
            },
            &FaultInjector::new(),
        )
        .expect("initialize schema");
        connection
    }

    fn insert_root(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        let node_name = match fact {
            NodeFact::Started(StartedFact {
                root: Some(root), ..
            }) => root.definition.entry.as_str(),
            _ => panic!("root fact must contain a definition"),
        };
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 1, ?1, NULL, ?2, 'session', 1, ?3, ?4, 1)",
                params![
                    tree_id,
                    node_name,
                    fact.event_type(),
                    fact.encode_detail().unwrap()
                ],
            )
            .expect("insert root fact");
    }

    fn insert_second_fact(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 2, ?1, NULL, 'session', 'session', 1, ?2, ?3, 2)",
                params![tree_id, fact.event_type(), fact.encode_detail().unwrap()],
            )
            .expect("insert second fact");
    }

    fn insert_third_fact(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 3, ?1, NULL, 'session', 'session', 1, ?2, ?3, 3)",
                params![tree_id, fact.event_type(), fact.encode_detail().unwrap()],
            )
            .expect("insert third fact");
    }

    fn session_root(session_id: &str, workspace_identity: &str, worktree_path: &str) -> NodeFact {
        SessionExecutionTreeRootFacts::new(
            session_id,
            workspace_identity,
            worktree_path,
            ProviderKind::Codex,
        )
        .unwrap()
        .started
    }

    fn session_attached(session_id: &str) -> NodeFact {
        NodeFact::SessionAttached(SessionAttachedFact {
            session_id: session_id.to_string(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
        })
    }

    fn workflow_root(worktree_path: &str) -> NodeFact {
        NodeFact::Started(StartedFact {
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: None,
                definition_resolution: Default::default(),
                workspace_identity: worktree_path.to_string(),
                worktree_path: worktree_path.to_string(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                definition: WorkflowDefinition {
                    name: "wf".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Session(SessionSpec::default()),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::default(),
                        worktree: None,
                    }],
                    entry: "main".to_string(),
                },
                launched_as: ExecutionTreeLaunch::Workflow,
            })),
        })
    }

    fn connection_with_active_workflow_owners(count: usize) -> Connection {
        let connection = connection_with_node_events();
        for index in 0..count {
            insert_root(
                &connection,
                &format!("execution-{index}"),
                &workflow_root(&format!("/snapshot/worktree-{index}")),
            );
        }
        connection
    }

    #[test]
    fn app_data_gc_owner_snapshot_returns_one_bounded_lightweight_inventory() {
        let connection = connection_with_active_workflow_owners(2);

        let owners =
            canonical_runtime_owner_snapshot(&connection, 2).expect("complete owner snapshot");

        assert_eq!(owners.len(), 2);
        assert!(owners
            .iter()
            .all(|owner| matches!(owner, CanonicalRuntimeOwnerView::ActiveWorkflow { .. })));
    }

    #[test]
    fn app_data_gc_owner_snapshot_lists_open_session_trees() {
        let connection = connection_with_node_events();
        insert_root(
            &connection,
            "agent-session-1",
            &session_root(
                "agent-session-1",
                "/snapshot/worktree-a",
                "/snapshot/worktree-a",
            ),
        );
        insert_second_fact(
            &connection,
            "agent-session-1",
            &session_attached("agent-session-1"),
        );

        let owners =
            canonical_runtime_owner_snapshot(&connection, 8).expect("complete owner snapshot");

        assert_eq!(
            owners,
            vec![CanonicalRuntimeOwnerView::AgentSession {
                worktree_path: "/snapshot/worktree-a".to_string(),
                active: true,
            }]
        );
    }

    #[test]
    fn app_data_gc_owner_snapshot_limit_plus_one_fails_closed() {
        let connection = connection_with_active_workflow_owners(2);

        assert_eq!(
            canonical_runtime_owner_snapshot(&connection, 1),
            Err(LocalEventQueryError::ResponseTooLarge)
        );
    }

    #[test]
    fn app_data_gc_owner_snapshot_applies_limit_after_closed_sessions_are_removed() {
        let connection = connection_with_active_workflow_owners(1);
        for session_id in ["closed-session-1", "closed-session-2"] {
            insert_root(
                &connection,
                session_id,
                &session_root(session_id, "/snapshot", &format!("/snapshot/{session_id}")),
            );
            insert_second_fact(&connection, session_id, &session_attached(session_id));
            insert_third_fact(&connection, session_id, &NodeFact::ArchiveRequested);
        }

        let owners = canonical_runtime_owner_snapshot(&connection, 1).unwrap();

        assert_eq!(owners.len(), 1);
        assert!(matches!(
            owners[0],
            CanonicalRuntimeOwnerView::ActiveWorkflow { .. }
        ));
    }
}
