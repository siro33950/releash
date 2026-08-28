//! Single-writer worker with a bounded two-lane write queue.
//!
//! One dedicated thread owns the writer connection; rusqlite is never called
//! on a tokio task. The critical lane (terminal / Stop / shutdown closure)
//! is always drained first so normal backlog cannot starve it. Lane bounds
//! are enforced at admission time, before the writer sees the request.

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::adaptor::gateway::local_event_store::envelope::EncodedEventPayload;
use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
use crate::domain::local_event::{CommitBatchError, CommitBatchResult, LocalAtomicBatch, StreamId};

pub const NORMAL_LANE_MAX_REQUESTS: usize = 1024;
pub const NORMAL_LANE_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const CRITICAL_LANE_MAX_REQUESTS: usize = 128;
pub const CRITICAL_LANE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BATCH_EVENTS: usize = 4096;
pub const MAX_BATCH_STATE_MUTATIONS: usize = 8192;
pub const MAX_BATCH_DECODED_BYTES: usize = 16 * 1024 * 1024;

/// One event already encoded through the codec registry, ready for the
/// envelope columns.
#[derive(Debug, Clone)]
pub struct PreparedEvent {
    pub stream_id: StreamId,
    pub payload: EncodedEventPayload,
    pub payload_sha256: [u8; 32],
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedNodeEvent {
    pub(crate) row: NewNodeEventRow,
    pub(crate) timestamp_ms: i64,
    pub(crate) expect_tree_absent: bool,
}

/// A batch validated and encoded before queue admission.
pub struct PreparedBatch {
    pub batch: LocalAtomicBatch,
    pub events: Vec<PreparedEvent>,
    pub(crate) node_events: Vec<PreparedNodeEvent>,
    pub decoded_bytes: usize,
    pub critical: bool,
}

pub struct CommitWriteRequest {
    pub prepared: PreparedBatch,
    pub reply: oneshot::Sender<Result<CommitBatchResult, CommitBatchError>>,
}

/// Append one fact row to `node_events`. Never atomic with anything else.
pub struct NodeEventAppendRequest {
    pub row: NewNodeEventRow,
    /// 事実の発生時刻。None なら store の clock で刻む。
    pub timestamp_ms: Option<i64>,
    pub reply: mpsc::SyncSender<Result<i64, NodeEventWriteError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NodeEventWriteError {
    #[error("node event storage is unavailable")]
    StorageUnavailable,
    /// Admission failed or the reply was lost: the write may or may not be
    /// durable. Callers re-derive from the log and retry idempotently.
    #[error("node event write outcome is unknown")]
    OutcomeUnknown,
}

pub enum WriteRequest {
    Commit(CommitWriteRequest),
    NodeEventAppend(NodeEventAppendRequest),
}

impl WriteRequest {
    fn decoded_bytes(&self) -> usize {
        match self {
            Self::Commit(request) => request.prepared.decoded_bytes,
            Self::NodeEventAppend(request) => request.row.detail.len() + 256,
        }
    }

