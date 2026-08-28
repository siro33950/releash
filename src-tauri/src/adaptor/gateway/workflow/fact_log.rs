//! 統一 Node 事実ログ（node_events）への gateway。
//!
//! 書き込み: エンジンが発するイベント列から純粋事実のみを行へ写像して
//! 単一行 append する。遷移イベント（NodeCompleted / ExecutionCompleted /
//! ApprovalRequested 等）と合成子の導出成果はここで捨てられ、永続化されない。
//! 読み出し: tree 単位の行列を [`NodeFactRecord`] へ復元し、状態導出は
//! domain の fold（`fact_replay`）に委ねる。

use std::collections::HashMap;
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::node_events::{
    self, NewNodeEventRow, NodeEventRow,
};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::workflow::{
    ApprovalGrantedFact, ArtifactProducedFact, CommandSpawnedFact, ExecutionTreeLaunch, NodeFact,
    NodeFactMeta, NodeFactRecord, NodeKindName, ProcessExitedFact, RuntimeFailureObservedFact,
    SessionAttachedFact, StartedFact, StopReceivedFact, SubmitReceivedFact, SubmitRejectedFact,
    TreeRootFact, WorkflowEvent,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

const MAX_RECONCILIATION_ADVANCE_ROUNDS: usize = 4_096;

#[cfg(test)]
#[path = "fact_log_test.rs"]
mod fact_log_test;

fn kind_column(kind: NodeKindName) -> &'static str {
    match kind {
        NodeKindName::Session => "session",
        NodeKindName::Command => "command",
        NodeKindName::Fanout => "fanout",
        NodeKindName::Sequence => "sequence",
    }
}

fn kind_from_column(value: &str) -> Result<NodeKindName, String> {
    match value {
        "session" => Ok(NodeKindName::Session),
        "command" => Ok(NodeKindName::Command),
        "fanout" => Ok(NodeKindName::Fanout),
        "sequence" => Ok(NodeKindName::Sequence),
        other => Err(format!("unknown node kind column value: {other}")),
    }
}

/// 追記待ちの1事実行（時刻つき）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingFactRow {
    pub(crate) row: NewNodeEventRow,
    pub(crate) timestamp_ms: i64,
}

fn pending_row(
    meta: &FactRowMeta,
    tree_id: &str,
    fact: &NodeFact,
    timestamp: f64,
) -> Result<PendingFactRow, String> {
    let session_id = match fact {
        NodeFact::SessionAttached(fact) => Some(fact.session_id.clone()),
        _ => None,
    };
    let detail = fact
        .encode_detail()
        .map_err(|error| format!("node fact encode failed: {error}"))?;
    Ok(PendingFactRow {
        row: NewNodeEventRow {
            tree_id: tree_id.to_string(),
            node_execution_id: meta.node_execution_id.clone(),
            parent_id: meta.parent_id.clone(),
            node_name: meta.node_name.clone(),
            kind: kind_column(meta.kind).to_string(),
            attempt: i64::from(meta.attempt),
            event_type: fact.event_type().to_string(),
            session_id,
            detail,
        },
        timestamp_ms: (timestamp * 1000.0) as i64,
    })
}

/// 行の同定カラム（イベントが運ばない分は既存行から補完する）。
#[derive(Debug, Clone)]
struct FactRowMeta {
    node_execution_id: String,
    parent_id: Option<String>,
    node_name: String,
    kind: NodeKindName,
    attempt: u32,
}

