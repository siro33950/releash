//! 統一 Node 事実ログ（node_events）への gateway。
//!
//! 書き込み: エンジンが発するイベント列から純粋事実のみを行へ写像して
//! append する。実行木の完了は同じイベント列の事実と原子的に記録し、
//! Node の承認要求や合成子の導出成果は永続化しない。
//! 読み出し: tree 単位の行列を [`NodeFactRecord`] へ復元し、状態導出は
//! domain の fold（`fact_replay`）に委ねる。

use crate::adaptor::gateway::workflow::fact_codec;
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
    let detail = fact_codec::encode_detail(fact)
        .map_err(|error| format!("node fact encode failed: {error}"))?;
    Ok(PendingFactRow {
        row: NewNodeEventRow {
            tree_id: tree_id.to_string(),
            node_execution_id: meta.node_execution_id.clone(),
            parent_id: meta.parent_id.clone(),
            node_name: meta.node_name.clone(),
            kind: kind_column(meta.kind).to_string(),
            attempt: i64::from(meta.attempt),
            event_type: fact_codec::event_type(fact).to_string(),
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
/// - command の完了は process_exited、runtime が確定した NodeFailed は
///   runtime_failure_observed として写像する。
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
    let mut batch_roots: HashMap<String, FactRowMeta> = HashMap::new();

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
                repository_root,
                worktree_path,
                created_from,
                request,
                definition,
                ..
            } => {
                pending_root = Some(TreeRootFact {
                    repository_root: repository_root.clone(),
                    workspace_identity: WorkspaceIdentity::new(worktree_path).as_str().to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: *created_from,
                    request: request.clone(),
                    workflow_name: definition.name.clone(),
                    definition: Some(definition.clone()),
                    launched_as: ExecutionTreeLaunch::Workflow,
                });
            }
            WorkflowEvent::NodeStarted {
                worktree,
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
                    batch_roots
                        .entry(tree_id.to_string())
                        .or_insert_with(|| meta.clone());
                    pending_root.take().map(Box::new)
                } else {
                    None
                };
                let fact = NodeFact::Started(StartedFact {
                    worktree: worktree.clone(),
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
            WorkflowEvent::DelegateResultInjected {
                node_execution_id,
                child_execution_id,
                ..
            } => {
                let meta = resolve(&batch_meta, node_execution_id)?;
                rows.push(pending_row(
                    &meta,
                    tree_id,
                    &NodeFact::DelegateResultInjected(child_execution_id.clone()),
                    timestamp,
                )?);
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
                // 合成子の成果（fanout 集約 / sequence 統合 map）は導出であり
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
                        failure_kind: None,
                        exit_code: Some(0),
                        result_summary: result_summary.clone(),
                        failure_reason: None,
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
                        pending.row.detail = fact_codec::encode_detail(&NodeFact::StopReceived(
                            stop,
                        ))
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
                let fact = NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                    reason: reason.clone(),
                    failure_kind: *failure_kind,
                });
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
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
            WorkflowEvent::ExecutionCompleted { .. } | WorkflowEvent::ExecutionAborted { .. } => {
                let meta = match batch_roots.get(tree_id) {
                    Some(meta) => meta.clone(),
                    None => root_lookup(tree_id)?.ok_or_else(|| {
                        format!(
                            "terminal fact references tree {tree_id} without a root started fact"
                        )
                    })?,
                };
                let fact = match event {
                    WorkflowEvent::ExecutionCompleted { .. } => NodeFact::ExecutionCompleted,
                    _ => NodeFact::AbortRequested(Default::default()),
                };
                rows.push(pending_row(&meta, tree_id, &fact, timestamp)?);
            }
            // 遷移・観測の導出はログに書かない。
            WorkflowEvent::ApprovalRequested { .. }
            | WorkflowEvent::StallObserved { .. }
            | WorkflowEvent::StallCleared { .. } => {}
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

pub(crate) fn node_meta_from_row(row: &NodeEventRow) -> Result<NodeFactMeta, String> {
    let meta = meta_from_row(row)?;
    Ok(NodeFactMeta {
        tree_id: row.tree_id.clone(),
        node_execution_id: meta.node_execution_id,
        parent_id: meta.parent_id,
        node_name: meta.node_name,
        kind: meta.kind,
        attempt: meta.attempt,
    })
}

/// イベント列を事実行へ写像して node_events に追記する。
pub(crate) fn append_facts_for_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    append_pending_rows_blocking(store, pending_rows_for_events(store, events)?)
}

pub(crate) fn pending_rows_for_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<Vec<PendingFactRow>, String> {
    let lookup_store = Arc::clone(store);
    let root_store = Arc::clone(store);
    fact_rows_for_events(
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
    )
}

/// 完了事実を含む行列は原子的に、それ以外は単一行ずつ append する。
pub(crate) fn append_pending_rows_blocking(
    store: &Arc<LocalEventStore>,
    rows: Vec<PendingFactRow>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    if rows
        .iter()
        .any(|pending| pending.row.event_type == "execution_completed")
    {
        return store
            .append_node_events_blocking(
                rows.into_iter()
                    .map(|pending| (pending.row, Some(pending.timestamp_ms)))
                    .collect(),
            )
            .map(|_| ())
            .map_err(|error| format!("node fact append failed: {error}"));
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
    pub(crate) fn run_indexed<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
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
    records_from_tree_rows(&rows)
}

fn terminal_fact_in_rows(rows: &[NodeEventRow]) -> Result<bool, String> {
    for row in rows {
        if fact_codec::terminal_event_types().contains(&row.event_type.as_str()) {
            return fact_codec::decode(&row.event_type, &row.detail)
                .map(|fact| fact.terminal_state().is_some())
                .map_err(|error| error.to_string());
        }
    }
    Ok(false)
}

pub(crate) fn records_from_tree_rows(rows: &[NodeEventRow]) -> Result<Vec<NodeFactRecord>, String> {
    let terminal = terminal_fact_in_rows(rows)?;
    let mut legacy_worktrees = HashMap::new();
    if terminal {
        for row in rows.iter().take_while(|row| {
            !fact_codec::terminal_event_types().contains(&row.event_type.as_str())
        }) {
            if row.event_type == "isolated_worktree_created" {
                legacy_worktrees.insert(
                    (row.node_execution_id.as_str(), row.attempt),
                    decode_legacy_worktree(&row.detail)?,
                );
            }
        }
    }
    rows.iter()
        .filter_map(|row| {
            if terminal && row.event_type == "started" {
                Some(
                    super::stored_definition::decode_terminal_started(&row.detail).and_then(
                        |mut fact| {
                            if let NodeFact::Started(started) = &mut fact {
                                if let (Some(root), Ok(NodeFact::Started(decoded))) = (
                                    started.root.as_mut(),
                                    super::stored_definition::decode_started(&row.detail),
                                ) {
                                    root.definition = decoded.root.and_then(|root| root.definition);
                                }
                                if started.worktree.is_none() {
                                    started.worktree = legacy_worktrees
                                        .get(&(row.node_execution_id.as_str(), row.attempt))
                                        .cloned();
                                }
                            }
                            Ok(NodeFactRecord {
                                meta: node_meta_from_row(row)?,
                                seq: row.seq,
                                timestamp_ms: row.timestamp_ms,
                                fact,
                            })
                        },
                    ),
                )
            } else {
                record_from_row(row).transpose()
            }
        })
        .collect()
}

pub(crate) fn read_latest_activity_record_for_node(
    backend: &FactLogReadBackend,
    node_execution_id: &str,
) -> Result<Option<NodeFactRecord>, String> {
    let requested = node_execution_id.to_string();
    let event_types = fact_codec::activity_replay_event_types();
    backend
        .run_indexed(move |connection| {
            node_events::latest_row_for_node_with_event_types(connection, &requested, event_types)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node activity fact lookup failed: {error:?}"))?
        .as_ref()
        .map(record_from_row)
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn read_tree_archive_records(
    backend: &FactLogReadBackend,
    tree_id: &str,
) -> Result<Vec<NodeFactRecord>, String> {
    read_tree_archive_records_for(backend, &[tree_id.to_string()])
}

pub(crate) fn read_tree_archive_records_for(
    backend: &FactLogReadBackend,
    tree_ids: &[String],
) -> Result<Vec<NodeFactRecord>, String> {
    let tree_ids = tree_ids.to_vec();
    let rows = backend
        .run_indexed(move |connection| {
            node_events::latest_root_rows_for_trees(
                connection,
                &tree_ids,
                &["archive_requested", "restore_requested"],
            )
            .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("tree archive query failed: {error:?}"))?;
    rows.iter()
        .filter_map(|row| record_from_row(row).transpose())
        .collect()
}

pub(crate) fn read_records_for_event_types(
    backend: &FactLogReadBackend,
    event_types: &[&str],
) -> Result<Vec<NodeFactRecord>, String> {
    let event_types = event_types
        .iter()
        .map(|event_type| (*event_type).to_string())
        .collect::<Vec<_>>();
    let rows = backend
        .run_indexed(move |connection| {
            let event_types = event_types.iter().map(String::as_str).collect::<Vec<_>>();
            node_events::rows_for_event_types(connection, &event_types)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("node lifecycle fact lookup failed: {error:?}"))?;
    rows.iter()
        .filter_map(|row| record_from_row(row).transpose())
        .collect()
}

pub(crate) fn read_latest_record_for_node_with_event_types(
    backend: &FactLogReadBackend,
    node_execution_id: &str,
    event_types: &[&str],
) -> Result<Option<NodeFactRecord>, String> {
    let node_execution_id = node_execution_id.to_string();
    let event_types = event_types
        .iter()
        .map(|event_type| (*event_type).to_string())
        .collect::<Vec<_>>();
    backend
        .run_indexed(move |connection| {
            let event_types = event_types.iter().map(String::as_str).collect::<Vec<_>>();
            node_events::latest_row_for_node_with_event_types(
                connection,
                &node_execution_id,
                &event_types,
            )
            .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("latest node fact lookup failed: {error:?}"))?
        .as_ref()
        .map(record_from_row)
        .transpose()
        .map(Option::flatten)
}

/// 1 tree 分の事実行列を読み出して domain の record へ復元する（writer store）。
pub(crate) fn read_tree_records(
    store: &Arc<LocalEventStore>,
    tree_id: &str,
) -> Result<Vec<NodeFactRecord>, String> {
    read_tree_records_from(&FactLogReadBackend::Live(Arc::clone(store)), tree_id)
}

fn decode_legacy_worktree(
    detail: &str,
) -> Result<crate::domain::workflow::IsolatedWorktree, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RetiredWorktreeCreated {
        #[serde(rename = "repositoryRoot")]
        _repository_root: String,
        worktree_path: String,
        branch: String,
    }
    serde_json::from_str::<RetiredWorktreeCreated>(detail)
        .map(|worktree| crate::domain::workflow::IsolatedWorktree {
            path: worktree.worktree_path,
            branch: worktree.branch,
        })
        .map_err(|error| format!("node fact decode failed: {error}"))
}

pub(crate) fn decode_stored_fact(
    event_type: &str,
    detail: &str,
    timestamp_ms: i64,
) -> Result<Option<NodeFact>, String> {
    match event_type {
        "isolated_worktree_created" => decode_legacy_worktree(detail).map(|_| None),
        "isolated_worktree_released" | "isolated_worktree_lost" => {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(detail)
                .map(|_| None)
                .map_err(|error| error.to_string())
        }
        "archive_requested" => {
            let mut detail: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(detail).map_err(|error| error.to_string())?;
            if detail
                .get("archivedAt")
                .is_none_or(serde_json::Value::is_null)
            {
                detail.insert(
                    "archivedAt".into(),
                    serde_json::json!(timestamp_ms as f64 / 1000.0),
                );
            }
            fact_codec::decode(event_type, &serde_json::Value::Object(detail).to_string())
                .map(Some)
                .map_err(|error| error.to_string())
        }
        "started" => super::stored_definition::decode_started(detail).map(Some),
        _ => fact_codec::decode(event_type, detail)
            .map(Some)
            .map_err(|error| error.to_string()),
    }
    .map_err(|error| format!("node fact decode failed: {error}"))
}

pub(crate) fn record_from_row(row: &NodeEventRow) -> Result<Option<NodeFactRecord>, String> {
    let kind = kind_from_column(&row.kind)?;
    let attempt = u32::try_from(row.attempt)
        .map_err(|_| format!("stored attempt {} is invalid", row.attempt))?;
    let Some(fact) = decode_stored_fact(&row.event_type, &row.detail, row.timestamp_ms)? else {
        return Ok(None);
    };
    Ok(Some(NodeFactRecord {
        meta: NodeFactMeta {
            tree_id: row.tree_id.clone(),
            node_execution_id: row.node_execution_id.clone(),
            parent_id: row.parent_id.clone(),
            node_name: row.node_name.clone(),
            kind,
            attempt,
        },
        seq: row.seq,
        timestamp_ms: row.timestamp_ms,
        fact,
    }))
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
    let detail = fact_codec::encode_detail(fact)
        .map_err(|error| format!("node fact encode failed: {error}"))?;
    let row = NewNodeEventRow {
        tree_id: meta.tree_id.clone(),
        node_execution_id: meta.node_execution_id.clone(),
        parent_id: meta.parent_id.clone(),
        node_name: meta.node_name.clone(),
        kind: kind_column(meta.kind).to_string(),
        attempt: i64::from(meta.attempt),
        event_type: fact_codec::event_type(fact).to_string(),
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
    /// 前進の実行で必要になった合成子の準備と葉 runtime の起動。
    pub(crate) starts: Vec<crate::domain::workflow::entities::workflow_execution::NodeStart>,
}

/// 1 tree の冪等 reconciliation パス:
/// 導出された状態を見て途切れた前進を実行し、その事実を追記する。
///
/// 既に事実が揃っている行動は導出の差分に現れないため、同じパスを何度
/// 実行しても新しい行は生まれない（冪等）。
pub(crate) fn reconcile_tree_pass(
    store: &Arc<LocalEventStore>,
    tree_id: &str,
    now: f64,
    new_id: &mut dyn FnMut() -> String,
) -> Result<Option<TreeReconciliation>, crate::domain::workflow::WorkflowError> {
    use crate::adaptor::gateway::local_event_store::writer::NodeEventWriteError;
    use crate::domain::workflow::entities::workflow_execution::{
        ExecutionAdvanceDecision, RuntimeNodeExecutionStatus,
    };
    use crate::domain::workflow::{NodeCompletionSignalState, WorkflowError};

    let backend = FactLogReadBackend::Live(Arc::clone(store));
    let records = read_tree_records_from(&backend, tree_id).map_err(WorkflowError::external)?;
    let Some(folded) =
        crate::domain::workflow::services::fact_replay::fold_execution_tree(tree_id, &records)
            .map_err(WorkflowError::external)?
    else {
        return Ok(None);
    };
    if !folded.aggregate.is_active() {
        return Ok(Some(TreeReconciliation {
            folded,
            starts: Vec::new(),
        }));
    }
    let activated = records
        .iter()
        .filter_map(|record| match record.fact {
            NodeFact::SessionAttached(_) | NodeFact::CommandSpawned(_) => {
                Some(record.meta.node_execution_id.as_str())
            }
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let failed = records
        .iter()
        .filter(|record| matches!(record.fact, NodeFact::RuntimeFailureObserved(_)))
        .map(|record| record.meta.node_execution_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut exited = std::collections::HashSet::new();
    for record in &records {
        match &record.fact {
            NodeFact::ProcessExited(_) => {
                exited.insert(record.meta.node_execution_id.as_str());
            }
            NodeFact::SessionAttached(_)
            | NodeFact::CommandSpawned(_)
            | NodeFact::ResumeRequested => {
                exited.remove(record.meta.node_execution_id.as_str());
            }
            _ => {}
        }
    }
    let mut pending_leaf_ids = Vec::new();

    for node in &folded.aggregate.node_executions {
        if matches!(node.kind, NodeKindName::Session | NodeKindName::Command)
            && node.status == RuntimeNodeExecutionStatus::Running
            && node.completion_signals != NodeCompletionSignalState::StopReceived
            && !exited.contains(node.id.as_str())
            && !activated.contains(node.id.as_str())
            && !failed.contains(node.id.as_str())
        {
            pending_leaf_ids.push(node.id.clone());
        }
    }
    let mut head = records.last().map_or(0, |record| record.seq);
    let mut folded = folded;
    let mut leaves = pending_leaf_ids
        .into_iter()
        .map(|node_execution_id| {
            folded
                .aggregate
                .leaf_start_for(&node_execution_id)
                .map(crate::domain::workflow::entities::workflow_execution::NodeStart::Leaf)
                .map_err(|error| WorkflowError::external(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut advance_rounds = 0;
    loop {
        let advances = folded.aggregate.derive_pending_advances().into_iter().filter(|advance| {
            let scope_id = match advance {
                crate::domain::workflow::entities::workflow_execution::PendingAdvance::Delegate { node_execution_id } => node_execution_id,
                crate::domain::workflow::entities::workflow_execution::PendingAdvance::StartEntry { scope_id }
                | crate::domain::workflow::entities::workflow_execution::PendingAdvance::ExpandFanout { scope_id }
                | crate::domain::workflow::entities::workflow_execution::PendingAdvance::AfterChild { scope_id, .. } => scope_id,
            };
            !leaves.iter().any(|leaf| leaf.node_execution_id() == scope_id)
        }).collect::<Vec<_>>();
        if advances.is_empty() {
            break;
        }
        if advance_rounds == MAX_RECONCILIATION_ADVANCE_ROUNDS {
            return Err(WorkflowError::external(format!(
                "workflow tree {tree_id} reconciliation exceeded {MAX_RECONCILIATION_ADVANCE_ROUNDS} advance rounds"
            )));
        }
        advance_rounds += 1;
        for advance in advances {
            let scope_id = match &advance {
                crate::domain::workflow::entities::workflow_execution::PendingAdvance::Delegate { node_execution_id } => node_execution_id,
                crate::domain::workflow::entities::workflow_execution::PendingAdvance::StartEntry { scope_id }
                | crate::domain::workflow::entities::workflow_execution::PendingAdvance::ExpandFanout { scope_id }
                | crate::domain::workflow::entities::workflow_execution::PendingAdvance::AfterChild { scope_id, .. } => scope_id,
            };
            if let Some(start) = folded.aggregate.isolated_composite_start(scope_id) {
                leaves.push(crate::domain::workflow::entities::workflow_execution::NodeStart::PrepareComposite(start));
                continue;
            }
            let applied = folded
                .aggregate
                .apply_pending_advance(&advance, new_id, now)
                .map_err(|error| WorkflowError::external(error.to_string()))?;
            let rows =
                pending_rows_for_events(store, &applied.events).map_err(WorkflowError::external)?;
            let sequences = match store.append_node_events_at_head_blocking(
                rows.iter()
                    .map(|row| (row.row.clone(), Some(row.timestamp_ms)))
                    .collect(),
                Some((tree_id.into(), head)),
            ) {
                Err(error @ NodeEventWriteError::OutcomeUnknown) => {
                    let requested = tree_id.to_string();
                    let count = rows.len();
                    let stored = backend
                        .run_indexed(move |connection| {
                            node_events::read_tree_page(
                                connection,
                                &requested,
                                head as usize,
                                count,
                            )
                            .map_err(|_| LocalEventQueryError::InvalidRequest)
                        })
                        .map_err(|error| {
                            WorkflowError::external(format!(
                                "startup advancement readback failed: {error:?}"
                            ))
                        })?;
                    let persisted = stored.len() == rows.len()
                        && stored.iter().zip(&rows).all(|(stored, pending)| {
                            stored.tree_id == pending.row.tree_id
                                && stored.node_execution_id == pending.row.node_execution_id
                                && stored.parent_id == pending.row.parent_id
                                && stored.node_name == pending.row.node_name
                                && stored.kind == pending.row.kind
                                && stored.attempt == pending.row.attempt
                                && stored.event_type == pending.row.event_type
                                && stored.session_id == pending.row.session_id
                                && stored.detail == pending.row.detail
                                && stored.timestamp_ms == pending.timestamp_ms.max(0)
                        });
                    if persisted {
                        Ok(stored.iter().map(|row| row.seq).collect())
                    } else if stored.is_empty() {
                        Err(error)
                    } else {
                        Err(NodeEventWriteError::Conflict)
                    }
                }
                result => result,
            }
            .map_err(|error| match error {
                NodeEventWriteError::Conflict => WorkflowError::Conflict(format!(
                    "workflow tree {tree_id} changed before startup advancement commit"
                )),
                error => {
                    WorkflowError::external(format!("startup advancement commit failed: {error}"))
                }
            })?;
            head = sequences.last().copied().unwrap_or(head);
            if let ExecutionAdvanceDecision::StartNodes(applied_leaves) = applied.decision {
                leaves.extend(applied_leaves);
            }
        }
    }
    Ok(Some(TreeReconciliation {
        folded,
        starts: leaves,
    }))
}

/// worktree に root を植えた木の識別子と root 事実（root started の追記順）。
/// `worktree_path` が None なら全木。
///
/// 絞り込みは detail JSON を Rust で読む（SQL に判定規則を持ち込まない）。
pub(crate) fn list_tree_roots(
    backend: &FactLogReadBackend,
    worktree_path: Option<&str>,
) -> Result<Vec<(String, super::stored_definition::TreeRootHeader)>, String> {
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
        let Some(root) = super::stored_definition::read_tree_header(&row.detail)? else {
            continue;
        };
        if worktree_path.is_none_or(|wanted| wanted == root.worktree_path) {
            roots.push((row.tree_id, root));
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
    model: &crate::domain::workflow::ExecutionTree,
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
    let record = record_from_row(&row)?
        .ok_or_else(|| "session attachment index points to a retired fact".to_string())?;
    let NodeFact::SessionAttached(fact) = &record.fact else {
        return Err("session attachment index points to a non-attachment fact".to_string());
    };
    if fact.session_id != session {
        return Err("session attachment index identity mismatch".to_string());
    }
    Ok(Some(record))
}