    fn critical(&self) -> bool {
        match self {
            Self::Commit(request) => request.prepared.critical,
            Self::NodeEventAppend(_) => false,
        }
    }
}

/// Why a request was not admitted into the write queue.
pub enum AdmitRejection {
    /// Lane bounds exceeded (`CapacityExceeded` for the caller).
    Capacity,
    /// The queue is closed / the writer stopped; the caller must treat the
    /// outcome as unknown and resolve by commit identity.
    Closed,
}

pub enum QueuePop {
    Request(Box<WriteRequest>),
    Idle,
    Closed,
}

#[derive(Default)]
struct LaneState {
    queue: VecDeque<WriteRequest>,
    bytes: usize,
}

#[derive(Default)]
struct QueueState {
    normal: LaneState,
    critical: LaneState,
    closed: bool,
}

/// Bounded two-lane queue between async callers and the writer thread.
pub struct WriteQueue {
    state: Mutex<QueueState>,
    available: Condvar,
}

impl WriteQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(QueueState::default()),
            available: Condvar::new(),
        })
    }

    /// Admit a request into its lane. Lane bounds are checked here, before
    /// the writer ever sees the request.
    pub fn admit(&self, request: WriteRequest) -> Result<(), AdmitRejection> {
        let mut state = self.state.lock().expect("write queue poisoned");
        if state.closed {
            drop(request);
            return Err(AdmitRejection::Closed);
        }
        let bytes = request.decoded_bytes();
        let critical = request.critical();
        let lane = if critical {
            &mut state.critical
        } else {
            &mut state.normal
        };
        let (max_requests, max_bytes) = if critical {
            (CRITICAL_LANE_MAX_REQUESTS, CRITICAL_LANE_MAX_BYTES)
        } else {
            (NORMAL_LANE_MAX_REQUESTS, NORMAL_LANE_MAX_BYTES)
        };
        if lane.queue.len() >= max_requests || lane.bytes + bytes > max_bytes {
            drop(request);
            return Err(AdmitRejection::Capacity);
        }
        lane.bytes += bytes;
        lane.queue.push_back(request);
        drop(state);
        self.available.notify_one();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn pending_request_count(&self) -> usize {
        let state = self.state.lock().expect("write queue poisoned");
        state.normal.queue.len() + state.critical.queue.len()
    }

    /// Pop the next request, critical lane first. `None` when closed and
    /// drained.
    #[cfg(test)]
    pub fn pop_blocking(&self) -> Option<WriteRequest> {
        let mut state = self.state.lock().expect("write queue poisoned");
        loop {
            if let Some(request) = state.critical.queue.pop_front() {
                state.critical.bytes -= request.decoded_bytes();
                return Some(request);
            }
            if let Some(request) = state.normal.queue.pop_front() {
                state.normal.bytes -= request.decoded_bytes();
                return Some(request);
            }
            if state.closed {
                return None;
            }
            state = self.available.wait(state).expect("write queue poisoned");
        }
    }

    /// Pop with a bounded idle interval so the sole writer can perform
    /// background physical cleanup without introducing a second SQLite
    /// writer authority.
    pub fn pop_with_timeout(&self, timeout: Duration) -> QueuePop {
        let mut state = self.state.lock().expect("write queue poisoned");
        loop {
            if let Some(request) = state.critical.queue.pop_front() {
                state.critical.bytes -= request.decoded_bytes();
                return QueuePop::Request(Box::new(request));
            }
            if let Some(request) = state.normal.queue.pop_front() {
                state.normal.bytes -= request.decoded_bytes();
                return QueuePop::Request(Box::new(request));
            }
            if state.closed {
                return QueuePop::Closed;
            }
            let (next, wait) = self
                .available
                .wait_timeout(state, timeout)
                .expect("write queue poisoned");
            state = next;
            if wait.timed_out() {
                return QueuePop::Idle;
            }
        }
    }

    /// Close the queue; queued requests are dropped so their callers observe
    /// reply loss (`OutcomeUnknown` semantics decided by the caller).
    pub fn close(&self) {
        let mut state = self.state.lock().expect("write queue poisoned");
        state.closed = true;
        state.normal.queue.clear();
        state.normal.bytes = 0;
        state.critical.queue.clear();
        state.critical.bytes = 0;
        drop(state);
        self.available.notify_all();
    }

    /// Stop admission and let the writer consume every already-admitted
    /// request before it exits.
    pub fn close_after_drain(&self) {
        let mut state = self.state.lock().expect("write queue poisoned");
        state.closed = true;
        drop(state);
        self.available.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local_event::{CommitIdentity, IdempotencyBinding, OperationKind};

    fn request(critical: bool, bytes: usize) -> WriteRequest {
        let (reply, receiver) = oneshot::channel();
        drop(receiver);
        WriteRequest::Commit(CommitWriteRequest {
            prepared: PreparedBatch {
                batch: LocalAtomicBatch {
                    commit_id: CommitIdentity::parse("c-1").unwrap(),
                    idempotency: IdempotencyBinding {
                        installation_id: "g".to_string(),
                        operation_kind: OperationKind::ApplicationQuit.into(),
                        idempotency_key: "k".to_string(),
                        payload_hash: [0; 32],
                    },
                    expected_heads: vec![],
                    events: vec![],
                    state_mutations: vec![],
                },
                events: vec![],
                node_events: vec![],
                decoded_bytes: bytes,
                critical,
            },
            reply,
        })
    }

    #[test]
    fn critical_lane_is_popped_first() {
        let queue = WriteQueue::new();
        assert!(queue.admit(request(false, 1)).is_ok());
        assert!(queue.admit(request(true, 1)).is_ok());
        assert!(queue.pop_blocking().unwrap().critical());
        assert!(!queue.pop_blocking().unwrap().critical());
    }

    #[test]
    fn lane_request_bounds_are_enforced() {
        let queue = WriteQueue::new();
        for _ in 0..CRITICAL_LANE_MAX_REQUESTS {
            assert!(queue.admit(request(true, 1)).is_ok());
        }
        assert!(queue.admit(request(true, 1)).is_err());
        // The normal lane still admits.
        assert!(queue.admit(request(false, 1)).is_ok());
    }

    #[test]
    fn lane_byte_bounds_are_enforced() {
        let queue = WriteQueue::new();
        assert!(queue
            .admit(request(false, NORMAL_LANE_MAX_BYTES - 1))
            .is_ok());
        assert!(queue.admit(request(false, 2)).is_err());
        assert!(queue.admit(request(false, 1)).is_ok());
    }

    #[test]
    fn close_after_drain_preserves_admitted_requests() {
        let queue = WriteQueue::new();
        assert!(queue.admit(request(false, 1)).is_ok());
        assert!(queue.admit(request(true, 1)).is_ok());

        queue.close_after_drain();

        assert!(queue.pop_blocking().unwrap().critical());
        assert!(!queue.pop_blocking().unwrap().critical());
        assert!(queue.pop_blocking().is_none());
        assert!(matches!(
            queue.admit(request(false, 1)),
            Err(AdmitRejection::Closed)
        ));
    }
}