/// イベント列から事実行への写像。
///
/// - 事実でないイベント（遷移・観測の導出・合成子の導出成果）は行にしない。
/// - command / session の NodeCompleted / NodeFailed は「プロセスの終了」という
///   事実（process_exited）として写像する。
/// - meta をイベントが運ばない事実は、同一バッチ内の started か、既存の
///   node_events 行（`lookup`）から同定カラムを補完する。
fn fact_rows_for_events(
    events: &[WorkflowEvent],
    mut lookup: impl FnMut(&str) -> Result<Option<FactRowMeta>, String>,
    mut root_lookup: impl FnMut(&str) -> Result<Option<FactRowMeta>, String>,
) -> Result<Vec<PendingFactRow>, String> {
    let mut rows: Vec<PendingFactRow> = Vec::new();
    let mut batch_meta: HashMap<String, FactRowMeta> = HashMap::new();
    let mut pending_root: Option<TreeRootFact> = None;

    let mut resolve = |batch_meta: &HashMap<String, FactRowMeta>,
                       node_execution_id: &str|
     -> Result<FactRowMeta, String> {
        if let Some(meta) = batch_meta.get(node_execution_id) {
            return Ok(meta.clone());
        }
        lookup(node_execution_id)?.ok_or_else(|| {
            format!("node fact references unknown node_execution_id {node_execution_id}")
        })
    };

    for event in events {
        let tree_id = event.execution_id();
        let timestamp = event.timestamp();
        match event {
            WorkflowEvent::ExecutionStarted {
                worktree_path,
                created_from,
                request,
                definition,
                ..
            } => {
                pending_root = Some(TreeRootFact {
                    workspace_identity: WorkspaceIdentity::new(worktree_path).as_str().to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: *created_from,
                    request: request.clone(),
                    definition: definition.clone(),
                    launched_as: ExecutionTreeLaunch::Workflow,
                });
            }
            WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                kind,
                attempt,
                parent,
                ..
            } => {
                let meta = FactRowMeta {
                    node_execution_id: node_execution_id.clone(),
                    parent_id: parent.as_ref().map(|parent| parent.parent_id.clone()),
                    node_name: node_name.clone(),
                    kind: *kind,
                    attempt: *attempt,
                };
                let root = if parent.is_none() {
                    pending_root.take()
                } else {
                    None
                };
                let fact = NodeFact::Started(StartedFact {
                    parent: parent.clone(),
                    root,
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
                batch_meta.insert(node_execution_id.clone(), meta);
            }
            WorkflowEvent::SessionAttached {
                node_execution_id,
                session_id,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                // engine 経由の attach は spawn 時に node の初期指示を配送する
                // （prepare が常に指示を渡す）ため、配送済みとして記録する。
                let fact = NodeFact::SessionAttached(SessionAttachedFact {
                    session_id: session_id.clone(),
                    provider_session_id: None,
                    transcript_ref: None,
                    initial_instruction_admitted: true,
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::NodeSubmitReceived {
                node_execution_id, ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::NodeStopReceived {
                node_execution_id, ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::NodeRetryRequested {
                node_execution_id, ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                rows.push(pending_row(
                    &meta,
                    tree_id,
                    &NodeFact::RetryRequested,
                    timestamp,
                )?);
            }
            WorkflowEvent::NodeResumed {
                node_execution_id, ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                rows.push(pending_row(
                    &meta,
                    tree_id,
                    &NodeFact::ResumeRequested,
                    timestamp,
                )?);
            }
            WorkflowEvent::NodeProcessExitObserved {
                node_execution_id,
                exit_code,
                failure_reason,
                failure_kind,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                rows.push(pending_row(
                    &meta,
                    tree_id,
                    &NodeFact::ProcessExited(ProcessExitedFact {
                        exit_code: *exit_code,
                        result_summary: None,
                        failure_reason: failure_reason.clone(),
                        failure_kind: *failure_kind,
                    }),
                    timestamp,
                )?);
            }
            // Paused は導出（プロセス事実 + 未揃いの完了信号）であり記録しない。
            WorkflowEvent::NodePaused { .. } => {}
            WorkflowEvent::CommandSpawned {
                node_execution_id,
                display_command,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = NodeFact::CommandSpawned(CommandSpawnedFact {
                    display_command: display_command.clone(),
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::ArtifactProduced {
                node_execution_id,
                contract,
                value,
                request_id,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                // 合成子の成果（fanout 集約 / sequence output）は導出であり
                // 記録しない。外部入力の Artifact（leaf への提出物）のみが事実。
                if meta.kind.is_composite_kind() {
                    continue;
                }
                let fact = NodeFact::ArtifactProduced(ArtifactProducedFact {
                    contract: contract.clone(),
                    value: value.clone(),
                    request_id: request_id.clone(),
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                result_summary,
                token_usage,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                // command の完了はプロセス終了の事実。session / 合成子の完了は
                // 導出（記録しない）。
                if meta.kind == NodeKindName::Command {
                    let fact = NodeFact::ProcessExited(ProcessExitedFact {
                        exit_code: Some(0),
                        result_summary: result_summary.clone(),
                        failure_reason: None,
                        failure_kind: None,
                    });
                    rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
                } else if meta.kind == NodeKindName::Session {
                    // session の完了は導出だが、settle 時の result_summary /
                    // token_usage は事実として stop_received の detail が担う。
                    // 同一バッチ（stop 受理 → 完了決着）の stop 行へ充填する。
                    if let Some(pending) = rows.iter_mut().rev().find(|pending| {
                        pending.row.node_execution_id == *node_execution_id
                            && pending.row.event_type == "stop_received"
                    }) {
                        let mut stop: StopReceivedFact = serde_json::from_str(&pending.row.detail)
                            .map_err(|error| {
                                format!("stop_received detail re-read failed: {error}")
                            })?;
                        if stop.result_summary.is_none() {
                            stop.result_summary = result_summary.clone();
                        }
                        if stop.token_usage.is_none() {
                            stop.token_usage = token_usage.clone();
                        }
                        pending.row.detail = NodeFact::StopReceived(stop)
                            .encode_detail()
                            .map_err(|error| format!("stop_received re-encode failed: {error}"))?;
                    }
                }
            }
            WorkflowEvent::NodeFailed {
                node_execution_id,
                reason,
                failure_kind,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = match meta.kind {
                    NodeKindName::Session => Some(NodeFact::RuntimeFailureObserved(
                        RuntimeFailureObservedFact {
                            reason: reason.clone(),
                            failure_kind: *failure_kind,
                        },
                    )),
                    NodeKindName::Command => Some(NodeFact::ProcessExited(ProcessExitedFact {
                        exit_code: None,
                        result_summary: None,
                        failure_reason: Some(reason.clone()),
                        failure_kind: Some(*failure_kind),
                    })),
                    NodeKindName::Fanout | NodeKindName::Sequence => None,
                };
                if let Some(fact) = fact {
                    rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
                }
            }
            WorkflowEvent::ApprovalResolved {
                node_execution_id,
                comment,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = NodeFact::ApprovalGranted(ApprovalGrantedFact {
                    comment: comment.clone(),
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::ContractViolated {
                node_execution_id,
                violations,
                repair_attempt,
                request_id,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                let fact = NodeFact::SubmitRejected(SubmitRejectedFact {
                    violations: violations.clone(),
                    repair_attempt: *repair_attempt,
                    request_id: request_id.clone(),
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            WorkflowEvent::ExecutionAborted { .. } => {
                let meta = root_lookup(tree_id)?.ok_or_else(|| {
                    format!("abort references tree {tree_id} without a root started fact")
                })?;
                rows.push(pending_row(
                    &meta,
                    tree_id,
                    &NodeFact::AbortRequested,
                    timestamp,
                )?);
            }
            // 遷移・観測の導出はログに書かない。
            WorkflowEvent::ApprovalRequested { .. }
            | WorkflowEvent::ExecutionCompleted { .. }
            | WorkflowEvent::StallObserved { .. }
            | WorkflowEvent::StallCleared { .. } => {}
            WorkflowEvent::ExecutionInterrupted { .. } => {
                log::warn!(
                    "workflow event ExecutionInterrupted for {} is not representable as a node fact and was dropped",
                    event.execution_id()
                );
            }
            WorkflowEvent::ExecutionResumed { .. } => {
                log::warn!(
                    "workflow event ExecutionResumed for {} is not representable as a node fact and was dropped",
                    event.execution_id()
                );
            }
        }
    }
    Ok(rows)
}

fn meta_from_row(row: &NodeEventRow) -> Result<FactRowMeta, String> {
    Ok(FactRowMeta {
        node_execution_id: row.node_execution_id.clone(),
        parent_id: row.parent_id.clone(),
        node_name: row.node_name.clone(),
        kind: kind_from_column(&row.kind)?,
        attempt: u32::try_from(row.attempt)
            .map_err(|_| format!("stored attempt {} is invalid", row.attempt))?,
    })
}

/// イベント列を事実行へ写像して node_events に追記する（単一行 append の列）。
pub(crate) fn append_facts_for_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let lookup_store = Arc::clone(store);
    let root_store = Arc::clone(store);
    let rows = fact_rows_for_events(
        events,
        move |node_execution_id| {
            let node_execution_id = node_execution_id.to_string();
            lookup_store
                .submit_indexed_query_blocking(move |connection| {
                    node_events::latest_row_for_node(connection, &node_execution_id)
                        .map_err(|_| LocalEventQueryError::InvalidRequest)
                })
                .map_err(|error| format!("node fact meta lookup failed: {error:?}"))?
                .map(|row| meta_from_row(&row))
                .transpose()
        },
        move |tree_id| {
            let tree_id = tree_id.to_string();
            root_store
                .submit_indexed_query_blocking(move |connection| {
                    node_events::first_row_of_tree(connection, &tree_id)
                        .map_err(|_| LocalEventQueryError::InvalidRequest)
                })
                .map_err(|error| format!("tree root lookup failed: {error:?}"))?
                .map(|row| meta_from_row(&row))
                .transpose()
        },
    )?;
    append_pending_rows_blocking(store, rows)
}

/// 事実行の列を順に append する（それぞれ独立した単一行 append）。
pub(crate) fn append_pending_rows_blocking(
    store: &Arc<LocalEventStore>,
    rows: Vec<PendingFactRow>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    for pending in rows {
        store
            .append_node_event_blocking(pending.row, Some(pending.timestamp_ms))
            .map_err(|error| format!("node fact append failed: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn append_fact_batch_for_seed(
    store: &Arc<LocalEventStore>,
    facts: &[(NodeFactMeta, NodeFact)],
    first_timestamp_ms: i64,
    seed_identity: &str,
) -> Result<(), String> {
    use crate::adaptor::gateway::local_event_store::writer::PreparedNodeEvent;
    use crate::domain::local_event::{
        CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
    };
    use sha2::{Digest, Sha256};

    if facts.is_empty() {
        return Ok(());
    }
    let mut canonical = Vec::new();
    let mut node_events = Vec::with_capacity(facts.len());
    for (index, (meta, fact)) in facts.iter().enumerate() {
        let timestamp_ms = first_timestamp_ms.saturating_add(i64::try_from(index).unwrap_or(0));
        let pending = pending_single_fact(meta, fact, timestamp_ms)?;
        for value in [
            pending.row.tree_id.as_bytes(),
            pending.row.node_execution_id.as_bytes(),
            pending
                .row
                .parent_id
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            pending.row.node_name.as_bytes(),
            pending.row.kind.as_bytes(),
            pending.row.event_type.as_bytes(),
            pending.row.detail.as_bytes(),
        ] {
            canonical.extend_from_slice(&(value.len() as u64).to_be_bytes());
            canonical.extend_from_slice(value);
        }
        canonical.extend_from_slice(&pending.row.attempt.to_be_bytes());
        canonical.extend_from_slice(&timestamp_ms.to_be_bytes());
        node_events.push(PreparedNodeEvent {
            row: pending.row,
            timestamp_ms,
            expect_tree_absent: index == 0,
        });
    }
    let payload_hash: [u8; 32] = Sha256::digest(&canonical).into();
    let commit_digest = Sha256::digest(
        [
            b"node-fact-seed/v1\0".as_slice(),
            seed_identity.as_bytes(),
            b"\0",
            canonical.as_slice(),
        ]
        .concat(),
    );
    let commit_id = CommitIdentity::parse(&hex::encode(commit_digest))
        .map_err(|error| format!("node fact seed commit identity is invalid: {error}"))?;
    let batch = LocalAtomicBatch {
        commit_id,
        idempotency: IdempotencyBinding {
            installation_id: store.installation_id().to_string(),
            operation_kind: CommitOperationKind::UserMutation,
            idempotency_key: format!("node-fact-seed.{}", hex::encode(payload_hash)),
            payload_hash,
        },
        expected_heads: Vec::new(),
        events: Vec::new(),
        state_mutations: Vec::new(),
    };
    let store = Arc::clone(store);
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| format!("failed to create fact seed runtime: {error}"))?;
                runtime
                    .block_on(store.commit_batch_with_node_events(batch, node_events))
                    .map(|_| ())
                    .map_err(|error| format!("node fact seed batch failed: {error}"))
            })
            .join()
            .map_err(|_| "node fact seed worker panicked".to_string())?
    })
}

/// 事実ログの読み出し元。writer プロセスの store と read-only の store の
/// どちらからでも同じ形で読める。
#[derive(Clone)]
pub(crate) enum FactLogReadBackend {
    Live(Arc<LocalEventStore>),
    ReadOnly(Arc<LocalEventReadStore>),
}

impl FactLogReadBackend {
    fn run_indexed<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        match self {
            Self::Live(store) => store.submit_indexed_query_blocking(run),
            Self::ReadOnly(store) => store.submit_indexed_query_blocking(run),
        }
    }

    /// node_execution_id からその node が属する tree_id を引く。
    pub(crate) fn tree_id_for_node(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<String>, String> {
        let requested = node_execution_id.to_string();
        self.run_indexed(move |connection| {
            node_events::latest_row_for_node(connection, &requested)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map(|row| row.map(|row| row.tree_id))
        .map_err(|error| format!("node fact tree lookup failed: {error:?}"))
    }
}

/// 1 tree 分の事実行列を読み出して domain の record へ復元する。
pub(crate) fn read_tree_records_from(
    backend: &FactLogReadBackend,
    tree_id: &str,
) -> Result<Vec<NodeFactRecord>, String> {
    let tree_id_owned = tree_id.to_string();
    let rows = backend
        .run_indexed(move |connection| {
            node_events::read_tree(connection, &tree_id_owned)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node fact tree read failed: {error:?}"))?;
    rows.iter().map(record_from_row).collect()
}

pub(crate) fn read_tree_record_page_from(
    backend: &FactLogReadBackend,
    tree_id: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<NodeFactRecord>, String> {
    let tree_id_owned = tree_id.to_string();
    let rows = backend
        .run_indexed(move |connection| {
            node_events::read_tree_page(connection, &tree_id_owned, offset, limit)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node fact tree page read failed: {error:?}"))?;
    rows.iter().map(record_from_row).collect()
}

pub(crate) fn read_tree_root_from(
    backend: &FactLogReadBackend,
    tree_id: &str,
) -> Result<Option<NodeFactRecord>, String> {
    let tree_id_owned = tree_id.to_string();
    backend
        .run_indexed(move |connection| {
            node_events::first_row_of_tree(connection, &tree_id_owned)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node fact tree root read failed: {error:?}"))?
        .as_ref()
        .map(record_from_row)
        .transpose()
}

pub(crate) fn read_latest_activity_record_for_node(
    backend: &FactLogReadBackend,
    node_execution_id: &str,
) -> Result<Option<NodeFactRecord>, String> {
    let requested = node_execution_id.to_string();
    let event_types = NodeFact::activity_replay_event_types();
    backend
        .run_indexed(move |connection| {
            node_events::latest_row_for_node_with_event_types(connection, &requested, event_types)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node activity fact lookup failed: {error:?}"))?
        .as_ref()
        .map(record_from_row)
        .transpose()
}

/// 1 tree 分の事実行列を読み出して domain の record へ復元する（writer store）。
pub(crate) fn read_tree_records(
    store: &Arc<LocalEventStore>,
    tree_id: &str,
) -> Result<Vec<NodeFactRecord>, String> {
    read_tree_records_from(&FactLogReadBackend::Live(Arc::clone(store)), tree_id)
}

pub(crate) fn record_from_row(row: &NodeEventRow) -> Result<NodeFactRecord, String> {
    let fact = NodeFact::decode(&row.event_type, &row.detail)
        .map_err(|error| format!("node fact decode failed: {error}"))?;
    Ok(NodeFactRecord {
        meta: NodeFactMeta {
            tree_id: row.tree_id.clone(),
            node_execution_id: row.node_execution_id.clone(),
            parent_id: row.parent_id.clone(),
            node_name: row.node_name.clone(),
            kind: kind_from_column(&row.kind)?,
            attempt: u32::try_from(row.attempt)
                .map_err(|_| format!("stored attempt {} is invalid", row.attempt))?,
        },
        seq: row.seq,
        timestamp_ms: row.timestamp_ms,
        fact,
    })
}

/// 単独の事実（human の行動等）を1行 append する。
pub(crate) fn append_single_fact(
    store: &Arc<LocalEventStore>,
    meta: &NodeFactMeta,
    fact: &NodeFact,
    timestamp_ms: i64,
) -> Result<(), String> {
    append_pending_rows_blocking(store, vec![pending_single_fact(meta, fact, timestamp_ms)?])
}

pub(crate) fn pending_single_fact(
    meta: &NodeFactMeta,
    fact: &NodeFact,
    timestamp_ms: i64,
) -> Result<PendingFactRow, String> {
    let detail = fact
        .encode_detail()
        .map_err(|error| format!("node fact encode failed: {error}"))?;
    let row = NewNodeEventRow {
        tree_id: meta.tree_id.clone(),
        node_execution_id: meta.node_execution_id.clone(),
        parent_id: meta.parent_id.clone(),
        node_name: meta.node_name.clone(),
        kind: kind_column(meta.kind).to_string(),
        attempt: i64::from(meta.attempt),
        event_type: fact.event_type().to_string(),
        session_id: match fact {
            NodeFact::SessionAttached(fact) => Some(fact.session_id.clone()),
            _ => None,
        },
        detail,
    };
    Ok(PendingFactRow { row, timestamp_ms })
}

/// 1 tree に対する reconciliation パスの結果。
pub(crate) struct TreeReconciliation {
    pub(crate) folded: crate::domain::workflow::services::fact_replay::FoldedTree,
    /// 前進の実行で新たに起動すべきになった leaf。
    pub(crate) leaves: Vec<crate::domain::workflow::entities::workflow_execution::LeafStart>,
}

pub(crate) struct WorktreeReconciliationPorts<'a> {
    pub(crate) ledger: &'a dyn crate::domain::workflow::IsolatedWorktreeLedgerRepository,
    pub(crate) inventory: &'a [crate::domain::workflow::RepositoryWorktreeInventory],
}

/// 1 tree の冪等 reconciliation パス:
/// 導出された状態を見て、まだ実行していない行動（プロセス喪失の観測・
/// 途切れた前進）を実行し、実行した事実を追記して、再導出した状態を返す。
///
/// 既に事実が揃っている行動は導出の差分に現れないため、同じパスを何度
/// 実行しても新しい行は生まれない（冪等）。
pub(crate) fn reconcile_tree_pass(
    store: &Arc<LocalEventStore>,
    tree_id: &str,
    now: f64,
    new_id: &mut dyn FnMut() -> String,
    worktrees: Option<WorktreeReconciliationPorts<'_>>,
) -> Result<Option<TreeReconciliation>, String> {
    use crate::domain::workflow::entities::workflow_execution::{
        ExecutionAdvanceDecision, RuntimeNodeExecutionStatus,
    };
    use crate::domain::workflow::services::worktree_reconciliation::{
        reconcile_worktrees, IsolatedWorktreeOwnerLifecycle, IsolatedWorktreeOwnerState,
    };
    use crate::domain::workflow::{
        IsolatedWorktreeIdentity, NodeCompletionSignalState, ProcessExitedFact,
    };

    let backend = FactLogReadBackend::Live(Arc::clone(store));
    let Some(mut folded) = fold_tree_from(&backend, tree_id)? else {
        return Ok(None);
    };
    if let Some(worktrees) = worktrees {
        let owner_states = folded
            .aggregate
            .node_executions
            .iter()
            .map(|node| IsolatedWorktreeOwnerState {
                identity: IsolatedWorktreeIdentity {
                    tree_id: tree_id.to_string(),
                    node_execution_id: node.id.clone(),
                    attempt: node.attempt,
                },
                lifecycle: if node.status.is_active() {
                    IsolatedWorktreeOwnerLifecycle::Active
                } else {
                    IsolatedWorktreeOwnerLifecycle::Ended
                },
            })
            .collect::<Vec<_>>();
        for inventory in worktrees.inventory {
            let reconciliation =
                reconcile_worktrees(&folded.isolated_worktrees, &owner_states, inventory);
            for loss in reconciliation.losses {
                worktrees
                    .ledger
                    .append(
                        &loss.entry.owner,
                        &NodeFact::IsolatedWorktreeLost,
                        (now * 1000.0) as i64,
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        let Some(refolded) = fold_tree_from(&backend, tree_id)? else {
            return Ok(None);
        };
        folded = refolded;
    }
    if !folded.aggregate.is_active() {
        return Ok(Some(TreeReconciliation {
            folded,
            leaves: Vec::new(),
        }));
    }
    let records = read_tree_records_from(&backend, tree_id)?;
    let activated = records
        .iter()
        .filter_map(|record| match record.fact {
            NodeFact::SessionAttached(_) | NodeFact::CommandSpawned(_) => {
                Some(record.meta.node_execution_id.as_str())
            }
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let mut pending_leaf_ids = Vec::new();

    // 1) started だけが永続化された leaf は未起動なので再実行対象に戻す。
    //    attach / spawn まで記録された leaf だけを、前プロセスと共に消えた
    //    プロセスの喪失として観測する。
    for node in &folded.aggregate.node_executions {
        let is_leaf = matches!(node.kind, NodeKindName::Session | NodeKindName::Command);
        if !is_leaf
            || node.status != RuntimeNodeExecutionStatus::Running
            || node.completion_signals == NodeCompletionSignalState::StopReceived
        {
            continue;
        }
        let worktree_identity = IsolatedWorktreeIdentity {
            tree_id: tree_id.to_string(),
            node_execution_id: node.id.clone(),
            attempt: node.attempt,
        };
        if folded
            .isolated_worktrees
            .recovery_cause(&worktree_identity)
            .is_some()
        {
            continue;
        }
        if !activated.contains(node.id.as_str()) {
            pending_leaf_ids.push(node.id.clone());
            continue;
        }
        let meta = NodeFactMeta {
            tree_id: tree_id.to_string(),
            node_execution_id: node.id.clone(),
            parent_id: node.parent.as_ref().map(|parent| parent.parent_id.clone()),
            node_name: node.node_name.clone(),
            kind: node.kind,
            attempt: node.attempt,
        };
        append_single_fact(
            store,
            &meta,
            &NodeFact::ProcessExited(ProcessExitedFact {
                exit_code: None,
                result_summary: None,
                failure_reason: Some("process lost across application restart".to_string()),
                failure_kind: None,
            }),
            (now * 1000.0) as i64,
        )?;
    }
    // 2) 喪失を含めて再導出し、未実行の前進を実行して事実を追記する。
    let Some(mut folded) = fold_tree_from(&backend, tree_id)? else {
        return Ok(None);
    };
    let mut leaves = pending_leaf_ids
        .into_iter()
        .map(|node_execution_id| {
            folded
                .aggregate
                .leaf_start_for(&node_execution_id)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut advance_rounds = 0;
    loop {
        let advances = folded.aggregate.derive_pending_advances();
        if advances.is_empty() {
            break;
        }
        if advance_rounds == MAX_RECONCILIATION_ADVANCE_ROUNDS {
            return Err(format!(
                "workflow tree {tree_id} reconciliation exceeded {MAX_RECONCILIATION_ADVANCE_ROUNDS} advance rounds"
            ));
        }
        advance_rounds += 1;
        for advance in advances {
            let applied = folded
                .aggregate
                .apply_pending_advance(&advance, new_id, now)
                .map_err(|error| error.to_string())?;
            append_facts_for_events(store, &applied.events)?;
            if let ExecutionAdvanceDecision::StartLeaves(applied_leaves) = applied.decision {
                leaves.extend(applied_leaves);
            }
        }
    }
    Ok(Some(TreeReconciliation { folded, leaves }))
}

/// worktree に root を植えた木の識別子と root 事実（root started の追記順）。
/// `worktree_path` が None なら全木。
///
/// 絞り込みは detail JSON を Rust で読む（SQL に判定規則を持ち込まない）。
pub(crate) fn list_tree_roots(
    backend: &FactLogReadBackend,
    worktree_path: Option<&str>,
) -> Result<Vec<(String, TreeRootFact)>, String> {
    let rows = backend
        .run_indexed(move |connection| {
            node_events::list_tree_roots(connection, "started")
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node fact root listing failed: {error:?}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut roots = Vec::new();
    for row in rows {
        if !seen.insert(row.tree_id.clone()) {
            continue;
        }
        let record = record_from_row(&row)?;
        let NodeFact::Started(started) = &record.fact else {
            continue;
        };
        let Some(root) = &started.root else {
            continue;
        };
        if worktree_path.is_none_or(|wanted| wanted == root.worktree_path) {
            roots.push((row.tree_id, root.clone()));
        }
    }
    Ok(roots)
}

pub(crate) fn list_tree_ids(
    backend: &FactLogReadBackend,
    worktree_path: Option<&str>,
) -> Result<Vec<String>, String> {
    list_tree_roots(backend, worktree_path)
        .map(|roots| roots.into_iter().map(|(tree_id, _)| tree_id).collect())
}

/// 1 tree の fold（読み出し + 導出）。
pub(crate) fn fold_tree_from(
    backend: &FactLogReadBackend,
    tree_id: &str,
) -> Result<Option<crate::domain::workflow::services::fact_replay::FoldedTree>, String> {
    let records = read_tree_records_from(backend, tree_id)?;
    crate::domain::workflow::services::fact_replay::fold_execution_tree(tree_id, &records)
}

/// fold 済み read model から実行 metadata record を導出する。
pub(crate) fn metadata_record_from_read_model(
    model: &crate::domain::workflow::WorkflowExecution,
) -> crate::domain::local_event::WorkflowExecutionMetadataRecord {
    crate::domain::local_event::WorkflowExecutionMetadataRecord {
        execution_id: model.id.clone(),
        workflow_name: model.workflow_name.clone(),
        status: model.status,
        worktree_path: model.worktree_path.clone(),
        current_node: model.current_node.clone(),
        created_from: model.created_from,
        started_at_bits: model.started_at.to_bits(),
        updated_at_bits: model.updated_at.to_bits(),
        completed_at_bits: model.completed_at.map(f64::to_bits),
        error_reason: model.error_reason.clone(),
        interruption_reason: model.interruption_reason,
        resume_from_node: model.resume_from_node.clone(),
        total_token_usage: model.total_token_usage.clone(),
    }
}

/// session_id → その session が attach された node（tree_id, node_execution_id）。
///
/// event_type の絞り込みだけを SQL で行い、session_id の照合は detail を
/// Rust で読む。
pub(crate) fn find_session_attachment(
    backend: &FactLogReadBackend,
    session_id: &str,
) -> Result<Option<(String, String)>, String> {
    find_session_attachment_record(backend, session_id)
        .map(|record| record.map(|record| (record.meta.tree_id, record.meta.node_execution_id)))
}

pub(crate) fn find_session_attachment_record(
    backend: &FactLogReadBackend,
    session_id: &str,
) -> Result<Option<NodeFactRecord>, String> {
    let session = session_id.to_string();
    let query_session = session.clone();
    let row = backend
        .run_indexed(move |connection| {
            node_events::latest_session_attachment(connection, &query_session)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("session attachment lookup failed: {error:?}"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let record = record_from_row(&row)?;
    let NodeFact::SessionAttached(fact) = &record.fact else {
        return Err("session attachment index points to a non-attachment fact".to_string());
    };
    if fact.session_id != session {
        return Err("session attachment index identity mismatch".to_string());
    }
    Ok(Some(record))
}
