//! Execution Store: workflow execution metadata の active/completed 管理を担う。
//!
//! 役割:
//! - active な execution を `execution_id` キーの in-memory map で管理し、worktree_path → execution_id の
//!   secondary index を提供する。
//! - production は SQLite projection/obligation を authority とし、旧
//!   `workflow_executions/*.json` は判断を反転しない derived view とする。
//! - 状態遷移ロジックは持たず、engine からの「開始通知」「終了通知」を受けて反映するのみ。

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::fs::OpenOptions;
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::domain::local_event::{
    LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    ObligationMutation, ObligationRecord, PendingIndexEntry, PendingPartition, QueryCursor,
    Revision, RevisionGuard, SessionProjectionMutation, SessionProjectionRecord,
    WorkflowExecutionMetadataRecord, WorkflowExecutionProjectionRecord,
    WorkflowWorktreeOwnerRecord,
};
use crate::domain::workflow::{
    ExecutionInterruptionReason, TokenUsage, WorkflowExecution as DomainWorkflowExecution,
};
pub(crate) use crate::domain::workflow::{
    ExecutionListFilter, ExecutionOrigin, ExecutionStatus, ExecutionStatusFilter,
    WorkflowExecutionSummary,
};

/// `complete_execution` への入力を terminal status のみに制約する型。
///
/// Spec issues-1011 finding 12: release build でも非 terminal status を `complete_execution` に
/// 渡せないように型レベルで強制する。`From` で `ExecutionStatus` への変換を提供する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExecutionStatus {
    Completed,
    Failed,
    Aborted,
}

impl From<TerminalExecutionStatus> for ExecutionStatus {
    fn from(t: TerminalExecutionStatus) -> Self {
        match t {
            TerminalExecutionStatus::Completed => ExecutionStatus::Completed,
            TerminalExecutionStatus::Failed => ExecutionStatus::Failed,
            TerminalExecutionStatus::Aborted => ExecutionStatus::Aborted,
        }
    }
}

/// 1 回の workflow 実行インスタンス。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionMetadata {
    pub execution_id: String,
    pub workflow_name: String,
    #[serde(with = "execution_status_serde")]
    pub status: ExecutionStatus,
    pub worktree_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    #[serde(with = "execution_origin_serde")]
    pub created_from: ExecutionOrigin,
    pub started_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_interruption_reason_serde"
    )]
    pub interruption_reason: Option<ExecutionInterruptionReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_node: Option<String>,
    #[serde(default, with = "token_usage_serde")]
    pub total_token_usage: TokenUsage,
}

fn valid_event_reconciliation_transition(
    current: &WorkflowExecutionMetadata,
    projected: &WorkflowExecutionMetadata,
) -> bool {
    let immutable_fields_match = projected.execution_id == current.execution_id
        && projected.workflow_name == current.workflow_name
        && projected.worktree_path == current.worktree_path
        && projected.created_from == current.created_from
        && projected.started_at == current.started_at;
    let time_and_usage_are_monotonic = projected.updated_at.is_finite()
        && projected.updated_at >= current.updated_at
        && projected.total_token_usage.input_tokens >= current.total_token_usage.input_tokens
        && projected.total_token_usage.output_tokens >= current.total_token_usage.output_tokens;
    let source_can_reconcile = current.status.is_active() || current.status.is_resumable();
    let target_is_closed_checkpoint = match projected.status {
        ExecutionStatus::Interrupted => {
            projected.completed_at.is_none()
                && projected.error_reason.is_none()
                && projected.interruption_reason.is_some()
                && projected
                    .resume_from_node
                    .as_deref()
                    .is_some_and(|node| !node.is_empty())
        }
        ExecutionStatus::Completed => {
            projected.completed_at.is_some()
                && projected.error_reason.is_none()
                && projected.interruption_reason.is_none()
                && projected.resume_from_node.is_none()
        }
        ExecutionStatus::Failed => {
            projected.completed_at.is_some()
                && projected.error_reason.is_some()
                && projected.interruption_reason.is_none()
                && projected.resume_from_node.is_none()
        }
        ExecutionStatus::Aborted => {
            projected.completed_at.is_some()
                && projected.interruption_reason.is_none()
                && projected.resume_from_node.is_none()
        }
        ExecutionStatus::Running | ExecutionStatus::WaitingApproval => false,
    };
    immutable_fields_match
        && time_and_usage_are_monotonic
        && source_can_reconcile
        && target_is_closed_checkpoint
}

/// `Interrupted` metadata を Running reservation に戻した際の rollback token。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResumeExecutionReservation {
    pub(crate) interrupted: WorkflowExecutionMetadata,
    pub(crate) running: WorkflowExecutionMetadata,
}

/// Interrupted → Aborted transition の event append 前 in-memory reservation。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AbortInterruptedReservation {
    pub(crate) interrupted: WorkflowExecutionMetadata,
    pub(crate) aborted: WorkflowExecutionMetadata,
}

/// Running / WaitingApproval → Interrupted の event commit と runtime cleanup を直列化する token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveInterruptionReservation {
    pub(crate) execution_id: String,
    pub(crate) worktree_path: String,
}

impl From<&WorkflowExecutionMetadata> for WorkflowExecutionSummary {
    fn from(execution: &WorkflowExecutionMetadata) -> Self {
        Self {
            execution_id: execution.execution_id.clone(),
            workflow_name: execution.workflow_name.clone(),
            status: execution.status,
            worktree_path: execution.worktree_path.clone(),
            current_node: execution.current_node.clone(),
            created_from: execution.created_from,
            started_at: execution.started_at,
            updated_at: execution.updated_at,
            completed_at: execution.completed_at,
            error_reason: execution.error_reason.clone(),
            interruption_reason: execution.interruption_reason,
            resume_from_node: execution.resume_from_node.clone(),
            total_token_usage: execution.total_token_usage.clone(),
        }
    }
}

pub(crate) fn workflow_execution_record(
    execution: &WorkflowExecutionMetadata,
) -> WorkflowExecutionMetadataRecord {
    WorkflowExecutionMetadataRecord {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        worktree_path: execution.worktree_path.clone(),
        current_node: execution.current_node.clone(),
        created_from: execution.created_from,
        started_at_bits: execution.started_at.to_bits(),
        updated_at_bits: execution.updated_at.to_bits(),
        completed_at_bits: execution.completed_at.map(f64::to_bits),
        error_reason: execution.error_reason.clone(),
        interruption_reason: execution.interruption_reason,
        resume_from_node: execution.resume_from_node.clone(),
        total_token_usage: execution.total_token_usage.clone(),
    }
}

pub(crate) fn workflow_execution_metadata(
    execution: &WorkflowExecutionMetadataRecord,
) -> WorkflowExecutionMetadata {
    WorkflowExecutionMetadata {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        worktree_path: execution.worktree_path.clone(),
        current_node: execution.current_node.clone(),
        created_from: execution.created_from,
        started_at: f64::from_bits(execution.started_at_bits),
        updated_at: f64::from_bits(execution.updated_at_bits),
        completed_at: execution.completed_at_bits.map(f64::from_bits),
        error_reason: execution.error_reason.clone(),
        interruption_reason: execution.interruption_reason,
        resume_from_node: execution.resume_from_node.clone(),
        total_token_usage: execution.total_token_usage.clone(),
    }
}

impl From<&DomainWorkflowExecution> for WorkflowExecutionMetadata {
    fn from(execution: &DomainWorkflowExecution) -> Self {
        Self {
            execution_id: execution.id.clone(),
            workflow_name: execution.workflow_name.clone(),
            status: execution.status,
            worktree_path: execution.worktree_path.clone(),
            current_node: execution.current_node.clone(),
            created_from: execution.created_from,
            started_at: execution.started_at,
            updated_at: execution.updated_at,
            completed_at: execution.completed_at,
            error_reason: execution.error_reason.clone(),
            interruption_reason: execution.interruption_reason,
            resume_from_node: execution.resume_from_node.clone(),
            total_token_usage: execution.total_token_usage.clone(),
        }
    }
}

mod optional_interruption_reason_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::domain::workflow::ExecutionInterruptionReason;

    pub(super) fn serialize<S>(
        value: &Option<ExecutionInterruptionReason>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(reason) => serializer.serialize_some(reason.as_str()),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Option<ExecutionInterruptionReason>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        value
            .map(|value| match value.as_str() {
                "crash" => Ok(ExecutionInterruptionReason::Crash),
                "stale" => Ok(ExecutionInterruptionReason::Stale),
                "stop" => Ok(ExecutionInterruptionReason::Stop),
                "orphan" => Ok(ExecutionInterruptionReason::Orphan),
                _ => Err(serde::de::Error::unknown_variant(
                    &value,
                    &["crash", "stale", "stop", "orphan"],
                )),
            })
            .transpose()
    }
}

mod execution_status_serde {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    use crate::domain::workflow::ExecutionStatus;

    pub(super) fn serialize<S>(value: &ExecutionStatus, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_str())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<ExecutionStatus, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "running" => Ok(ExecutionStatus::Running),
            "waiting_approval" => Ok(ExecutionStatus::WaitingApproval),
            "completed" => Ok(ExecutionStatus::Completed),
            "failed" => Ok(ExecutionStatus::Failed),
            "aborted" => Ok(ExecutionStatus::Aborted),
            "interrupted" => Ok(ExecutionStatus::Interrupted),
            value => Err(D::Error::custom(format!(
                "unknown execution status: {value}"
            ))),
        }
    }
}

mod execution_origin_serde {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    use crate::domain::workflow::ExecutionOrigin;

    pub(super) fn serialize<S>(value: &ExecutionOrigin, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_public_value())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<ExecutionOrigin, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        ExecutionOrigin::from_public_value(&value).map_err(D::Error::custom)
    }
}

mod token_usage_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::domain::workflow::TokenUsage;

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StoredTokenUsage {
        input_tokens: u64,
        output_tokens: u64,
    }

    pub(super) fn serialize<S>(value: &TokenUsage, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        StoredTokenUsage {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<TokenUsage, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stored = StoredTokenUsage::deserialize(deserializer)?;
        Ok(TokenUsage {
            input_tokens: stored.input_tokens,
            output_tokens: stored.output_tokens,
        })
    }
}

/// Execution metadata 永続化のサブディレクトリ名。
#[cfg(test)]
const EXECUTIONS_SUBDIR: &str = "workflow_executions";
#[cfg(test)]
const MAX_EXECUTION_METADATA_BYTES: u64 = 256 * 1024;

#[cfg(test)]
fn executions_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(EXECUTIONS_SUBDIR)
}

#[cfg(test)]
fn execution_file_path(data_dir: &Path, execution_id: &str) -> PathBuf {
    executions_dir(data_dir).join(format!("{execution_id}.json"))
}

/// `execution_id` を UUID として検証する。Execution Store のすべての lookup/read 経路で path traversal
/// を防ぐ目的で利用する（Spec issues-1011: 信頼境界・execution_id の形式検証）。
fn is_valid_execution_id(execution_id: &str) -> bool {
    uuid::Uuid::parse_str(execution_id).is_ok()
}

/// `path` が `executions_dir` の直下にあり、ファイル名のステムが `execution_id` と一致することを検証する
/// （canonicalize 後の prefix 一致 + metadata.execution_id == 渡された execution_id の二重検査）。
#[cfg(test)]
fn is_within_executions_dir(executions_dir: &Path, path: &Path) -> bool {
    let canonical_executions_dir = match fs::canonicalize(executions_dir) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let canonical_path = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    canonical_path
        .parent()
        .is_some_and(|parent| parent == canonical_executions_dir)
}

/// `workflow_executions/{execution_id}.json` の検証済みローダ。Spec issues-1011 line 130:
/// 外部入力として、以下の条件を満たさないものは破損エントリとして扱う:
/// - ファイル名 stem が UUID 形式
/// - metadata.execution_id がファイル名 stem と一致
///
/// list / reverse lookup の両経路でこの loader を共有することで、検証ロジックを 1 箇所に
/// 集約する（Spec issues-1011 finding 11: list_completed と resolve_worktree_by_execution の検証
/// レベルの分散を解消）。
#[cfg(test)]
fn load_validated_execution_file(path: &Path) -> Result<WorkflowExecutionMetadata, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "missing file stem".to_string())?;
    if !is_valid_execution_id(stem) {
        return Err(format!("invalid execution_id in filename: {stem}"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("stat: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err("metadata file must not be a symlink".to_string());
    }
    if metadata.len() > MAX_EXECUTION_METADATA_BYTES {
        return Err(format!(
            "metadata file too large: {} bytes (max {MAX_EXECUTION_METADATA_BYTES})",
            metadata.len()
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let execution: WorkflowExecutionMetadata =
        serde_json::from_str(&text).map_err(|e| format!("deserialize: {e}"))?;
    if execution.execution_id != stem {
        return Err(format!(
            "metadata.execution_id ({}) does not match filename stem ({stem})",
            execution.execution_id
        ));
    }
    Ok(execution)
}

#[cfg(test)]
fn load_validated_metadata_entry(
    executions_dir: &Path,
    path: &Path,
) -> Result<WorkflowExecutionMetadata, String> {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err("metadata entry is not a json file".to_string());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("metadata file must not be a symlink".to_string());
        }
        Ok(_) => {}
        Err(e) => return Err(format!("stat: {e}")),
    }
    if !is_within_executions_dir(executions_dir, path) {
        return Err("metadata path is outside workflow_executions/".to_string());
    }
    load_validated_execution_file(path)
}

/// [05] API / CLI 共通の projection helper。`Vec<WorkflowExecutionMetadata>` に filter（status /
/// worktree_path）を適用し、active を先頭・以降は完了時刻降順で並べた
/// `Vec<WorkflowExecutionSummary>` を返す。
///
/// `ExecutionStore::list_executions`（API 経路）と CLI の file-direct 経路の双方が同じ projection
/// に揃うことで観測ロジックの divergence を防ぐ（spec [05] API / CLI の意味的等価性境界）。
pub fn project_executions_to_summaries(
    executions: Vec<WorkflowExecutionMetadata>,
    filter: &ExecutionListFilter,
) -> Vec<WorkflowExecutionSummary> {
    let mut summaries: Vec<WorkflowExecutionSummary> = executions
        .into_iter()
        .filter(|execution| match filter.status {
            // Public `active` filter means unfinished, and therefore includes a
            // resumable Interrupted checkpoint. This is distinct from the
            // in-memory active reservation set (Running / WaitingApproval only).
            Some(ExecutionStatusFilter::Active) => !execution.status.is_finished(),
            Some(ExecutionStatusFilter::Terminal) => execution.status.is_finished(),
            None => true,
        })
        .filter(|execution| match filter.worktree_path.as_deref() {
            Some(wt) => execution.worktree_path == wt,
            None => true,
        })
        .map(|execution| WorkflowExecutionSummary::from(&execution))
        .collect();
    sort_summaries_active_first(&mut summaries);
    summaries
}

/// `Vec<WorkflowExecutionSummary>` を「active を先頭・以降は completed_at（無ければ updated_at）
/// の降順」で並び替える。projection helper と ExecutionStore::list_executions から共通で使う。
pub(crate) fn sort_summaries_active_first(summaries: &mut [WorkflowExecutionSummary]) {
    summaries.sort_by(|a, b| {
        let a_active = !a.status.is_finished();
        let b_active = !b.status.is_finished();
        match (a_active, b_active) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let a_key = a.completed_at.unwrap_or(a.updated_at);
                let b_key = b.completed_at.unwrap_or(b.updated_at);
                b_key
                    .partial_cmp(&a_key)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        }
    });
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub(crate) struct WorkflowExecutionMetadataScan {
    pub(crate) executions: Vec<WorkflowExecutionMetadata>,
    pub(crate) is_complete: bool,
}

#[cfg(test)]
impl Default for WorkflowExecutionMetadataScan {
    fn default() -> Self {
        Self {
            executions: Vec::new(),
            is_complete: true,
        }
    }
}

#[cfg(test)]
pub(crate) fn scan_valid_execution_metadata(data_dir: &Path) -> WorkflowExecutionMetadataScan {
    let executions_dir = executions_dir(data_dir);
    if !executions_dir.exists() {
        return WorkflowExecutionMetadataScan::default();
    }
    let entries = match fs::read_dir(&executions_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("ExecutionStore: failed to read executions dir: {e}");
            return WorkflowExecutionMetadataScan {
                executions: Vec::new(),
                is_complete: false,
            };
        }
    };
    let mut executions = Vec::new();
    let mut is_complete = true;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("ExecutionStore: failed to read execution metadata entry: {e}");
                is_complete = false;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_validated_metadata_entry(&executions_dir, &path) {
            Ok(execution) => executions.push(execution),
            Err(e) => {
                log::warn!(
                    "ExecutionStore: skip corrupted execution metadata at {}: {e}",
                    path.display()
                );
                is_complete = false;
            }
        }
    }
    WorkflowExecutionMetadataScan {
        executions,
        is_complete,
    }
}

#[cfg(test)]
pub(crate) fn read_valid_execution_metadata(
    data_dir: &Path,
    execution_id: &str,
) -> Result<Option<WorkflowExecutionMetadata>, String> {
    if !is_valid_execution_id(execution_id) {
        return Err("invalid execution_id".to_string());
    }
    let executions_dir = executions_dir(data_dir);
    let path = execution_file_path(data_dir, execution_id);
    if !path.exists() {
        return Ok(None);
    }
    load_validated_metadata_entry(&executions_dir, &path).map(Some)
}

#[cfg(test)]
pub(crate) fn project_valid_execution_metadata_page(
    data_dir: &Path,
    filter: &ExecutionListFilter,
    offset: usize,
    limit: usize,
) -> Vec<WorkflowExecutionSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let executions_dir = executions_dir(data_dir);
    let entries = match fs::read_dir(&executions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("ExecutionStore: failed to read executions dir: {error}");
            return Vec::new();
        }
    };
    let window_end = offset.saturating_add(limit);
    let mut window = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                log::warn!("ExecutionStore: failed to read execution metadata entry: {error}");
                continue;
            }
        };
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let execution = match load_validated_metadata_entry(&executions_dir, &path) {
            Ok(execution) => execution,
            Err(error) => {
                log::warn!(
                    "ExecutionStore: skip corrupted execution metadata at {}: {error}",
                    path.display()
                );
                continue;
            }
        };
        if !execution_matches_filter(&execution, filter) {
            continue;
        }
        let insertion_index = window
            .binary_search_by(|candidate| compare_execution_metadata(candidate, &execution))
            .unwrap_or_else(|index| index);
        if insertion_index < window_end {
            window.insert(insertion_index, execution);
            if window.len() > window_end {
                window.pop();
            }
        }
    }
    window
        .into_iter()
        .skip(offset)
        .map(|execution| WorkflowExecutionSummary::from(&execution))
        .collect()
}

#[cfg(test)]
fn execution_matches_filter(
    execution: &WorkflowExecutionMetadata,
    filter: &ExecutionListFilter,
) -> bool {
    let status_matches = match filter.status {
        Some(ExecutionStatusFilter::Active) => !execution.status.is_finished(),
        Some(ExecutionStatusFilter::Terminal) => execution.status.is_finished(),
        None => true,
    };
    status_matches
        && filter
            .worktree_path
            .as_deref()
            .is_none_or(|worktree| execution.worktree_path == worktree)
}

#[cfg(test)]
fn compare_execution_metadata(
    left: &WorkflowExecutionMetadata,
    right: &WorkflowExecutionMetadata,
) -> std::cmp::Ordering {
    match (!left.status.is_finished(), !right.status.is_finished()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => {
            let left_key = left.completed_at.unwrap_or(left.updated_at);
            let right_key = right.completed_at.unwrap_or(right.updated_at);
            right_key
                .partial_cmp(&left_key)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}

#[cfg(test)]
pub(crate) fn iter_valid_execution_metadata(data_dir: &Path) -> Vec<WorkflowExecutionMetadata> {
    let WorkflowExecutionMetadataScan { executions, .. } = scan_valid_execution_metadata(data_dir);
    executions
}

/// Execution Store の in-memory state。`active` と `by_worktree` を単一 Mutex で保護することで、
/// 重複チェックと挿入を原子的に行う（Spec Rule: 同一 worktree への並行登録不整合を防ぐ）。
struct ExecutionStoreInner {
    active: HashMap<String, WorkflowExecutionMetadata>,
    by_worktree: HashMap<String, String>,
    pending_interrupted_transitions: HashSet<String>,
    pending_resume_worktrees: HashMap<String, String>,
}

impl ExecutionStoreInner {
    fn new() -> Self {
        Self {
            active: HashMap::new(),
            by_worktree: HashMap::new(),
            pending_interrupted_transitions: HashSet::new(),
            pending_resume_worktrees: HashMap::new(),
        }
    }

    /// `execution_id` をキーに `active` / `by_worktree` の両方から削除する補助関数。
    /// `by_worktree` の entry は `worktree_path` から逆引きするため、active から
    /// 取り出した `worktree_path` のみを対象に削除する。
    fn remove_execution(&mut self, execution_id: &str) -> Option<WorkflowExecutionMetadata> {
        let removed = self.active.remove(execution_id)?;
        if self
            .by_worktree
            .get(&removed.worktree_path)
            .is_some_and(|id| id == execution_id)
        {
            self.by_worktree.remove(&removed.worktree_path);
        }
        Some(removed)
    }

    /// `complete_execution` / `update_active` の永続化失敗時の rollback で、`previous` スナップショットを
    /// `active` / `by_worktree` に再投入する。
    ///
    /// `complete_execution` は `remove_execution` 実行と persist の間で Mutex を解放するため、その間に同一
    /// `worktree_path` へ別 execution が `register_active_execution` で割り当てられる可能性がある。その状態で
    /// 無条件に再投入すると以下の不変条件が壊れる:
    /// - `active` 内に同一 `worktree_path` を持つ execution が 2 件存在する
    /// - `by_worktree` と `active` の双方向整合が崩れる（`by_worktree` は 1 件のみ）
    ///
    /// そのため、再投入前に以下を検査し、いずれかが競合する場合は再投入をスキップして false を
    /// 返す。呼出側は warn ログを出して PersistFailed を返すことで、不変条件を保ったまま rollback
    /// を諦める（永続化失敗で失われた状態は呼出元の上位経路で対応する）。
    /// - `by_worktree[previous.worktree_path]` が `previous.execution_id` 以外を指している
    /// - `active` に既に `previous.execution_id` が存在する
    fn try_reinsert_after_persist_failure(&mut self, previous: WorkflowExecutionMetadata) -> bool {
        let worktree_conflict = self
            .by_worktree
            .get(&previous.worktree_path)
            .is_some_and(|id| id != &previous.execution_id)
            || self
                .pending_resume_worktrees
                .get(&previous.worktree_path)
                .is_some_and(|id| id != &previous.execution_id);
        let execution_id_conflict = self.active.contains_key(&previous.execution_id)
            || self
                .pending_interrupted_transitions
                .contains(&previous.execution_id);
        if worktree_conflict || execution_id_conflict {
            return false;
        }
        self.by_worktree.insert(
            previous.worktree_path.clone(),
            previous.execution_id.clone(),
        );
        self.active.insert(previous.execution_id.clone(), previous);
        true
    }
}

/// Execution Store: active/list UI projection と SQLite projection authority を管理する。
///
/// active な execution は in-memory map（`active: HashMap<execution_id, WorkflowExecutionMetadata>`）と
/// secondary index（`by_worktree: HashMap<worktree_path, execution_id>`）として保持する。
/// SQLite authority 導入後は filesystem metadata を読まず、projection は authority から再構築する。
pub struct ExecutionStore {
    inner: Mutex<ExecutionStoreInner>,
    data_dir: Mutex<Option<PathBuf>>,
    authority: Mutex<Option<WorkflowExecutionAuthority>>,
    #[cfg(test)]
    allow_in_memory_without_data_dir: bool,
    #[cfg(test)]
    fail_next_resume_commit: AtomicBool,
    #[cfg(test)]
    fail_next_active_interruption_rollback: AtomicBool,
}

#[derive(Clone)]
struct ExecutionMetadataStore {
    #[cfg(test)]
    data_dir: PathBuf,
}

impl ExecutionMetadataStore {
    #[cfg(test)]
    fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    async fn persist(&self, execution: WorkflowExecutionMetadata) -> Result<(), String> {
        #[cfg(test)]
        {
            persist_metadata(self.data_dir.clone(), execution).await
        }
        #[cfg(not(test))]
        {
            let _ = execution;
            Err("legacy workflow metadata fixture is unavailable in production".to_string())
        }
    }

    async fn remove(&self, execution_id: String) -> Result<(), String> {
        #[cfg(test)]
        {
            remove_metadata_file(self.data_dir.clone(), execution_id).await
        }
        #[cfg(not(test))]
        {
            let _ = execution_id;
            Err("legacy workflow metadata fixture is unavailable in production".to_string())
        }
    }

    #[cfg(test)]
    async fn list_valid(&self) -> Vec<WorkflowExecutionMetadata> {
        let data_dir = self.data_dir.clone();
        tokio::task::spawn_blocking(move || iter_valid_execution_metadata(&data_dir))
            .await
            .unwrap_or_else(|e| {
                log::warn!("ExecutionStore: failed to join metadata listing request: {e}");
                Vec::new()
            })
    }
}

#[derive(Clone)]
struct WorkflowExecutionAuthority {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
}

#[derive(Clone, Copy)]
enum WorkflowProjectionMutationExpectation {
    CreateAbsent,
    UpdatePresent,
}

enum WorkflowExecutionAuthorityRead {
    Absent,
    Present {
        metadata: WorkflowExecutionMetadata,
        revision: Revision,
    },
    Deleted {
        revision: Revision,
    },
}

#[derive(Clone, Copy)]
enum WorkflowProjectionMutationBase {
    Absent,
    Present { revision: Revision },
}

enum WorkflowMetadataTransition<'a> {
    InterruptedAbort {
        completed_at: f64,
    },
    EventReconciliation {
        projected: &'a DomainWorkflowExecution,
    },
}

impl WorkflowExecutionAuthority {
    fn storage_key(execution_id: &str) -> String {
        format!("workflow:{execution_id}")
    }

    fn worktree_storage_key(worktree_path: &str) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(worktree_path.as_bytes());
        format!("workflow-worktree:{}", hex::encode(digest))
    }

    async fn load(&self, execution_id: &str) -> Result<WorkflowExecutionAuthorityRead, String> {
        let result = self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: Self::storage_key(execution_id),
            })
            .await
            .map_err(|error| format!("workflow SQLite projection read failed: {error}"))?;
        let LocalEventQueryResult::SessionProjectionByIdentity(current) = result else {
            return Err("workflow SQLite projection returned the wrong result type".to_string());
        };
        let Some(current) = current else {
            return Ok(WorkflowExecutionAuthorityRead::Absent);
        };
        match current.projection {
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Deleted {
                    execution_id: stored_id,
                },
            ) if stored_id == execution_id => Ok(WorkflowExecutionAuthorityRead::Deleted {
                revision: current.revision,
            }),
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Present(execution),
            ) if execution.execution_id == execution_id => {
                Ok(WorkflowExecutionAuthorityRead::Present {
                    metadata: workflow_execution_metadata(&execution),
                    revision: current.revision,
                })
            }
            _ => Err("workflow SQLite projection invariant failed".to_string()),
        }
    }

    async fn projection_mutations(
        &self,
        execution: &WorkflowExecutionMetadata,
        base: WorkflowProjectionMutationBase,
    ) -> Result<Vec<LocalStateMutation>, String> {
        let (expected, revision) = match base {
            WorkflowProjectionMutationBase::Absent => (
                RevisionGuard::Absent,
                Revision::new(0).expect("zero revision"),
            ),
            WorkflowProjectionMutationBase::Present { revision } => (
                RevisionGuard::Expected(revision),
                revision
                    .next()
                    .ok_or_else(|| "workflow projection revision exhausted".to_string())?,
            ),
        };
        let worktree_key = Self::worktree_storage_key(&execution.worktree_path);
        let worktree_current = match self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: worktree_key.clone(),
            })
            .await
            .map_err(|error| format!("workflow worktree owner read failed: {error}"))?
        {
            LocalEventQueryResult::SessionProjectionByIdentity(value) => value,
            _ => return Err("workflow worktree owner returned the wrong result type".to_string()),
        };
        if let Some(current) = &worktree_current {
            let SessionProjectionRecord::WorkflowWorktreeOwner(owner) = &current.projection else {
                return Err("workflow worktree owner is incompatible".to_string());
            };
            if owner.worktree_path != execution.worktree_path {
                return Err("workflow worktree owner invariant failed".to_string());
            }
            if owner.active && owner.execution_id != execution.execution_id {
                return Err("workflow worktree already has an active execution".to_string());
            }
        }
        let (worktree_expected, worktree_revision) = match worktree_current {
            Some(current) => (
                RevisionGuard::Expected(current.revision),
                current
                    .revision
                    .next()
                    .ok_or_else(|| "workflow worktree owner revision exhausted".to_string())?,
            ),
            None => (
                RevisionGuard::Absent,
                Revision::new(0).expect("zero revision"),
            ),
        };
        let worktree_projection = WorkflowWorktreeOwnerRecord {
            worktree_path: execution.worktree_path.clone(),
            execution_id: execution.execution_id.clone(),
            active: !execution.status.is_finished(),
        };
        Ok(vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: Self::storage_key(&execution.execution_id),
                projection: SessionProjectionRecord::WorkflowExecution(
                    WorkflowExecutionProjectionRecord::Present(workflow_execution_record(
                        execution,
                    )),
                ),
                expected,
                revision,
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: format!("workflow-execution-{}", execution.execution_id),
                record: ObligationRecord::WorkflowExecution {
                    execution: workflow_execution_record(execution),
                },
                pending: (!execution.status.is_finished()).then(|| PendingIndexEntry {
                    ordered_key: format!("workflow_execution:{}", execution.execution_id),
                    owner: "workflow-runtime".to_string(),
                    partition: PendingPartition::UnownedRuntime,
                    shutdown_plan: None,
                }),
                expected,
                revision,
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: worktree_key,
                projection: SessionProjectionRecord::WorkflowWorktreeOwner(worktree_projection),
                expected: worktree_expected,
                revision: worktree_revision,
            }),
        ])
    }

    async fn deletion_mutations(
        &self,
        execution: &WorkflowExecutionMetadata,
        revision: Revision,
    ) -> Result<Vec<LocalStateMutation>, String> {
        let next_revision = revision
            .next()
            .ok_or_else(|| "workflow projection revision exhausted".to_string())?;
        let worktree_key = Self::worktree_storage_key(&execution.worktree_path);
        let worktree_current = match self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: worktree_key.clone(),
            })
            .await
            .map_err(|error| format!("workflow worktree owner read failed: {error}"))?
        {
            LocalEventQueryResult::SessionProjectionByIdentity(value) => value,
            _ => return Err("workflow worktree owner returned the wrong result type".to_string()),
        };
        let (worktree_expected, worktree_revision) = match &worktree_current {
            Some(current) => {
                let SessionProjectionRecord::WorkflowWorktreeOwner(owner) = &current.projection
                else {
                    return Err("workflow worktree owner is incompatible".to_string());
                };
                if owner.worktree_path != execution.worktree_path
                    || owner.execution_id != execution.execution_id
                {
                    return Err("workflow worktree owner invariant failed".to_string());
                }
                (
                    RevisionGuard::Expected(current.revision),
                    current
                        .revision
                        .next()
                        .ok_or_else(|| "workflow worktree owner revision exhausted".to_string())?,
                )
            }
            None => (
                RevisionGuard::Absent,
                Revision::new(0).expect("zero revision"),
            ),
        };
        Ok(vec![
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: Self::storage_key(&execution.execution_id),
                projection: SessionProjectionRecord::WorkflowExecution(
                    WorkflowExecutionProjectionRecord::Deleted {
                        execution_id: execution.execution_id.clone(),
                    },
                ),
                expected: RevisionGuard::Expected(revision),
                revision: next_revision,
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: format!("workflow-execution-{}", execution.execution_id),
                record: ObligationRecord::WorkflowExecution {
                    execution: workflow_execution_record(execution),
                },
                pending: None,
                expected: RevisionGuard::Expected(revision),
                revision: next_revision,
            }),
            LocalStateMutation::SessionProjection(SessionProjectionMutation {
                session_id: worktree_key,
                projection: SessionProjectionRecord::WorkflowWorktreeOwner(
                    WorkflowWorktreeOwnerRecord {
                        worktree_path: execution.worktree_path.clone(),
                        execution_id: execution.execution_id.clone(),
                        active: false,
                    },
                ),
                expected: worktree_expected,
                revision: worktree_revision,
            }),
        ])
    }

    async fn list_non_terminal(&self) -> Result<Vec<WorkflowExecutionMetadata>, String> {
        let mut cursor = None;
        let mut executions = Vec::new();
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 200,
                    partition: None,
                    owner: Some("workflow-runtime".to_string()),
                    ordered_key_prefix: Some("workflow_execution:".to_string()),
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|error| format!("workflow recovery projection read failed: {error}"))?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(
                    "workflow recovery projection returned the wrong result type".to_string(),
                );
            };
            for stored in page.entries {
                let ObligationRecord::WorkflowExecution { execution } = &stored.record else {
                    return Err("pending workflow projection is incompatible".to_string());
                };
                let execution = workflow_execution_metadata(execution);
                if execution.status.is_finished() {
                    return Err(
                        "terminal workflow is present in pending recovery index".to_string()
                    );
                }
                executions.push(execution);
            }
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(QueryCursor::from_opaque(next.as_str().to_string()));
        }
        Ok(executions)
    }
}

#[derive(Deserialize)]
struct WorkflowExecutionProjectionV1 {
    schema: String,
    deleted: bool,
    #[serde(default)]
    execution: Option<WorkflowExecutionMetadata>,
    #[serde(default)]
    execution_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct WorkflowWorktreeOwnerV1 {
    schema: String,
    worktree_path: String,
    execution_id: String,
    active: bool,
}

fn decode_workflow_projection(raw: &str) -> Result<WorkflowExecutionProjectionV1, String> {
    let envelope: WorkflowExecutionProjectionV1 = serde_json::from_str(raw)
        .map_err(|_| "workflow SQLite projection decoding failed".to_string())?;
    if envelope.schema != "workflow_execution_projection_v1" {
        return Err("workflow SQLite projection schema is unsupported".to_string());
    }
    Ok(envelope)
}

pub(crate) fn encode_workflow_execution_projection_record_v1(
    projection: &WorkflowExecutionProjectionRecord,
) -> Result<String, String> {
    let value = match projection {
        WorkflowExecutionProjectionRecord::Present(execution) => {
            let execution = workflow_execution_metadata(execution);
            serde_json::json!({
                "schema": "workflow_execution_projection_v1",
                "deleted": false,
                "execution": execution,
            })
        }
        WorkflowExecutionProjectionRecord::Deleted { execution_id } => serde_json::json!({
            "schema": "workflow_execution_projection_v1",
            "deleted": true,
            "execution_id": execution_id,
        }),
    };
    serde_json::to_string(&value).map_err(|_| "workflow projection encoding failed".to_string())
}

pub(crate) fn decode_workflow_execution_projection_record_v1(
    raw: &str,
) -> Result<WorkflowExecutionProjectionRecord, String> {
    let envelope = decode_workflow_projection(raw)?;
    match (envelope.deleted, envelope.execution, envelope.execution_id) {
        (false, Some(execution), None) if !execution.execution_id.is_empty() => Ok(
            WorkflowExecutionProjectionRecord::Present(workflow_execution_record(&execution)),
        ),
        (true, None, Some(execution_id)) if !execution_id.is_empty() => {
            Ok(WorkflowExecutionProjectionRecord::Deleted { execution_id })
        }
        _ => Err("workflow projection invariant failed".to_string()),
    }
}

pub(crate) fn encode_workflow_worktree_owner_record_v1(
    owner: &WorkflowWorktreeOwnerRecord,
) -> Result<String, String> {
    serde_json::to_string(&WorkflowWorktreeOwnerV1 {
        schema: "workflow_worktree_owner_v1".to_string(),
        worktree_path: owner.worktree_path.clone(),
        execution_id: owner.execution_id.clone(),
        active: owner.active,
    })
    .map_err(|_| "workflow worktree owner encoding failed".to_string())
}

pub(crate) fn decode_workflow_worktree_owner_record_v1(
    raw: &str,
) -> Result<WorkflowWorktreeOwnerRecord, String> {
    let owner: WorkflowWorktreeOwnerV1 = serde_json::from_str(raw)
        .map_err(|_| "workflow worktree owner is incompatible".to_string())?;
    if owner.schema != "workflow_worktree_owner_v1"
        || owner.worktree_path.is_empty()
        || owner.execution_id.is_empty()
    {
        return Err("workflow worktree owner invariant failed".to_string());
    }
    Ok(WorkflowWorktreeOwnerRecord {
        worktree_path: owner.worktree_path,
        execution_id: owner.execution_id,
        active: owner.active,
    })
}

#[cfg(test)]
impl Default for ExecutionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionStore {
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ExecutionStoreInner::new()),
            data_dir: Mutex::new(None),
            authority: Mutex::new(None),
            #[cfg(test)]
            allow_in_memory_without_data_dir: false,
            #[cfg(test)]
            fail_next_resume_commit: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_active_interruption_rollback: AtomicBool::new(false),
        }
    }

    pub(crate) fn new_canonical(
        data_dir: Option<PathBuf>,
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            inner: Mutex::new(ExecutionStoreInner::new()),
            data_dir: Mutex::new(data_dir),
            authority: Mutex::new(Some(WorkflowExecutionAuthority {
                repository,
                installation_id,
            })),
            #[cfg(test)]
            allow_in_memory_without_data_dir: false,
            #[cfg(test)]
            fail_next_resume_commit: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_active_interruption_rollback: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn new_in_memory_for_tests() -> Self {
        Self {
            inner: Mutex::new(ExecutionStoreInner::new()),
            data_dir: Mutex::new(None),
            authority: Mutex::new(None),
            allow_in_memory_without_data_dir: true,
            fail_next_resume_commit: AtomicBool::new(false),
            fail_next_active_interruption_rollback: AtomicBool::new(false),
        }
    }

    /// データディレクトリを設定する。アプリ起動時の setup から 1 度だけ呼ぶ。
    #[cfg(test)]
    pub async fn set_data_dir(&self, dir: PathBuf) {
        let mut guard = self.data_dir.lock().await;
        *guard = Some(dir);
    }

    #[cfg(test)]
    pub async fn set_local_event_repository(
        &self,
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) {
        *self.authority.lock().await = Some(WorkflowExecutionAuthority {
            repository,
            installation_id,
        });
    }

    pub(crate) async fn local_event_authority(
        &self,
    ) -> Option<(Arc<dyn LocalEventTransactionRepository>, String)> {
        self.authority.lock().await.as_ref().map(|authority| {
            (
                authority.repository.clone(),
                authority.installation_id.clone(),
            )
        })
    }

    async fn data_dir(&self) -> Option<PathBuf> {
        self.data_dir.lock().await.clone()
    }

    pub(crate) async fn configured_data_dir(&self) -> Option<PathBuf> {
        self.data_dir().await
    }

    #[cfg(test)]
    pub(crate) async fn data_dir_for_test(&self) -> Option<PathBuf> {
        self.data_dir().await
    }

    #[cfg(test)]
    async fn persistence_dir(&self) -> Result<Option<PathBuf>, ExecutionStoreError> {
        match self.data_dir().await {
            Some(dir) => Ok(Some(dir)),
            None if self.allow_in_memory_without_data_dir => Ok(None),
            None => Err(ExecutionStoreError::DataDirNotConfigured),
        }
    }

    async fn metadata_store(&self) -> Result<Option<ExecutionMetadataStore>, ExecutionStoreError> {
        if self.authority.lock().await.is_some() {
            // With the permanent SQLite authority installed, the in-memory
            // map is a bounded list/UI projection rebuilt from SQLite. The
            // legacy workflow_executions JSON tree is not written or deleted,
            // so its filesystem outcome cannot accept, reject, or roll back a
            // canonical workflow transition.
            return Ok(None);
        }
        #[cfg(not(test))]
        return Err(ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        });
        #[cfg(test)]
        Ok(self
            .persistence_dir()
            .await?
            .map(ExecutionMetadataStore::new))
    }

    /// 新規 execution を active として登録し、metadata を初期保存する。
    /// 既に同一 worktree に別 execution_id の active execution が存在する場合は `Err` を返す。
    /// 既に同一 execution_id を別 worktree_path で登録しようとした場合も `Err` を返す
    /// （古い by_worktree index が孤立しないように同一 critical section で拒否する）。
    ///
    /// 重複チェックと active map 更新だけを Mutex 内で行い、metadata 永続化は
    /// `spawn_blocking` 経由で実行する。永続化失敗時は同一 snapshot の場合だけ rollback する。
    pub async fn register_active_execution(
        &self,
        execution: WorkflowExecutionMetadata,
    ) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(&execution.execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution.execution_id.clone(),
            });
        }
        // Spec issues-1011 finding 11: terminal status の active 登録を型レベル相当の
        // runtime guard で禁止する。active 集合の不変条件（is_active な execution のみが
        // active に存在する）を API 境界で強制し、`update_active` の typed invariant と
        // 整合させる。
        if !execution.status.is_active() {
            return Err(ExecutionStoreError::NonActiveStatusInActiveSet {
                execution_id: execution.execution_id.clone(),
                status: execution.status,
            });
        }
        let metadata_store = self.metadata_store().await?;
        {
            let mut inner = self.inner.lock().await;
            if inner
                .pending_interrupted_transitions
                .contains(&execution.execution_id)
            {
                return Err(ExecutionStoreError::TransitionInProgress {
                    execution_id: execution.execution_id.clone(),
                });
            }
            // 同一 execution_id が別 worktree で既に登録されている場合は不整合（古い by_worktree が
            // 孤立する原因）なので拒否する。
            if let Some(existing) = inner.active.get(&execution.execution_id) {
                if existing.worktree_path != execution.worktree_path {
                    return Err(ExecutionStoreError::ExecutionIdWorktreeMismatch {
                        execution_id: execution.execution_id.clone(),
                        existing_worktree_path: existing.worktree_path.clone(),
                        new_worktree_path: execution.worktree_path.clone(),
                    });
                }
            }
            // 同一 worktree に別 execution_id の active execution があれば拒否する。
            if let Some(existing_execution_id) = inner.by_worktree.get(&execution.worktree_path) {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            if let Some(existing_execution_id) =
                inner.pending_resume_worktrees.get(&execution.worktree_path)
            {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            inner.by_worktree.insert(
                execution.worktree_path.clone(),
                execution.execution_id.clone(),
            );
            inner
                .active
                .insert(execution.execution_id.clone(), execution.clone());
        }
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(execution.clone()).await {
                let mut inner = self.inner.lock().await;
                if inner
                    .active
                    .get(&execution.execution_id)
                    .is_some_and(|active| active == &execution)
                {
                    inner.remove_execution(&execution.execution_id);
                }
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution.execution_id,
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// command rollback 専用: mutation 前の active snapshot を in-memory Execution Store に戻す。
    ///
    /// 通常の `register_active_execution` は metadata 永続化に失敗すると in-memory 挿入も取り消す。
    /// しかし command 受理サイクルの rollback では、失敗原因が Execution Store 永続化先そのものの
    /// 障害である場合でも、少なくとも process 内の active projection は mutation 前 snapshot
    /// に戻す必要がある。metadata persist は best-effort として試み、失敗は Err で返すが、
    /// in-memory snapshot は保持する。
    pub(crate) async fn restore_active_snapshot_for_rollback(
        &self,
        execution: WorkflowExecutionMetadata,
    ) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(&execution.execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution.execution_id.clone(),
            });
        }
        if !execution.status.is_active() {
            return Err(ExecutionStoreError::NonActiveStatusInActiveSet {
                execution_id: execution.execution_id.clone(),
                status: execution.status,
            });
        }
        let metadata_store = self.metadata_store().await?;
        {
            let mut inner = self.inner.lock().await;
            if inner
                .pending_interrupted_transitions
                .contains(&execution.execution_id)
            {
                return Err(ExecutionStoreError::TransitionInProgress {
                    execution_id: execution.execution_id.clone(),
                });
            }
            if let Some(existing) = inner.active.get(&execution.execution_id) {
                if existing.worktree_path != execution.worktree_path {
                    return Err(ExecutionStoreError::ExecutionIdWorktreeMismatch {
                        execution_id: execution.execution_id.clone(),
                        existing_worktree_path: existing.worktree_path.clone(),
                        new_worktree_path: execution.worktree_path.clone(),
                    });
                }
            }
            if let Some(existing_execution_id) = inner.by_worktree.get(&execution.worktree_path) {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            if let Some(existing_execution_id) =
                inner.pending_resume_worktrees.get(&execution.worktree_path)
            {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            inner.by_worktree.insert(
                execution.worktree_path.clone(),
                execution.execution_id.clone(),
            );
            inner
                .active
                .insert(execution.execution_id.clone(), execution.clone());
        }
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(execution.clone()).await {
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution.execution_id,
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// active execution の現在 node / status / updated_at を更新する（状態遷移ではない属性更新含む）。
    /// `mutator` は in-memory の execution を直接書き換える。metadata 永続化は Mutex を解放してから
    /// `spawn_blocking` 経由で行い、永続化失敗時は同一 snapshot の場合だけ rollback する。
    ///
    /// 永続化失敗時は in-memory 側の変更を rollback して `Err` を返す。
    /// Spec issues-1011 finding 4: `execution_id` は UUID 形式である必要がある。
    async fn update_active<F>(
        &self,
        execution_id: &str,
        mutator: F,
    ) -> Result<(), ExecutionStoreError>
    where
        F: FnOnce(&mut WorkflowExecutionMetadata),
    {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        let (previous, updated) = {
            let mut inner = self.inner.lock().await;
            let Some(execution) = inner.active.get_mut(execution_id) else {
                // 対象が存在しない場合は no-op（呼出元の状態遷移後 race を許容する）。
                return Ok(());
            };
            let previous = execution.clone();
            mutator(execution);
            // Spec issues-1011 finding 10: typed invariant guard。
            // 呼出側が execution_id / worktree_path / terminal status を変更しないことを mutation 後に
            // 必ず再検証する。違反時は in-memory state を rollback し、永続化に進まない。
            if execution.execution_id != previous.execution_id {
                *execution = previous.clone();
                return Err(ExecutionStoreError::ImmutableFieldChanged {
                    execution_id: previous.execution_id,
                    field: "execution_id".to_string(),
                });
            }
            if execution.worktree_path != previous.worktree_path {
                *execution = previous.clone();
                return Err(ExecutionStoreError::ImmutableFieldChanged {
                    execution_id: previous.execution_id,
                    field: "worktree_path".to_string(),
                });
            }
            if !execution.status.is_active() {
                *execution = previous.clone();
                return Err(ExecutionStoreError::NonActiveNotAllowedInUpdate {
                    execution_id: previous.execution_id,
                });
            }
            (previous, execution.clone())
        };
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(updated.clone()).await {
                let mut inner = self.inner.lock().await;
                if let Some(execution) = inner.active.get_mut(execution_id) {
                    if *execution == updated {
                        *execution = previous;
                    }
                }
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.to_string(),
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// engine の active snapshot から Execution Store の active projection を同期する。
    #[cfg(test)]
    pub async fn sync_active_projection(
        &self,
        execution_id: &str,
        status: ExecutionStatus,
        current_node: Option<String>,
        updated_at: f64,
    ) -> Result<(), ExecutionStoreError> {
        self.sync_active_projection_with_usage(execution_id, status, current_node, updated_at, None)
            .await
    }

    /// engine の active snapshot から token usage を含む read projection を同期する。
    ///
    /// `sync_active_projection` は既存の状態-only 呼出し向けに残し、runtime の durable
    /// commit 境界ではこちらを使う。これにより stop 後の Interrupted metadata と、
    /// terminal metadata の直前に観測される active read model が event-log snapshot と
    /// 同じ累計 usage を持つ。
    pub async fn sync_active_projection_with_usage(
        &self,
        execution_id: &str,
        status: ExecutionStatus,
        current_node: Option<String>,
        updated_at: f64,
        total_token_usage: Option<TokenUsage>,
    ) -> Result<(), ExecutionStoreError> {
        self.update_active(execution_id, |execution| {
            execution.status = status;
            execution.current_node = current_node;
            execution.updated_at = updated_at;
            if let Some(total_token_usage) = total_token_usage {
                execution.total_token_usage = total_token_usage;
            }
        })
        .await
    }

    /// active execution の現在値を rollback 用 snapshot として取得する。
    pub async fn active_execution_snapshot(
        &self,
        execution_id: &str,
    ) -> Option<WorkflowExecutionMetadata> {
        let inner = self.inner.lock().await;
        inner.active.get(execution_id).cloned()
    }

    /// active execution を terminal 状態に遷移させ、active set から除外する。metadata は更新して残す。
    /// in-memory mutation と persist を分離し、active map の Mutex を同期ファイル I/O 中に
    /// 保持しない（Spec issues-1011: Execution Store 永続化責務分離）。
    /// 永続化失敗時は in-memory active set への再投入による rollback を試みる。
    /// Spec issues-1011 finding 4: `execution_id` は UUID 形式である必要がある。
    /// Spec issues-1011 finding 12: terminal 制約は型レベル（`TerminalExecutionStatus`）で強制する。
    pub async fn complete_execution(
        &self,
        execution_id: &str,
        status: TerminalExecutionStatus,
        completed_at: f64,
        error_reason: Option<String>,
    ) -> Result<(), ExecutionStoreError> {
        self.complete_execution_with_usage(execution_id, status, completed_at, error_reason, None)
            .await
    }

    /// active execution を terminal 状態へ遷移し、同じ projection commit で累計 usage も
    /// 確定する。通常 runtime は event append 後の authoritative snapshot を渡す。
    pub async fn complete_execution_with_usage(
        &self,
        execution_id: &str,
        status: TerminalExecutionStatus,
        completed_at: f64,
        error_reason: Option<String>,
        total_token_usage: Option<TokenUsage>,
    ) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        let (previous, completed) = {
            let mut inner = self.inner.lock().await;
            let Some(execution) = inner.remove_execution(execution_id) else {
                return Ok(());
            };
            let previous = execution.clone();
            let mut completed = execution;
            completed.status = status.into();
            completed.completed_at = Some(completed_at);
            completed.updated_at = completed_at;
            completed.error_reason = error_reason;
            if let Some(total_token_usage) = total_token_usage {
                completed.total_token_usage = total_token_usage;
            }
            (previous, completed)
        };
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(completed).await {
                // rollback: terminal 化を取り消し、active set / by_worktree に戻す。
                // lock 解放区間に同一 worktree_path / execution_id へ別 execution が register_active_execution
                // されている場合は、再投入により不変条件（同一 worktree につき active は最大 1 件・
                // by_worktree と active の双方向整合）が壊れるため、競合検出時は再投入を諦める。
                let mut inner = self.inner.lock().await;
                let previous_execution_id = previous.execution_id.clone();
                if !inner.try_reinsert_after_persist_failure(previous) {
                    log::warn!(
                        "ExecutionStore: skip rollback reinsertion for {previous_execution_id} due to concurrent active conflict"
                    );
                }
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.to_string(),
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// Running / WaitingApproval execution を再開可能な checkpoint として永続化し、
    /// in-memory active reservation を解放する。
    #[cfg(test)]
    pub async fn interrupt_execution(
        &self,
        execution_id: &str,
        reason: ExecutionInterruptionReason,
        resume_from_node: Option<String>,
        updated_at: f64,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        self.interrupt_execution_with_usage(
            execution_id,
            reason,
            resume_from_node,
            updated_at,
            None,
        )
        .await
    }

    /// active execution を Interrupted checkpoint へ遷移し、最後に確定した node までの
    /// 累計 usage を同じ metadata write で保持する。
    pub async fn interrupt_execution_with_usage(
        &self,
        execution_id: &str,
        reason: ExecutionInterruptionReason,
        resume_from_node: Option<String>,
        updated_at: f64,
        total_token_usage: Option<TokenUsage>,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        let (previous, interrupted) = {
            let mut inner = self.inner.lock().await;
            let Some(previous) = inner.active.get(execution_id).cloned() else {
                return Err(ExecutionStoreError::ExecutionNotFound {
                    execution_id: execution_id.to_string(),
                });
            };
            if !previous.status.can_stop() {
                return Err(ExecutionStoreError::InvalidStatusTransition {
                    execution_id: execution_id.to_string(),
                    actual: previous.status,
                    expected: "running|waiting_approval",
                });
            }
            inner.remove_execution(execution_id);
            let mut interrupted = previous.clone();
            interrupted.status = ExecutionStatus::Interrupted;
            interrupted.updated_at = updated_at;
            interrupted.completed_at = None;
            interrupted.error_reason = None;
            interrupted.interruption_reason = Some(reason);
            interrupted.resume_from_node =
                resume_from_node.or_else(|| previous.current_node.clone());
            interrupted.current_node = None;
            if let Some(total_token_usage) = total_token_usage {
                interrupted.total_token_usage = total_token_usage;
            }
            (previous, interrupted)
        };
        if let Some(store) = metadata_store {
            if let Err(reason) = store.persist(interrupted.clone()).await {
                let mut inner = self.inner.lock().await;
                if !inner.try_reinsert_after_persist_failure(previous) {
                    log::warn!(
                        "ExecutionStore: skip interrupt rollback reinsertion for {execution_id} due to concurrent active conflict"
                    );
                }
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.to_string(),
                    reason,
                });
            }
        }
        Ok(interrupted)
    }

    /// Active interruption の event append から process/session cleanup 完了まで、同じ
    /// execution command と worktree reservation を直列化する。
    pub async fn reserve_active_interruption(
        &self,
        execution_id: &str,
    ) -> Result<ActiveInterruptionReservation, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        self.metadata_store().await?;
        let mut inner = self.inner.lock().await;
        let active = inner.active.get(execution_id).cloned().ok_or_else(|| {
            ExecutionStoreError::ExecutionNotFound {
                execution_id: execution_id.to_string(),
            }
        })?;
        if !active.status.can_stop() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: execution_id.to_string(),
                actual: active.status,
                expected: "running|waiting_approval",
            });
        }
        if !inner
            .pending_interrupted_transitions
            .insert(execution_id.to_string())
        {
            return Err(ExecutionStoreError::TransitionInProgress {
                execution_id: execution_id.to_string(),
            });
        }
        if let Some(owner) = inner
            .pending_resume_worktrees
            .get(&active.worktree_path)
            .cloned()
        {
            if owner != execution_id {
                inner.pending_interrupted_transitions.remove(execution_id);
                return Err(ExecutionStoreError::WorktreeAlreadyActive {
                    worktree_path: active.worktree_path,
                    existing_execution_id: owner,
                });
            }
        }
        inner
            .pending_resume_worktrees
            .insert(active.worktree_path.clone(), execution_id.to_string());
        Ok(ActiveInterruptionReservation {
            execution_id: execution_id.to_string(),
            worktree_path: active.worktree_path,
        })
    }

    pub async fn finish_active_interruption(
        &self,
        reservation: ActiveInterruptionReservation,
    ) -> Result<(), ExecutionStoreError> {
        #[cfg(test)]
        if self
            .fail_next_active_interruption_rollback
            .swap(false, Ordering::AcqRel)
        {
            return Err(ExecutionStoreError::InterruptionReservationChanged {
                execution_id: reservation.execution_id,
            });
        }
        let mut inner = self.inner.lock().await;
        let execution_reserved = inner
            .pending_interrupted_transitions
            .remove(&reservation.execution_id);
        let worktree_reserved = inner
            .pending_resume_worktrees
            .get(&reservation.worktree_path)
            .is_some_and(|owner| owner == &reservation.execution_id);
        if worktree_reserved {
            inner
                .pending_resume_worktrees
                .remove(&reservation.worktree_path);
        }
        if !execution_reserved || !worktree_reserved {
            return Err(ExecutionStoreError::InterruptionReservationChanged {
                execution_id: reservation.execution_id,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_active_interruption_rollback_for_test(&self) {
        self.fail_next_active_interruption_rollback
            .store(true, Ordering::Release);
    }

    pub async fn interrupted_transition_pending(&self, execution_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .pending_interrupted_transitions
            .contains(execution_id)
    }

    /// persisted Interrupted execution の resume transition を予約する。
    ///
    /// ExecutionResumed event が durable になるまでは metadata / active projection を
    /// Interrupted のまま保つ。pending guard と worktree reservation だけを先に取り、同じ
    /// execution への abort/resume と同じ worktree への新規 start を直列化する。
    pub async fn reserve_interrupted_for_resume(
        &self,
        execution_id: &str,
        updated_at: f64,
    ) -> Result<ResumeExecutionReservation, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        self.metadata_store().await?;
        let interrupted = self
            .get_execution_record(execution_id)
            .await?
            .ok_or_else(|| ExecutionStoreError::ExecutionNotFound {
                execution_id: execution_id.to_string(),
            })?;
        if !interrupted.status.can_resume() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: execution_id.to_string(),
                actual: interrupted.status,
                expected: "interrupted",
            });
        }
        let resume_from_node = interrupted.resume_from_node.clone().ok_or_else(|| {
            ExecutionStoreError::MissingResumePoint {
                execution_id: execution_id.to_string(),
            }
        })?;
        let mut running = interrupted.clone();
        running.status = ExecutionStatus::Running;
        running.current_node = Some(resume_from_node);
        running.updated_at = updated_at;
        running.completed_at = None;
        running.error_reason = None;
        running.interruption_reason = None;
        running.resume_from_node = None;

        {
            let mut inner = self.inner.lock().await;
            if inner.pending_interrupted_transitions.contains(execution_id) {
                return Err(ExecutionStoreError::TransitionInProgress {
                    execution_id: execution_id.to_string(),
                });
            }
            if let Some(existing) = inner.active.get(execution_id) {
                return Err(ExecutionStoreError::InvalidStatusTransition {
                    execution_id: execution_id.to_string(),
                    actual: existing.status,
                    expected: "interrupted",
                });
            }
            if let Some(existing_execution_id) = inner.by_worktree.get(&running.worktree_path) {
                if existing_execution_id != execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: running.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            if let Some(existing_execution_id) =
                inner.pending_resume_worktrees.get(&running.worktree_path)
            {
                if existing_execution_id != execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: running.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            inner
                .pending_interrupted_transitions
                .insert(execution_id.to_string());
            inner
                .pending_resume_worktrees
                .insert(running.worktree_path.clone(), execution_id.to_string());
        }
        Ok(ResumeExecutionReservation {
            interrupted,
            running,
        })
    }

    /// ExecutionResumed event append 成功後に Running metadata / active projection を commit
    /// し、resume transition guard を解放する。
    pub async fn commit_resume_reservation(
        &self,
        reservation: &ResumeExecutionReservation,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        let execution_id = reservation.running.execution_id.clone();
        {
            let inner = self.inner.lock().await;
            let reserved = inner
                .pending_interrupted_transitions
                .contains(&execution_id)
                && inner
                    .pending_resume_worktrees
                    .get(&reservation.running.worktree_path)
                    .is_some_and(|owner| owner == &execution_id);
            if !reserved || inner.active.contains_key(&execution_id) {
                return Err(ExecutionStoreError::ResumeReservationChanged { execution_id });
            }
        }
        #[cfg(test)]
        if self.fail_next_resume_commit.swap(false, Ordering::AcqRel) {
            return Err(ExecutionStoreError::PersistFailed {
                execution_id,
                reason: "injected Resume metadata commit failure".to_string(),
            });
        }
        if let Some(store) = self.metadata_store().await? {
            store
                .persist(reservation.running.clone())
                .await
                .map_err(|reason| ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.clone(),
                    reason,
                })?;
        }
        let mut inner = self.inner.lock().await;
        let reserved = inner
            .pending_interrupted_transitions
            .contains(&execution_id)
            && inner
                .pending_resume_worktrees
                .get(&reservation.running.worktree_path)
                .is_some_and(|owner| owner == &execution_id);
        if !reserved || inner.active.contains_key(&execution_id) {
            return Err(ExecutionStoreError::ResumeReservationChanged { execution_id });
        }
        if let Some(existing_execution_id) =
            inner.by_worktree.get(&reservation.running.worktree_path)
        {
            if existing_execution_id != &execution_id {
                return Err(ExecutionStoreError::WorktreeAlreadyActive {
                    worktree_path: reservation.running.worktree_path.clone(),
                    existing_execution_id: existing_execution_id.clone(),
                });
            }
        }
        inner.by_worktree.insert(
            reservation.running.worktree_path.clone(),
            execution_id.clone(),
        );
        inner
            .active
            .insert(execution_id.clone(), reservation.running.clone());
        inner.pending_interrupted_transitions.remove(&execution_id);
        inner
            .pending_resume_worktrees
            .remove(&reservation.running.worktree_path);
        Ok(reservation.running.clone())
    }

    /// A durable `ExecutionResumed` followed by a failed Running projection is closed by a
    /// durable Crash checkpoint. Persist that checkpoint from the reservation's original
    /// Interrupted metadata and release the transition only after the projection succeeds.
    #[cfg(test)]
    pub async fn checkpoint_failed_resume(
        &self,
        reservation: ResumeExecutionReservation,
        reason: ExecutionInterruptionReason,
        updated_at: f64,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        let execution_id = reservation.interrupted.execution_id.clone();
        {
            let inner = self.inner.lock().await;
            let reserved = inner
                .pending_interrupted_transitions
                .contains(&execution_id)
                && inner
                    .pending_resume_worktrees
                    .get(&reservation.running.worktree_path)
                    .is_some_and(|owner| owner == &execution_id);
            if !reserved || inner.active.contains_key(&execution_id) {
                return Err(ExecutionStoreError::ResumeReservationChanged { execution_id });
            }
        }
        let mut interrupted = reservation.interrupted;
        interrupted.status = ExecutionStatus::Interrupted;
        interrupted.current_node = None;
        interrupted.updated_at = updated_at;
        interrupted.completed_at = None;
        interrupted.error_reason = None;
        interrupted.interruption_reason = Some(reason);
        if let Some(store) = self.metadata_store().await? {
            store.persist(interrupted.clone()).await.map_err(|reason| {
                ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.clone(),
                    reason,
                }
            })?;
        }
        let mut inner = self.inner.lock().await;
        let reserved = inner.pending_interrupted_transitions.remove(&execution_id);
        let worktree_reserved = inner
            .pending_resume_worktrees
            .get(&reservation.running.worktree_path)
            .is_some_and(|owner| owner == &execution_id);
        if worktree_reserved {
            inner
                .pending_resume_worktrees
                .remove(&reservation.running.worktree_path);
        }
        if !reserved || !worktree_reserved {
            return Err(ExecutionStoreError::ResumeReservationChanged { execution_id });
        }
        Ok(interrupted)
    }

    #[cfg(test)]
    pub(crate) fn fail_next_resume_commit_for_test(&self) {
        self.fail_next_resume_commit.store(true, Ordering::Release);
    }

    /// Resume の required event append が失敗した場合に pending guard / worktree reservation
    /// を解放する。metadata は commit 前なので Interrupted のまま変わらない。
    pub async fn rollback_resume_reservation(
        &self,
        reservation: ResumeExecutionReservation,
    ) -> Result<(), ExecutionStoreError> {
        let execution_id = reservation.interrupted.execution_id.clone();
        self.metadata_store().await?;
        let mut inner = self.inner.lock().await;
        let reserved = inner.pending_interrupted_transitions.remove(&execution_id);
        let worktree_reserved = inner
            .pending_resume_worktrees
            .get(&reservation.running.worktree_path)
            .is_some_and(|owner| owner == &execution_id);
        if worktree_reserved {
            inner
                .pending_resume_worktrees
                .remove(&reservation.running.worktree_path);
        }
        if !reserved || !worktree_reserved {
            return Err(ExecutionStoreError::ResumeReservationChanged { execution_id });
        }
        Ok(())
    }

    /// Interrupted metadata を検証し、event append が失敗した場合に rollback できる token を返す。
    /// append-only event log を最初の永続 commit point に保つため、この段階では metadata を
    /// 書き換えない。同じ execution に対する resume / abort の同時受理を防ぐ in-memory
    /// reservation だけを commit または rollback まで保持する。
    pub async fn reserve_interrupted_for_abort(
        &self,
        execution_id: &str,
        completed_at: f64,
    ) -> Result<AbortInterruptedReservation, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        // Production store では event append 前に永続化先が構成済みであることだけ検証する。
        // metadata 自体は required event の append 成功後に commit する。
        self.metadata_store().await?;
        let interrupted = self
            .get_execution_record(execution_id)
            .await?
            .ok_or_else(|| ExecutionStoreError::ExecutionNotFound {
                execution_id: execution_id.to_string(),
            })?;
        if !interrupted.status.can_resume() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: execution_id.to_string(),
                actual: interrupted.status,
                expected: "interrupted",
            });
        }
        {
            let mut inner = self.inner.lock().await;
            if inner.active.contains_key(execution_id) {
                return Err(ExecutionStoreError::InvalidStatusTransition {
                    execution_id: execution_id.to_string(),
                    actual: inner.active[execution_id].status,
                    expected: "interrupted",
                });
            }
            if !inner
                .pending_interrupted_transitions
                .insert(execution_id.to_string())
            {
                return Err(ExecutionStoreError::TransitionInProgress {
                    execution_id: execution_id.to_string(),
                });
            }
        }

        let mut aborted = interrupted.clone();
        aborted.status = ExecutionStatus::Aborted;
        aborted.updated_at = completed_at;
        aborted.completed_at = Some(completed_at);
        aborted.error_reason = None;
        aborted.interruption_reason = None;
        aborted.resume_from_node = None;
        Ok(AbortInterruptedReservation {
            interrupted,
            aborted,
        })
    }

    /// ExecutionAborted event append 成功後に Aborted metadata projection を永続化し、
    /// reservation を解放する。永続化失敗時は event log が正典として残り、同一 process 内で
    /// stale Interrupted metadata を再操作できないよう reservation を保持する。
    pub async fn commit_interrupted_abort(
        &self,
        reservation: AbortInterruptedReservation,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        let execution_id = reservation.aborted.execution_id.clone();
        {
            let inner = self.inner.lock().await;
            if !inner
                .pending_interrupted_transitions
                .contains(&execution_id)
            {
                return Err(ExecutionStoreError::AbortReservationChanged { execution_id });
            }
        }
        if let Some(store) = self.metadata_store().await? {
            store
                .persist(reservation.aborted.clone())
                .await
                .map_err(|reason| ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.clone(),
                    reason,
                })?;
        }
        let removed = self
            .inner
            .lock()
            .await
            .pending_interrupted_transitions
            .remove(&execution_id);
        if !removed {
            return Err(ExecutionStoreError::AbortReservationChanged { execution_id });
        }
        Ok(reservation.aborted)
    }

    /// ExecutionAborted event append 失敗時に in-memory reservation を解放する。
    /// metadata は commit 前には変更していないため、Interrupted checkpoint のまま残る。
    pub async fn rollback_interrupted_abort(
        &self,
        reservation: AbortInterruptedReservation,
    ) -> Result<(), ExecutionStoreError> {
        let execution_id = reservation.interrupted.execution_id.clone();
        let removed = self
            .inner
            .lock()
            .await
            .pending_interrupted_transitions
            .remove(&execution_id);
        if !removed {
            return Err(ExecutionStoreError::AbortReservationChanged { execution_id });
        }
        Ok(())
    }

    /// 起動時に stale な Running / WaitingApproval / Interrupted metadata を
    /// event-log projection に揃える。
    /// projection が active のままなら、呼出側が先に ExecutionInterrupted を append する必要がある。
    pub async fn reconcile_orphan_from_projection(
        &self,
        mut metadata: WorkflowExecutionMetadata,
        projection: &DomainWorkflowExecution,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        if !is_valid_execution_id(&metadata.execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: metadata.execution_id,
            });
        }
        if metadata.status.is_finished() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: metadata.execution_id.clone(),
                actual: metadata.status,
                expected: "running|waiting_approval|interrupted",
            });
        }
        if projection.id != metadata.execution_id {
            return Err(ExecutionStoreError::ImmutableFieldChanged {
                execution_id: metadata.execution_id,
                field: "execution_id".to_string(),
            });
        }
        if projection.worktree_path != metadata.worktree_path {
            return Err(ExecutionStoreError::ExecutionIdWorktreeMismatch {
                execution_id: metadata.execution_id,
                existing_worktree_path: metadata.worktree_path,
                new_worktree_path: projection.worktree_path.clone(),
            });
        }
        if projection.status.is_active() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: metadata.execution_id,
                actual: projection.status,
                expected: "interrupted|completed|failed|aborted",
            });
        }

        metadata.workflow_name = projection.workflow_name.clone();
        metadata.status = projection.status;
        metadata.current_node = projection.current_node.clone();
        metadata.created_from = projection.created_from;
        metadata.started_at = projection.started_at;
        metadata.updated_at = projection.updated_at;
        metadata.completed_at = projection.completed_at;
        metadata.error_reason = projection.error_reason.clone();
        metadata.interruption_reason = projection.interruption_reason;
        metadata.resume_from_node = projection.resume_from_node.clone();
        metadata.total_token_usage = projection.total_token_usage.clone();
        if let Some(store) = self.metadata_store().await? {
            store.persist(metadata.clone()).await.map_err(|reason| {
                ExecutionStoreError::PersistFailed {
                    execution_id: metadata.execution_id.clone(),
                    reason,
                }
            })?;
        }
        Ok(metadata)
    }

    /// active set から該当 execution を取り除き、永続化された metadata ファイルも削除する。
    /// `complete_execution` と異なり terminal metadata は残さず、reservation 状態を完全に
    /// 撤回する用途で使う（Spec issues-1011 finding 9: start_workflow の rollback で
    /// 失敗した reservation を撤回するため、terminal entry を completed 一覧に残さない）。
    pub async fn cancel_reservation(&self, execution_id: &str) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        // Spec issues-1011 finding 7: metadata file 削除を先に試み、成功後にのみ
        // in-memory active / by_worktree を消す。remove_file 失敗時は active reservation を
        // 維持したまま Err を返し、孤立した metadata を残さない（呼出側の fallback
        // complete_execution が対象を引けるようにする）。
        if let Some(store) = metadata_store {
            if let Err(e) = store.remove(execution_id.to_string()).await {
                log::warn!(
                    "ExecutionStore: failed to remove reservation metadata for {execution_id}: {e}"
                );
                return Err(ExecutionStoreError::PersistFailed {
                    execution_id: execution_id.to_string(),
                    reason: e,
                });
            }
        }
        let mut inner = self.inner.lock().await;
        inner.remove_execution(execution_id);
        Ok(())
    }

    /// active な execution を一覧する（`WorkflowExecutionSummary` で返す）。
    pub async fn list_active(&self) -> Result<Vec<WorkflowExecutionSummary>, ExecutionStoreError> {
        if let Some(authority) = self.authority.lock().await.clone() {
            let executions = authority.list_non_terminal().await.map_err(|error| {
                ExecutionStoreError::AuthorityReadFailed {
                    reason: error.to_string(),
                }
            })?;
            let mut executions: Vec<_> = executions
                .iter()
                .filter(|execution| execution.status.is_active())
                .map(WorkflowExecutionSummary::from)
                .collect();
            executions.sort_by(|a, b| {
                b.started_at
                    .partial_cmp(&a.started_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return Ok(executions);
        }
        #[cfg(test)]
        if self.allow_in_memory_without_data_dir {
            let inner = self.inner.lock().await;
            let mut executions: Vec<WorkflowExecutionSummary> = inner
                .active
                .values()
                .map(WorkflowExecutionSummary::from)
                .collect();
            executions.sort_by(|a, b| {
                b.started_at
                    .partial_cmp(&a.started_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return Ok(executions);
        }
        Err(ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        })
    }

    /// Rebuild the bounded in-memory list/UI mirror from the SQLite pending
    /// index. This method never writes authority state and is safe to retry
    /// after a post-commit projection failure.
    pub(crate) async fn rebuild_active_projection_from_authority(
        &self,
    ) -> Result<(), ExecutionStoreError> {
        let authority = self.authority.lock().await.clone().ok_or_else(|| {
            ExecutionStoreError::AuthorityReadFailed {
                reason: "SQLite workflow authority is not installed".to_string(),
            }
        })?;
        let executions = authority
            .list_non_terminal()
            .await
            .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason })?;
        let mut inner = self.inner.lock().await;
        inner.active.clear();
        inner.by_worktree.clear();
        inner.pending_resume_worktrees.clear();
        inner.pending_interrupted_transitions.clear();
        for execution in executions
            .into_iter()
            .filter(|execution| execution.status.is_active())
        {
            inner.by_worktree.insert(
                execution.worktree_path.clone(),
                execution.execution_id.clone(),
            );
            inner
                .active
                .insert(execution.execution_id.clone(), execution);
        }
        Ok(())
    }

    /// テスト専用: worktree_path 限定の active+terminal 一覧（合成順）を返す。
    /// production 経路は `list_executions(ExecutionListFilter { worktree_path: Some(..), .. })` を使う。
    #[cfg(test)]
    pub async fn list_for_worktree(&self, worktree_path: &str) -> Vec<WorkflowExecutionSummary> {
        self.list_executions(ExecutionListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })
        .await
    }

    #[cfg(test)]
    pub async fn list_completed(&self) -> Vec<WorkflowExecutionSummary> {
        self.list_executions(ExecutionListFilter {
            status: Some(ExecutionStatusFilter::Terminal),
            worktree_path: None,
        })
        .await
    }

    /// [05] read-only API: active / terminal を含む全 execution summary を、optional な
    /// status / worktree filter を適用して返す。filter なしの場合は全件を返す。
    /// 並び順は active を先頭・以降は完了時刻降順とする。
    ///
    /// Test-only file fixture または SQLite authority から再構築した bounded in-memory
    /// projection を読む。最終的に CLI と同じ
    /// `project_executions_to_summaries` を経由することで観測ロジックの divergence を防ぐ
    /// （spec [05] API / CLI の意味的等価性境界, 観測値の整合性境界: list / get の
    /// データソース統一）。
    #[cfg(test)]
    pub async fn list_executions(
        &self,
        filter: ExecutionListFilter,
    ) -> Vec<WorkflowExecutionSummary> {
        let active_executions: HashMap<String, WorkflowExecutionMetadata> = {
            let inner = self.inner.lock().await;
            inner.active.clone()
        };
        let file_executions = match self.metadata_store().await {
            Ok(Some(store)) => store.list_valid().await,
            _ => Vec::new(),
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut combined: Vec<WorkflowExecutionMetadata> =
            Vec::with_capacity(file_executions.len() + active_executions.len());
        for execution in active_executions.values() {
            if seen.insert(execution.execution_id.clone()) {
                combined.push(execution.clone());
            }
        }
        for execution in file_executions {
            if seen.insert(execution.execution_id.clone()) {
                combined.push(execution);
            }
        }
        project_executions_to_summaries(combined, &filter)
    }

    /// テスト専用 API: 単一 execution の summary を取得する。
    /// active map → terminal metadata file の順で lookup する。`execution_id` は UUID 形式
    /// として検証する（path traversal 対策）。
    #[cfg(test)]
    pub async fn get_execution(&self, execution_id: &str) -> Option<WorkflowExecutionSummary> {
        self.get_execution_record(execution_id)
            .await
            .ok()
            .flatten()
            .map(|execution| WorkflowExecutionSummary::from(&execution))
    }

    pub(crate) async fn get_execution_record(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionMetadata>, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        if let Some(authority) = self.authority.lock().await.clone() {
            return authority
                .load(execution_id)
                .await
                .map(|read| match read {
                    WorkflowExecutionAuthorityRead::Present { metadata, .. } => Some(metadata),
                    WorkflowExecutionAuthorityRead::Absent
                    | WorkflowExecutionAuthorityRead::Deleted { .. } => None,
                })
                .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason });
        }
        #[cfg(not(test))]
        {
            Err(ExecutionStoreError::AuthorityReadFailed {
                reason: "workflow SQLite authority is not configured".to_string(),
            })
        }
        #[cfg(test)]
        {
            {
                let inner = self.inner.lock().await;
                if let Some(execution) = inner.active.get(execution_id) {
                    return Ok(Some(execution.clone()));
                }
            }
            let Some(dir) = self.data_dir().await else {
                return if self.allow_in_memory_without_data_dir {
                    Ok(None)
                } else {
                    Err(ExecutionStoreError::DataDirNotConfigured)
                };
            };
            let path = execution_file_path(&dir, execution_id);
            if !path.exists() {
                return Ok(None);
            }
            load_validated_metadata_entry(&executions_dir(&dir), &path)
                .map(Some)
                .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason })
        }
    }

    pub(crate) async fn prepare_atomic_initial_snapshot_mutations(
        &self,
        snapshot: &crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        self.prepare_atomic_snapshot_mutations(
            snapshot,
            WorkflowProjectionMutationExpectation::CreateAbsent,
        )
        .await
    }

    pub(crate) async fn prepare_atomic_existing_snapshot_mutations(
        &self,
        snapshot: &crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        self.prepare_atomic_snapshot_mutations(
            snapshot,
            WorkflowProjectionMutationExpectation::UpdatePresent,
        )
        .await
    }

    async fn prepare_atomic_snapshot_mutations(
        &self,
        snapshot: &crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot,
        expectation: WorkflowProjectionMutationExpectation,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        let authority = self.authority.lock().await.clone();
        #[cfg(test)]
        if authority.is_none() && self.allow_in_memory_without_data_dir {
            return Ok(Vec::new());
        }
        let authority = authority.ok_or_else(|| ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        })?;
        let read = authority
            .load(&snapshot.execution_id)
            .await
            .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason })?;
        let (mut execution, base) = match (expectation, read) {
            (
                WorkflowProjectionMutationExpectation::CreateAbsent,
                WorkflowExecutionAuthorityRead::Absent,
            ) => (
                WorkflowExecutionMetadata {
                    execution_id: snapshot.execution_id.clone(),
                    workflow_name: snapshot.workflow_name.clone(),
                    status: crate::domain::workflow::ExecutionStatus::Running,
                    worktree_path: snapshot.worktree_path.clone(),
                    current_node: Some(snapshot.current_node_name.clone()),
                    created_from: snapshot.created_from,
                    started_at: snapshot.started_at,
                    updated_at: snapshot.updated_at,
                    completed_at: None,
                    error_reason: None,
                    interruption_reason: None,
                    resume_from_node: None,
                    total_token_usage: crate::domain::workflow::TokenUsage::default(),
                },
                WorkflowProjectionMutationBase::Absent,
            ),
            (
                WorkflowProjectionMutationExpectation::CreateAbsent,
                WorkflowExecutionAuthorityRead::Present { .. },
            ) => {
                return Err(ExecutionStoreError::ExecutionAlreadyExists {
                    execution_id: snapshot.execution_id.clone(),
                });
            }
            (
                WorkflowProjectionMutationExpectation::CreateAbsent,
                WorkflowExecutionAuthorityRead::Deleted { revision },
            )
            | (
                WorkflowProjectionMutationExpectation::UpdatePresent,
                WorkflowExecutionAuthorityRead::Deleted { revision },
            ) => {
                return Err(ExecutionStoreError::ExecutionDeleted {
                    execution_id: snapshot.execution_id.clone(),
                    revision,
                });
            }
            (
                WorkflowProjectionMutationExpectation::UpdatePresent,
                WorkflowExecutionAuthorityRead::Present { metadata, revision },
            ) => (
                metadata,
                WorkflowProjectionMutationBase::Present { revision },
            ),
            (
                WorkflowProjectionMutationExpectation::UpdatePresent,
                WorkflowExecutionAuthorityRead::Absent,
            ) => {
                return Err(ExecutionStoreError::ExecutionNotFound {
                    execution_id: snapshot.execution_id.clone(),
                });
            }
        };
        execution.status = match &snapshot.state {
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Running => {
                crate::domain::workflow::ExecutionStatus::Running
            }
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::WaitingApproval => {
                crate::domain::workflow::ExecutionStatus::WaitingApproval
            }
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Interrupted => {
                crate::domain::workflow::ExecutionStatus::Interrupted
            }
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Completed => {
                crate::domain::workflow::ExecutionStatus::Completed
            }
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Failed { .. } => {
                crate::domain::workflow::ExecutionStatus::Failed
            }
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Aborted => {
                crate::domain::workflow::ExecutionStatus::Aborted
            }
        };
        execution.current_node = Some(snapshot.current_node_name.clone());
        execution.updated_at = snapshot.updated_at;
        execution.completed_at = execution
            .status
            .is_finished()
            .then_some(snapshot.updated_at);
        execution.error_reason = snapshot.error_reason.clone();
        execution.interruption_reason = matches!(
            &snapshot.state,
            crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Interrupted
        )
        .then(|| {
            snapshot
                .error_reason
                .as_deref()
                .and_then(crate::domain::workflow::ExecutionInterruptionReason::from_reason)
                .unwrap_or(crate::domain::workflow::ExecutionInterruptionReason::Crash)
        });
        execution.resume_from_node = execution
            .interruption_reason
            .is_some()
            .then(|| snapshot.current_node_name.clone());
        execution.total_token_usage = crate::domain::workflow::TokenUsage {
            input_tokens: snapshot.total_token_usage.input_tokens,
            output_tokens: snapshot.total_token_usage.output_tokens,
        };
        authority
            .projection_mutations(&execution, base)
            .await
            .map_err(|reason| ExecutionStoreError::PersistFailed {
                execution_id: snapshot.execution_id.clone(),
                reason,
            })
    }

    pub(crate) async fn prepare_atomic_interrupted_abort_metadata_mutations(
        &self,
        expected_current: &WorkflowExecutionMetadata,
        completed_at: f64,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        self.prepare_atomic_existing_metadata_transition(
            expected_current,
            WorkflowMetadataTransition::InterruptedAbort { completed_at },
        )
        .await
    }

    pub(crate) async fn prepare_atomic_event_reconciliation_metadata_mutations(
        &self,
        expected_current: &WorkflowExecutionMetadata,
        projected: &DomainWorkflowExecution,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        self.prepare_atomic_existing_metadata_transition(
            expected_current,
            WorkflowMetadataTransition::EventReconciliation { projected },
        )
        .await
    }

    pub(crate) async fn prepare_atomic_stale_reservation_deletion_mutations(
        &self,
        expected_current: &WorkflowExecutionMetadata,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        let authority = self.authority.lock().await.clone();
        #[cfg(test)]
        if authority.is_none() && self.allow_in_memory_without_data_dir {
            return Ok(Vec::new());
        }
        let authority = authority.ok_or_else(|| ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        })?;
        let read = authority
            .load(&expected_current.execution_id)
            .await
            .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason })?;
        let revision = match read {
            WorkflowExecutionAuthorityRead::Absent => {
                return Err(ExecutionStoreError::ExecutionNotFound {
                    execution_id: expected_current.execution_id.clone(),
                });
            }
            WorkflowExecutionAuthorityRead::Deleted { revision } => {
                return Err(ExecutionStoreError::ExecutionDeleted {
                    execution_id: expected_current.execution_id.clone(),
                    revision,
                });
            }
            WorkflowExecutionAuthorityRead::Present { metadata, revision } => {
                if &metadata != expected_current {
                    return Err(ExecutionStoreError::ExecutionRecordChanged {
                        execution_id: expected_current.execution_id.clone(),
                    });
                }
                revision
            }
        };
        authority
            .deletion_mutations(expected_current, revision)
            .await
            .map_err(|reason| ExecutionStoreError::PersistFailed {
                execution_id: expected_current.execution_id.clone(),
                reason,
            })
    }

    async fn prepare_atomic_existing_metadata_transition(
        &self,
        expected_current: &WorkflowExecutionMetadata,
        transition: WorkflowMetadataTransition<'_>,
    ) -> Result<Vec<LocalStateMutation>, ExecutionStoreError> {
        let authority = self.authority.lock().await.clone();
        #[cfg(test)]
        if authority.is_none() && self.allow_in_memory_without_data_dir {
            return Ok(Vec::new());
        }
        let authority = authority.ok_or_else(|| ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        })?;
        let read = authority
            .load(&expected_current.execution_id)
            .await
            .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason })?;
        let (canonical, revision) = match read {
            WorkflowExecutionAuthorityRead::Absent => {
                return Err(ExecutionStoreError::ExecutionNotFound {
                    execution_id: expected_current.execution_id.clone(),
                });
            }
            WorkflowExecutionAuthorityRead::Deleted { revision } => {
                return Err(ExecutionStoreError::ExecutionDeleted {
                    execution_id: expected_current.execution_id.clone(),
                    revision,
                });
            }
            WorkflowExecutionAuthorityRead::Present { metadata, revision } => {
                if &metadata != expected_current {
                    return Err(ExecutionStoreError::ExecutionRecordChanged {
                        execution_id: expected_current.execution_id.clone(),
                    });
                }
                (metadata, revision)
            }
        };
        let execution = match transition {
            WorkflowMetadataTransition::InterruptedAbort { completed_at } => {
                if canonical.status != ExecutionStatus::Interrupted
                    || !completed_at.is_finite()
                    || completed_at < canonical.updated_at
                {
                    return Err(ExecutionStoreError::InvalidCanonicalTransition {
                        execution_id: canonical.execution_id.clone(),
                    });
                }
                let mut aborted = canonical.clone();
                aborted.status = ExecutionStatus::Aborted;
                aborted.updated_at = completed_at;
                aborted.completed_at = Some(completed_at);
                aborted.error_reason = None;
                aborted.interruption_reason = None;
                aborted.resume_from_node = None;
                aborted
            }
            WorkflowMetadataTransition::EventReconciliation { projected } => {
                let projected = WorkflowExecutionMetadata::from(projected);
                if !valid_event_reconciliation_transition(&canonical, &projected) {
                    return Err(ExecutionStoreError::InvalidCanonicalTransition {
                        execution_id: canonical.execution_id.clone(),
                    });
                }
                projected
            }
        };
        if execution == canonical {
            return Ok(Vec::new());
        }
        authority
            .projection_mutations(
                &execution,
                WorkflowProjectionMutationBase::Present { revision },
            )
            .await
            .map_err(|reason| ExecutionStoreError::PersistFailed {
                execution_id: execution.execution_id.clone(),
                reason,
            })
    }

    /// worktree_path から active な execution_id を解決する。
    #[cfg(test)]
    pub async fn resolve_execution_by_worktree(&self, worktree_path: &str) -> Option<String> {
        let inner = self.inner.lock().await;
        inner.by_worktree.get(worktree_path).cloned()
    }

    /// execution_id から worktree_path を解決する。
    /// active な execution のみならず、終了済み execution も `workflow_executions/{execution_id}.json` から
    /// metadata を読み込んで返す（Spec Rule 4: 実行インスタンスから worktree を解決する
    /// 対象は active に限定されない）。
    ///
    /// path traversal 対策として、ディスクへフォールバックする場合のみ `execution_id` を
    /// UUID として検証し、解決後のパスが `workflow_executions/` 直下にあり、metadata 内の
    /// `execution_id` フィールドが引数と一致することを二重検査する（Spec issues-1011: 信頼境界）。
    /// in-memory active map の lookup は外部入力を file system に渡さないため検証を要求しない。
    #[cfg(test)]
    pub async fn resolve_worktree_by_execution(&self, execution_id: &str) -> Option<String> {
        {
            let inner = self.inner.lock().await;
            if let Some(execution) = inner.active.get(execution_id) {
                return Some(execution.worktree_path.clone());
            }
        }
        if !is_valid_execution_id(execution_id) {
            log::warn!(
                "ExecutionStore: rejected non-UUID execution_id in resolve_worktree_by_execution"
            );
            return None;
        }
        if let Some(authority) = self.authority.lock().await.clone() {
            return match authority.load(execution_id).await {
                Ok(WorkflowExecutionAuthorityRead::Present { metadata, .. }) => {
                    Some(metadata.worktree_path)
                }
                Ok(WorkflowExecutionAuthorityRead::Absent)
                | Ok(WorkflowExecutionAuthorityRead::Deleted { .. }) => None,
                Err(error) => {
                    log::warn!(
                        "ExecutionStore: canonical workflow reverse lookup failed for {execution_id}: {error}"
                    );
                    None
                }
            };
        }
        let dir = self.data_dir().await?;
        let path = execution_file_path(&dir, execution_id);
        if !path.exists() {
            return None;
        }
        match load_validated_metadata_entry(&executions_dir(&dir), &path) {
            Ok(execution) => Some(execution.worktree_path),
            Err(e) => {
                log::warn!(
                    "ExecutionStore: failed to load execution metadata at {} for reverse lookup: {e}",
                    path.display()
                );
                None
            }
        }
    }

    /// テスト・engine 内部のリストア経路から、active execution の attribute を直接設定する補助。
    /// 通常は `register_active_execution` / `update_active` / `complete_execution` を経由すること。
    #[cfg(test)]
    pub async fn active_len(&self) -> usize {
        self.inner.lock().await.active.len()
    }

    /// Startup replay 対象。active orphan に加え、event commit 後の metadata persist failure
    /// で stale になり得る Interrupted checkpoint も再照合する。
    #[cfg(test)]
    pub async fn list_non_terminal_metadata(&self) -> Vec<WorkflowExecutionMetadata> {
        self.try_list_non_terminal_metadata()
            .await
            .unwrap_or_else(|error| {
                log::warn!("ExecutionStore: non-terminal metadata lookup failed: {error}");
                Vec::new()
            })
    }

    pub(crate) async fn try_list_non_terminal_metadata(
        &self,
    ) -> Result<Vec<WorkflowExecutionMetadata>, ExecutionStoreError> {
        if let Some(authority) = self.authority.lock().await.clone() {
            return authority
                .list_non_terminal()
                .await
                .map_err(|reason| ExecutionStoreError::AuthorityReadFailed { reason });
        }
        #[cfg(not(test))]
        return Err(ExecutionStoreError::AuthorityReadFailed {
            reason: "workflow SQLite authority is not configured".to_string(),
        });
        #[cfg(test)]
        {
            let Some(dir) = self.data_dir().await else {
                return Ok(Vec::new());
            };
            let executions =
                tokio::task::spawn_blocking(move || iter_valid_execution_metadata(&dir))
                    .await
                    .map_err(|error| ExecutionStoreError::AuthorityReadFailed {
                        reason: format!(
                            "failed to join recovery metadata listing request: {error}"
                        ),
                    })?;
            Ok(executions
                .into_iter()
                .filter(|execution| !execution.status.is_finished())
                .collect())
        }
    }
}

#[cfg(test)]
async fn persist_metadata(
    dir: PathBuf,
    execution: WorkflowExecutionMetadata,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || persist_metadata_sync(&dir, &execution))
        .await
        .map_err(|e| format!("metadata persist task failed: {e}"))?
}

/// metadata を `workflow_executions/{execution_id}.json` に永続化する（同期 I/O）。
/// async ExecutionStore API からは `spawn_blocking` 経由で呼び出し、active map の Mutex を
/// ファイル I/O 中に保持しない。
#[cfg(test)]
fn persist_metadata_sync(dir: &Path, execution: &WorkflowExecutionMetadata) -> Result<(), String> {
    let executions_dir = executions_dir(dir);
    if let Err(e) = fs::create_dir_all(&executions_dir) {
        log::warn!("ExecutionStore: failed to create executions dir: {e}");
        return Err(format!("create executions dir: {e}"));
    }
    let path = execution_file_path(dir, &execution.execution_id);
    let json = match serde_json::to_string_pretty(execution) {
        Ok(j) => j,
        Err(e) => {
            log::warn!(
                "ExecutionStore: failed to serialize execution {}: {e}",
                execution.execution_id
            );
            return Err(format!("serialize: {e}"));
        }
    };
    if let Err(e) = atomic_write(&path, &json) {
        log::warn!("ExecutionStore: failed to write {}: {e}", path.display());
        return Err(format!("write {}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
async fn remove_metadata_file(dir: PathBuf, execution_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = execution_file_path(&dir, &execution_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("metadata remove task failed: {e}"))?
}

#[cfg(test)]
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let file_name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let tmp = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let write_result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionStoreError {
    #[cfg(test)]
    #[error("ExecutionStore data_dir is not configured")]
    DataDirNotConfigured,
    #[error("failed to read canonical workflow execution authority: {reason}")]
    AuthorityReadFailed { reason: String },
    #[error("worktree {worktree_path} already has active execution {existing_execution_id}")]
    WorktreeAlreadyActive {
        worktree_path: String,
        existing_execution_id: String,
    },
    #[error(
        "execution_id {execution_id} is already active on worktree {existing_worktree_path} \
         and cannot be re-registered to {new_worktree_path}"
    )]
    ExecutionIdWorktreeMismatch {
        execution_id: String,
        existing_worktree_path: String,
        new_worktree_path: String,
    },
    #[error("failed to persist execution {execution_id} metadata: {reason}")]
    PersistFailed {
        execution_id: String,
        reason: String,
    },
    #[error("invalid execution_id format (must be UUID): {execution_id}")]
    InvalidExecutionId { execution_id: String },
    #[error(
        "cannot register execution {execution_id} into active set with non-active status: {status:?}"
    )]
    NonActiveStatusInActiveSet {
        execution_id: String,
        status: ExecutionStatus,
    },
    #[error("update_active for {execution_id} attempted to change immutable field {field}")]
    ImmutableFieldChanged { execution_id: String, field: String },
    #[error("update_active for {execution_id} cannot transition to a non-active status")]
    NonActiveNotAllowedInUpdate { execution_id: String },
    #[error("workflow execution already exists: {execution_id}")]
    ExecutionAlreadyExists { execution_id: String },
    #[error("workflow execution changed before projection update: {execution_id}")]
    ExecutionRecordChanged { execution_id: String },
    #[error("workflow execution transition is not derived from canonical state: {execution_id}")]
    InvalidCanonicalTransition { execution_id: String },
    #[error("workflow execution was not found: {execution_id}")]
    ExecutionNotFound { execution_id: String },
    #[error("workflow execution projection is deleted: {execution_id} (revision {revision:?})")]
    ExecutionDeleted {
        execution_id: String,
        revision: Revision,
    },
    #[error(
        "execution {execution_id} cannot transition from {actual:?}; expected one of {expected}"
    )]
    InvalidStatusTransition {
        execution_id: String,
        actual: ExecutionStatus,
        expected: &'static str,
    },
    #[error("interrupted execution {execution_id} has no resume point")]
    MissingResumePoint { execution_id: String },
    #[error("resume reservation for execution {execution_id} changed before rollback")]
    ResumeReservationChanged { execution_id: String },
    #[error("execution {execution_id} already has an interrupted transition in progress")]
    TransitionInProgress { execution_id: String },
    #[error("abort reservation for interrupted execution {execution_id} changed")]
    AbortReservationChanged { execution_id: String },
    #[error("active interruption reservation for execution {execution_id} changed")]
    InterruptionReservationChanged { execution_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// テスト内で使う安定 UUID。register_active_execution/update_active/complete_execution の API 境界で
    /// UUID 形式検証が走るため、テスト識別子は UUID にする（Spec issues-1011 finding 4）。
    fn test_uuid(seed: u8) -> String {
        let bytes = [seed; 16];
        uuid::Uuid::from_bytes(bytes).to_string()
    }

    fn make_execution(
        execution_id: &str,
        worktree: &str,
        status: ExecutionStatus,
        started_at: f64,
    ) -> WorkflowExecutionMetadata {
        WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: worktree.to_string(),
            current_node: Some("node-1".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at,
            updated_at: started_at,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    fn projected_execution(metadata: &WorkflowExecutionMetadata) -> DomainWorkflowExecution {
        DomainWorkflowExecution {
            id: metadata.execution_id.clone(),
            workflow_name: metadata.workflow_name.clone(),
            status: metadata.status,
            current_node: metadata.current_node.clone(),
            created_from: metadata.created_from,
            worktree_path: metadata.worktree_path.clone(),
            started_at: metadata.started_at,
            updated_at: metadata.updated_at,
            completed_at: metadata.completed_at,
            error_reason: metadata.error_reason.clone(),
            interruption_reason: metadata.interruption_reason,
            resume_from_node: metadata.resume_from_node.clone(),
            total_token_usage: metadata.total_token_usage.clone(),
            node_executions: Vec::new(),
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    #[derive(Clone)]
    enum CanonicalProjectionFixture {
        Absent,
        Present {
            metadata: WorkflowExecutionMetadata,
            revision: Revision,
        },
        Deleted {
            revision: Revision,
        },
        ReadFailure,
    }

    struct CanonicalProjectionRepository {
        execution_id: String,
        fixture: CanonicalProjectionFixture,
        execution_reads: std::sync::atomic::AtomicUsize,
    }

    impl CanonicalProjectionRepository {
        fn new(execution_id: &str, fixture: CanonicalProjectionFixture) -> Arc<Self> {
            Arc::new(Self {
                execution_id: execution_id.to_string(),
                fixture,
                execution_reads: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn execution_reads(&self) -> usize {
            self.execution_reads
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LocalEventTransactionRepository for CanonicalProjectionRepository {
        async fn commit_batch(
            &self,
            _batch: crate::domain::local_event::LocalAtomicBatch,
        ) -> Result<
            crate::domain::local_event::CommitBatchResult,
            crate::domain::local_event::CommitBatchError,
        > {
            unreachable!("canonical projection read fixture does not commit")
        }

        async fn resolve_commit(
            &self,
            _identity: crate::domain::local_event::CommitIdentity,
        ) -> Result<
            crate::domain::local_event::CommitResolution,
            crate::domain::local_event::LocalEventQueryError,
        > {
            unreachable!("canonical projection read fixture does not resolve commits")
        }

        async fn load_stream(
            &self,
            _request: crate::domain::local_event::LoadStreamRequest,
        ) -> Result<
            crate::domain::local_event::DomainEventPage,
            crate::domain::local_event::LocalEventQueryError,
        > {
            unreachable!("canonical projection read fixture does not load streams")
        }

        async fn query(
            &self,
            request: LocalEventQuery,
        ) -> Result<LocalEventQueryResult, crate::domain::local_event::LocalEventQueryError>
        {
            let LocalEventQuery::SessionProjectionByIdentity { session_id } = request else {
                unreachable!("canonical projection fixture only accepts identity reads");
            };
            if session_id != WorkflowExecutionAuthority::storage_key(&self.execution_id) {
                return Ok(LocalEventQueryResult::SessionProjectionByIdentity(None));
            }
            self.execution_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &self.fixture {
                CanonicalProjectionFixture::Absent => {
                    Ok(LocalEventQueryResult::SessionProjectionByIdentity(None))
                }
                CanonicalProjectionFixture::Present { metadata, revision } => {
                    Ok(LocalEventQueryResult::SessionProjectionByIdentity(Some(
                        crate::domain::local_event::SessionProjectionView {
                            session_id,
                            projection: SessionProjectionRecord::WorkflowExecution(
                                WorkflowExecutionProjectionRecord::Present(
                                    workflow_execution_record(metadata),
                                ),
                            ),
                            revision: *revision,
                        },
                    )))
                }
                CanonicalProjectionFixture::Deleted { revision } => {
                    Ok(LocalEventQueryResult::SessionProjectionByIdentity(Some(
                        crate::domain::local_event::SessionProjectionView {
                            session_id,
                            projection: SessionProjectionRecord::WorkflowExecution(
                                WorkflowExecutionProjectionRecord::Deleted {
                                    execution_id: self.execution_id.clone(),
                                },
                            ),
                            revision: *revision,
                        },
                    )))
                }
                CanonicalProjectionFixture::ReadFailure => {
                    Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                        correlation_id: "workflow-read-failure".to_string(),
                    })
                }
            }
        }

        fn subscribe(
            &self,
            _after: crate::domain::local_event::GlobalSequence,
        ) -> crate::domain::local_event::LocalEventSubscription {
            crate::domain::local_event::LocalEventSubscription::new(Box::pin(
                futures_util::stream::empty(),
            ))
        }
    }

    async fn install_canonical_projection_fixture(
        store: &ExecutionStore,
        execution_id: &str,
        fixture: CanonicalProjectionFixture,
    ) -> Arc<CanonicalProjectionRepository> {
        let repository = CanonicalProjectionRepository::new(execution_id, fixture);
        store
            .set_local_event_repository(repository.clone(), "test-generation".to_string())
            .await;
        repository
    }

    fn snapshot(
        execution_id: &str,
    ) -> crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot {
        crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/wt/canonical".to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: String::new(),
            error_reason: None,
            state: crate::adaptor::gateway::workflow::state::RuntimeExecutionState::Running,
            current_node_index: 0,
            current_node_name: "node-1".to_string(),
            current_session_id: None,
            node_history: Vec::new(),
            node_execution_counts: HashMap::new(),
            workflow_definition:
                crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml::default(),
            total_token_usage: crate::adaptor::gateway::workflow::event::TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            started_at: 100.0,
            updated_at: 101.0,
        }
    }

    #[tokio::test]
    async fn canonical_read_failure_is_not_mapped_to_not_found_or_default_projection() {
        let execution_id = test_uuid(201);
        let store = ExecutionStore::new_in_memory_for_tests();
        install_canonical_projection_fixture(
            &store,
            &execution_id,
            CanonicalProjectionFixture::ReadFailure,
        )
        .await;

        assert!(matches!(
            store.get_execution_record(&execution_id).await,
            Err(ExecutionStoreError::AuthorityReadFailed { .. })
        ));
        assert!(matches!(
            store
                .reserve_interrupted_for_resume(&execution_id, 200.0)
                .await,
            Err(ExecutionStoreError::AuthorityReadFailed { .. })
        ));
        assert!(matches!(
            store
                .prepare_atomic_initial_snapshot_mutations(&snapshot(&execution_id))
                .await,
            Err(ExecutionStoreError::AuthorityReadFailed { .. })
        ));
        assert!(matches!(
            store
                .prepare_atomic_existing_snapshot_mutations(&snapshot(&execution_id))
                .await,
            Err(ExecutionStoreError::AuthorityReadFailed { .. })
        ));
    }

    #[tokio::test]
    async fn snapshot_projection_create_requires_absent_and_update_requires_present() {
        let absent_id = test_uuid(202);
        let absent_store = ExecutionStore::new_in_memory_for_tests();
        let absent_repository = install_canonical_projection_fixture(
            &absent_store,
            &absent_id,
            CanonicalProjectionFixture::Absent,
        )
        .await;
        let initial = absent_store
            .prepare_atomic_initial_snapshot_mutations(&snapshot(&absent_id))
            .await
            .unwrap();
        assert_eq!(initial.len(), 3);
        assert_eq!(
            absent_repository.execution_reads(),
            1,
            "one versioned read supplies both payload and CAS base"
        );
        assert!(matches!(
            absent_store
                .prepare_atomic_existing_snapshot_mutations(&snapshot(&absent_id))
                .await,
            Err(ExecutionStoreError::ExecutionNotFound { .. })
        ));

        let present_id = test_uuid(203);
        let present_store = ExecutionStore::new_in_memory_for_tests();
        let present_revision = Revision::new(7).unwrap();
        let present_repository = install_canonical_projection_fixture(
            &present_store,
            &present_id,
            CanonicalProjectionFixture::Present {
                metadata: make_execution(
                    &present_id,
                    "/wt/canonical",
                    ExecutionStatus::Running,
                    100.0,
                ),
                revision: present_revision,
            },
        )
        .await;
        let update = present_store
            .prepare_atomic_existing_snapshot_mutations(&snapshot(&present_id))
            .await
            .unwrap();
        assert_eq!(
            present_repository.execution_reads(),
            1,
            "update must not re-read and adopt a newer revision for stale payload"
        );
        let LocalStateMutation::SessionProjection(execution_mutation) = &update[0] else {
            panic!("first mutation must update the workflow execution projection");
        };
        assert_eq!(
            execution_mutation.expected,
            RevisionGuard::Expected(present_revision)
        );
        assert_eq!(
            execution_mutation.revision,
            present_revision.next().unwrap()
        );
        assert!(matches!(
            present_store
                .prepare_atomic_initial_snapshot_mutations(&snapshot(&present_id))
                .await,
            Err(ExecutionStoreError::ExecutionAlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn metadata_update_rejects_stale_expected_or_proposed_payload() {
        let execution_id = test_uuid(205);
        let mut current = make_execution(
            &execution_id,
            "/wt/canonical",
            ExecutionStatus::Interrupted,
            100.0,
        );
        current.interruption_reason = Some(ExecutionInterruptionReason::Orphan);
        current.resume_from_node = Some("node-1".to_string());
        current.total_token_usage.input_tokens = 10;
        let mut stale = current.clone();
        stale.updated_at = 99.0;
        let store = ExecutionStore::new_in_memory_for_tests();
        let repository = install_canonical_projection_fixture(
            &store,
            &execution_id,
            CanonicalProjectionFixture::Present {
                metadata: current.clone(),
                revision: Revision::new(9).unwrap(),
            },
        )
        .await;

        assert!(matches!(
            store
                .prepare_atomic_interrupted_abort_metadata_mutations(&stale, 101.0)
                .await,
            Err(ExecutionStoreError::ExecutionRecordChanged { .. })
        ));

        let mut stale_proposed = stale;
        stale_proposed.status = ExecutionStatus::Interrupted;
        stale_proposed.current_node = None;
        stale_proposed.completed_at = None;
        stale_proposed.error_reason = None;
        stale_proposed.interruption_reason = Some(ExecutionInterruptionReason::Orphan);
        stale_proposed.resume_from_node = Some("node-1".to_string());
        assert!(matches!(
            store
                .prepare_atomic_event_reconciliation_metadata_mutations(
                    &current,
                    &projected_execution(&stale_proposed),
                )
                .await,
            Err(ExecutionStoreError::InvalidCanonicalTransition { .. })
        ));
        assert_eq!(
            repository.execution_reads(),
            2,
            "each stale payload must be rejected by its single versioned read"
        );
    }

    #[tokio::test]
    async fn tombstoned_projection_is_rejected_for_create_and_update() {
        let execution_id = test_uuid(204);
        let revision = Revision::new(3).unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        install_canonical_projection_fixture(
            &store,
            &execution_id,
            CanonicalProjectionFixture::Deleted { revision },
        )
        .await;

        assert!(matches!(
            store
                .prepare_atomic_initial_snapshot_mutations(&snapshot(&execution_id))
                .await,
            Err(ExecutionStoreError::ExecutionDeleted {
                revision: actual,
                ..
            }) if actual == revision
        ));
        assert!(matches!(
            store
                .prepare_atomic_existing_snapshot_mutations(&snapshot(&execution_id))
                .await,
            Err(ExecutionStoreError::ExecutionDeleted {
                revision: actual,
                ..
            }) if actual == revision
        ));
    }

    #[tokio::test]
    async fn list_active_requires_canonical_authority_outside_test_fixture_mode() {
        let store = ExecutionStore::new();
        assert!(matches!(
            store.list_active().await,
            Err(ExecutionStoreError::AuthorityReadFailed { .. })
        ));
    }

    /// Rule: workflow を 1 回起動するたびに、その実行は固有の識別子で記録される
    #[tokio::test]
    async fn register_active_records_execution_with_unique_id_and_persists() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_id = test_uuid(1);
        let execution = make_execution(&execution_id, "/wt/a", ExecutionStatus::Running, 100.0);
        store
            .register_active_execution(execution.clone())
            .await
            .unwrap();

        assert_eq!(store.active_len().await, 1);
        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);

        let path = execution_file_path(tmp.path(), &execution_id);
        assert!(path.exists(), "metadata must be persisted at {path:?}");
    }

    #[tokio::test]
    async fn production_store_requires_data_dir_for_mutating_operations() {
        let store = ExecutionStore::new();
        let execution_id = test_uuid(1);
        let err = store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, ExecutionStoreError::DataDirNotConfigured));

        let err = store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                101.0,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ExecutionStoreError::DataDirNotConfigured));
    }

    /// Rule: worktree と実行インスタンスは双方向に解決できる
    #[tokio::test]
    async fn bidirectional_lookup_between_execution_and_worktree() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        let execution = make_execution(&execution_id, "/wt/a", ExecutionStatus::Running, 100.0);
        store.register_active_execution(execution).await.unwrap();

        assert_eq!(
            store.resolve_execution_by_worktree("/wt/a").await,
            Some(execution_id.clone())
        );
        assert_eq!(
            store.resolve_worktree_by_execution(&execution_id).await,
            Some("/wt/a".to_string())
        );
    }

    /// Rule: 同一 worktree に進行中の実行が存在する間は、新たな workflow 起動は拒否される
    #[tokio::test]
    async fn second_active_execution_on_same_worktree_is_rejected() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id_1 = test_uuid(1);
        let execution_id_2 = test_uuid(2);
        store
            .register_active_execution(make_execution(
                &execution_id_1,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let err = store
            .register_active_execution(make_execution(
                &execution_id_2,
                "/wt/a",
                ExecutionStatus::Running,
                101.0,
            ))
            .await
            .unwrap_err();
        match err {
            ExecutionStoreError::WorktreeAlreadyActive {
                existing_execution_id,
                ..
            } => {
                assert_eq!(existing_execution_id, execution_id_1);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        // 既存 active が継続している
        assert_eq!(store.active_len().await, 1);
        assert_eq!(
            store.resolve_execution_by_worktree("/wt/a").await,
            Some(execution_id_1)
        );
    }

    /// Spec issues-1011 finding 6: 同一 execution_id を別 worktree_path で再登録しようとすると
    /// `ExecutionIdWorktreeMismatch` で拒否される。古い by_worktree index が孤立しない。
    #[tokio::test]
    async fn register_active_rejects_same_execution_id_with_different_worktree() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let err = store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/b",
                ExecutionStatus::Running,
                101.0,
            ))
            .await
            .unwrap_err();
        match err {
            ExecutionStoreError::ExecutionIdWorktreeMismatch {
                existing_worktree_path,
                new_worktree_path,
                ..
            } => {
                assert_eq!(existing_worktree_path, "/wt/a");
                assert_eq!(new_worktree_path, "/wt/b");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        // 古い by_worktree index は維持されている（/wt/b への孤立 entry は出ない）
        assert_eq!(
            store.resolve_execution_by_worktree("/wt/a").await,
            Some(execution_id.clone())
        );
        assert_eq!(store.resolve_execution_by_worktree("/wt/b").await, None);
        assert_eq!(store.active_len().await, 1);
    }

    /// Rule: 進行中の実行と終了した実行を区別して一覧できる
    #[tokio::test]
    async fn list_active_and_completed_are_separated() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_active = test_uuid(10);
        let execution_done = test_uuid(11);
        store
            .register_active_execution(make_execution(
                &execution_active,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &execution_done,
                "/wt/b",
                ExecutionStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_done,
                TerminalExecutionStatus::Completed,
                95.0,
                None,
            )
            .await
            .unwrap();

        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_active);
        assert_eq!(active[0].workflow_name, "wf");
        assert_eq!(active[0].worktree_path, "/wt/a");
        assert_eq!(active[0].started_at, 100.0);
        assert_eq!(active[0].updated_at, 100.0);
        assert_eq!(active[0].created_from, ExecutionOrigin::DesktopUi);
        assert_eq!(active[0].total_token_usage, TokenUsage::default());

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].execution_id, execution_done);
        assert_eq!(completed[0].workflow_name, "wf");
        assert_eq!(completed[0].worktree_path, "/wt/b");
        assert_eq!(completed[0].started_at, 90.0);
        assert_eq!(completed[0].updated_at, 95.0);
        assert_eq!(completed[0].created_from, ExecutionOrigin::DesktopUi);
        assert_eq!(completed[0].total_token_usage, TokenUsage::default());
        assert_eq!(completed[0].status, ExecutionStatus::Completed);
        assert_eq!(completed[0].completed_at, Some(95.0));

        let metadata: WorkflowExecutionMetadata = serde_json::from_str(
            &fs::read_to_string(execution_file_path(tmp.path(), &execution_done)).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.workflow_name, "wf");
        assert_eq!(metadata.worktree_path, "/wt/b");
        assert_eq!(metadata.started_at, 90.0);
        assert_eq!(metadata.updated_at, 95.0);
        assert_eq!(metadata.created_from, ExecutionOrigin::DesktopUi);
        assert_eq!(metadata.total_token_usage, TokenUsage::default());
    }

    /// [05] list_executions / list_for_worktree は active を先頭、以降は完了時刻降順で
    /// 返す（spec [05] read-only API 並び順）。worktree filter で対象 worktree のみに
    /// 絞り込まれる。
    #[tokio::test]
    async fn list_for_worktree_combines_filters_and_sorts_executions() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active = test_uuid(30);
        let completed_new = test_uuid(31);
        let completed_other = test_uuid(32);
        store
            .register_active_execution(make_execution(
                &completed_new,
                "/wt/a",
                ExecutionStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &completed_new,
                TerminalExecutionStatus::Completed,
                120.0,
                None,
            )
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &active,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &completed_other,
                "/wt/b",
                ExecutionStatus::Running,
                110.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &completed_other,
                TerminalExecutionStatus::Completed,
                130.0,
                None,
            )
            .await
            .unwrap();

        let executions = store.list_for_worktree("/wt/a").await;
        let ids: Vec<_> = executions
            .iter()
            .map(|execution| execution.execution_id.as_str())
            .collect();
        // active が先頭、以降は完了時刻降順（spec [05] 並び順）。
        assert_eq!(ids, vec![active.as_str(), completed_new.as_str()]);
    }

    /// 終了 status は completed / failed / aborted の 3 つを含む
    #[tokio::test]
    async fn completed_listing_includes_all_terminal_statuses() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_c = test_uuid(20);
        let execution_f = test_uuid(21);
        let execution_a = test_uuid(22);
        for (id, status) in [
            (&execution_c, TerminalExecutionStatus::Completed),
            (&execution_f, TerminalExecutionStatus::Failed),
            (&execution_a, TerminalExecutionStatus::Aborted),
        ] {
            store
                .register_active_execution(make_execution(
                    id,
                    &format!("/wt/{id}"),
                    ExecutionStatus::Running,
                    100.0,
                ))
                .await
                .unwrap();
            store
                .complete_execution(id, status, 101.0, Some("reason".to_string()))
                .await
                .unwrap();
        }

        let completed = store.list_completed().await;
        let ids: std::collections::HashSet<&str> =
            completed.iter().map(|r| r.execution_id.as_str()).collect();
        assert!(ids.contains(execution_c.as_str()));
        assert!(ids.contains(execution_f.as_str()));
        assert!(ids.contains(execution_a.as_str()));
    }

    /// Rule: 既に進行している worktree 上の実行は、新たな識別子を採番せずそのまま実行インスタンスとして扱われる
    ///
    /// Execution Store は採番しないことを確認する。同じ execution_id で再登録すれば（engine 側で
    /// `execution_id` を昇格させる経路に相当）通る。
    #[tokio::test]
    async fn register_with_same_execution_id_for_same_worktree_is_idempotent() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution = make_execution(&test_uuid(1), "/wt/a", ExecutionStatus::Running, 100.0);
        store
            .register_active_execution(execution.clone())
            .await
            .unwrap();
        // 同一 execution_id の再登録は許容（idempotent）
        store.register_active_execution(execution).await.unwrap();
        assert_eq!(store.active_len().await, 1);
    }

    /// Rule: 永続化された実行 metadata の一部が破損していても、実行インスタンスの一覧は継続して提供される
    #[tokio::test]
    async fn list_completed_skips_corrupted_entries() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_ok = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_ok,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_ok,
                TerminalExecutionStatus::Completed,
                101.0,
                None,
            )
            .await
            .unwrap();

        // 破損ファイル: JSON でない
        let executions_dir = executions_dir(tmp.path());
        fs::write(executions_dir.join("broken.json"), "not a json").unwrap();
        // 破損ファイル: ファイル名 stem が UUID でない
        fs::write(
            executions_dir.join("not-a-uuid.json"),
            serde_json::to_string(&make_execution(
                "00000000-0000-0000-0000-000000000099",
                "/wt/forged",
                ExecutionStatus::Completed,
                1.0,
            ))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            executions_dir.join(format!("{}.json", test_uuid(8))),
            "x".repeat((MAX_EXECUTION_METADATA_BYTES + 1) as usize),
        )
        .unwrap();
        // 破損ファイル: filename stem と metadata.execution_id が不一致
        let mismatch_uuid_path = test_uuid(7);
        let mismatch_meta = make_execution(
            "00000000-0000-0000-0000-000000000088",
            "/wt/mismatch",
            ExecutionStatus::Completed,
            1.0,
        );
        fs::write(
            executions_dir.join(format!("{mismatch_uuid_path}.json")),
            serde_json::to_string(&mismatch_meta).unwrap(),
        )
        .unwrap();

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].execution_id, execution_ok);
    }

    #[tokio::test]
    async fn sync_active_projection_mutates_in_memory_and_persists() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .sync_active_projection(
                &execution_id,
                ExecutionStatus::WaitingApproval,
                Some("review".to_string()),
                110.0,
            )
            .await
            .unwrap();

        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, ExecutionStatus::WaitingApproval);
        assert_eq!(active[0].current_node, Some("review".to_string()));

        let path = execution_file_path(tmp.path(), &execution_id);
        let saved: WorkflowExecutionMetadata =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.status, ExecutionStatus::WaitingApproval);
        assert_eq!(saved.updated_at, 110.0);
    }

    /// Rule 4: 終了済み execution でも実行インスタンスから worktree を解決できる。
    /// active から外れて metadata だけが残る状況で reverse lookup が機能することを保証する。
    /// path traversal 対策で disk fallback は UUID 形式のみ許容するため、UUID を使う。
    #[tokio::test]
    async fn resolve_worktree_by_execution_falls_back_to_persisted_metadata_for_terminal_executions(
    ) {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();

        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                105.0,
                None,
            )
            .await
            .unwrap();

        // active からは消えているが、永続化済み metadata から解決できる
        assert_eq!(store.resolve_execution_by_worktree("/wt/a").await, None);
        assert_eq!(
            store.resolve_worktree_by_execution(&execution_id).await,
            Some("/wt/a".to_string())
        );
    }

    /// path traversal 対策: disk fallback の lookup では非 UUID の execution_id を拒否する
    #[tokio::test]
    async fn resolve_worktree_by_execution_rejects_non_uuid_execution_id_on_disk_fallback() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // active には存在しない、disk にも存在しない id を投げる
        assert_eq!(
            store.resolve_worktree_by_execution("../etc/passwd").await,
            None
        );
        assert_eq!(
            store.resolve_worktree_by_execution("not-a-uuid").await,
            None
        );
    }

    /// Spec issues-1011: 同一 worktree への並行 `register_active_execution` で active / by_worktree が
    /// 整合する。Mutex で重複チェックと挿入を 1 critical section に閉じているので、
    /// レース後の状態は「ちょうど 1 つ active」かつ「by_worktree の entry が 1 つ」になる。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_active_on_same_worktree_keeps_active_and_by_worktree_consistent() {
        let store = std::sync::Arc::new(ExecutionStore::new_in_memory_for_tests());
        let mut handles = Vec::new();
        // 8 並列で同一 worktree に異なる execution_id (UUID) で register_active_execution を試みる。
        for i in 0..8 {
            let store_cloned = std::sync::Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let execution = make_execution(
                    &test_uuid(i),
                    "/wt/race",
                    ExecutionStatus::Running,
                    100.0 + i as f64,
                );
                store_cloned.register_active_execution(execution).await
            }));
        }
        let mut ok_count = 0usize;
        let mut conflict_count = 0usize;
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => ok_count += 1,
                Err(ExecutionStoreError::WorktreeAlreadyActive { .. }) => conflict_count += 1,
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
        assert_eq!(
            ok_count, 1,
            "exactly one register_active_execution must succeed under concurrent contention"
        );
        assert_eq!(conflict_count, 7);
        // 結果状態: active は 1 つだけ、by_worktree も 1 entry のみ。
        assert_eq!(store.active_len().await, 1);
        let resolved = store.resolve_execution_by_worktree("/wt/race").await;
        assert!(resolved.is_some());
    }

    /// path traversal 対策: 攻撃者が metadata.execution_id を別 id に偽装してもパス指定 execution_id と
    /// 一致しないと None になる。
    #[tokio::test]
    async fn resolve_worktree_by_execution_rejects_metadata_with_mismatched_execution_id() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let attacker_uuid = uuid::Uuid::new_v4().to_string();
        let executions_dir = executions_dir(tmp.path());
        fs::create_dir_all(&executions_dir).unwrap();
        // metadata 内の execution_id が path の execution_id と一致しないファイルを置く
        let other_uuid = uuid::Uuid::new_v4().to_string();
        let metadata = make_execution(&other_uuid, "/wt/forged", ExecutionStatus::Completed, 1.0);
        let path = execution_file_path(tmp.path(), &attacker_uuid);
        fs::write(&path, serde_json::to_string(&metadata).unwrap()).unwrap();

        assert_eq!(
            store.resolve_worktree_by_execution(&attacker_uuid).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_worktree_by_execution_returns_none_for_unknown_execution_id() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        assert_eq!(store.resolve_worktree_by_execution("missing").await, None);
    }

    /// G4: 永続化失敗時に in-memory state が rollback されること。
    /// `data_dir` を読み取り専用ファイルにすることで `persist_metadata` を強制失敗させ、
    /// `register_active_execution` が `Err(PersistFailed)` を返し active set / by_worktree が空になる
    /// ことを検証する。
    #[tokio::test]
    async fn register_active_rolls_back_on_persist_failure() {
        let tmp = TempDir::new().unwrap();
        // data_dir に指定したパスを既存のファイルにしてしまうと、`executions_dir = path.join("workflow_executions")`
        // の create_dir_all がファイル衝突で失敗する。
        let data_dir = tmp.path().join("data");
        // data_dir 自体はファイルとして作る（mkdir できない状況を作る）
        fs::write(&data_dir, "not a dir").unwrap();

        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(data_dir.clone()).await;
        let result = store
            .register_active_execution(make_execution(
                &test_uuid(1),
                "/wt/x",
                ExecutionStatus::Running,
                100.0,
            ))
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::PersistFailed { .. })
        ));
        // rollback により active / by_worktree は空のまま
        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_execution_by_worktree("/wt/x").await, None);
    }

    /// Spec issues-1011 finding 4: ExecutionStore API 境界で execution_id UUID 検証が走る。
    /// 非 UUID 形式の execution_id は register_active_execution / update_active / complete_execution で
    /// `InvalidExecutionId` として拒否される（command 層への漏れを防ぐ二重防御）。
    #[tokio::test]
    async fn execution_store_api_boundary_rejects_non_uuid_execution_id() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // register_active_execution は非 UUID を拒否する
        let bad = make_execution("not-a-uuid", "/wt/x", ExecutionStatus::Running, 100.0);
        assert!(matches!(
            store.register_active_execution(bad).await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));
        assert_eq!(store.active_len().await, 0);

        // update_active も非 UUID を拒否する
        assert!(matches!(
            store.update_active("../etc/passwd", |_| {}).await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));

        // complete_execution も非 UUID を拒否する
        assert!(matches!(
            store
                .complete_execution("not-a-uuid", TerminalExecutionStatus::Completed, 1.0, None)
                .await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));
    }

    #[tokio::test]
    async fn complete_execution_removes_from_active_and_sets_terminal_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Failed,
                105.0,
                Some("boom".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_execution_by_worktree("/wt/a").await, None);

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, ExecutionStatus::Failed);
        assert_eq!(completed[0].error_reason.as_deref(), Some("boom"));
        assert_eq!(completed[0].completed_at, Some(105.0));
    }

    /// `complete_execution` の rollback で、競合がない場合は previous が active / by_worktree に
    /// 戻されることを検証する（既存挙動の回帰テスト）。
    ///
    /// `data_dir` を「ファイル」にすることで永続化を強制失敗させる。`register_active_execution` は
    /// data_dir 設定前に行うため、最初の登録は in-memory のみで成功する。
    #[tokio::test]
    async fn complete_execution_reinserts_previous_on_persist_failure_without_conflict() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(data_dir.clone()).await;

        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();

        // data_dir をファイルに差し替えて persist を強制失敗させる
        fs::remove_dir_all(&data_dir).unwrap();
        fs::write(&data_dir, "blocking").unwrap();

        let result = store
            .complete_execution(&execution_id, TerminalExecutionStatus::Failed, 200.0, None)
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::PersistFailed { .. })
        ));

        // 競合がないので rollback により active / by_worktree に previous が戻っている
        assert_eq!(store.active_len().await, 1);
        let resolved = store.resolve_execution_by_worktree("/wt/a").await;
        assert_eq!(resolved.as_deref(), Some(execution_id.as_str()));
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: 競合なしのケース。
    /// previous がそのまま `active` / `by_worktree` に再投入され、true を返す。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_succeeds_without_conflict() {
        let mut inner = ExecutionStoreInner::new();
        let previous = make_execution(&test_uuid(1), "/wt/a", ExecutionStatus::Running, 100.0);
        let prev_execution_id = previous.execution_id.clone();

        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(ok);
        assert_eq!(inner.active.len(), 1);
        assert!(inner.active.contains_key(&prev_execution_id));
        assert_eq!(
            inner.by_worktree.get("/wt/a").map(String::as_str),
            Some(prev_execution_id.as_str())
        );
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: by_worktree が別 execution_id に
    /// 取られているケース（concurrent register_active_execution が同一 worktree へ別 execution を割り当てた状況）。
    /// 再投入をスキップし false を返す。`active` / `by_worktree` の状態は変更されない。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_skips_on_worktree_conflict() {
        let mut inner = ExecutionStoreInner::new();
        // 競合状態を構築: 別 execution_id (run2) が同一 worktree に紐づいている
        let other_execution_id = test_uuid(2);
        let other_execution = make_execution(
            &other_execution_id,
            "/wt/shared",
            ExecutionStatus::Running,
            150.0,
        );
        inner
            .active
            .insert(other_execution_id.clone(), other_execution);
        inner
            .by_worktree
            .insert("/wt/shared".to_string(), other_execution_id.clone());

        // previous (run1) を再投入しようとしても、worktree が他の execution に占有されているため拒否
        let previous = make_execution(&test_uuid(1), "/wt/shared", ExecutionStatus::Running, 100.0);
        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(!ok);
        // 既存 (other_execution) のみが残る。previous は混入しない。
        assert_eq!(inner.active.len(), 1);
        assert!(inner.active.contains_key(&other_execution_id));
        assert_eq!(
            inner.by_worktree.get("/wt/shared").map(String::as_str),
            Some(other_execution_id.as_str())
        );
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: active に同一 execution_id が
    /// 既に存在するケース（理論上は起きにくいが防御的に拒否する）。
    /// 再投入をスキップし false を返す。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_skips_on_execution_id_conflict() {
        let mut inner = ExecutionStoreInner::new();
        let execution_id = test_uuid(1);
        // 既に同一 execution_id が active に存在する状況を構築
        let existing = make_execution(
            &execution_id,
            "/wt/elsewhere",
            ExecutionStatus::Running,
            150.0,
        );
        inner.active.insert(execution_id.clone(), existing);
        inner
            .by_worktree
            .insert("/wt/elsewhere".to_string(), execution_id.clone());

        // 同一 execution_id を別 worktree (/wt/a) で再投入しようとしても拒否
        let previous = make_execution(&execution_id, "/wt/a", ExecutionStatus::Running, 100.0);
        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(!ok);
        // 既存 entry はそのまま、/wt/a の by_worktree は作られない
        assert_eq!(inner.active.len(), 1);
        assert_eq!(
            inner
                .active
                .get(&execution_id)
                .map(|r| r.worktree_path.as_str()),
            Some("/wt/elsewhere")
        );
        assert_eq!(inner.by_worktree.get("/wt/a"), None);
    }

    #[tokio::test]
    async fn stale_active_projection_after_complete_does_not_overwrite_terminal_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                105.0,
                None,
            )
            .await
            .unwrap();

        store
            .sync_active_projection(
                &execution_id,
                ExecutionStatus::Running,
                Some("stale".to_string()),
                106.0,
            )
            .await
            .unwrap();

        assert!(store.list_active().await.unwrap().is_empty());
        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, ExecutionStatus::Completed);
        assert_eq!(completed[0].completed_at, Some(105.0));

        let saved: WorkflowExecutionMetadata = serde_json::from_str(
            &fs::read_to_string(execution_file_path(tmp.path(), &execution_id)).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.status, ExecutionStatus::Completed);
        assert_eq!(saved.completed_at, Some(105.0));
        assert_eq!(saved.current_node.as_deref(), Some("node-1"));
    }

    #[tokio::test]
    async fn active_and_completed_lists_expose_worktree_scoped_target_executions() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let target_active = test_uuid(1);
        let other_active = test_uuid(2);
        let target_done = test_uuid(3);
        let other_done = test_uuid(4);
        store
            .register_active_execution(make_execution(
                &target_done,
                "/wt/target",
                ExecutionStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(&target_done, TerminalExecutionStatus::Failed, 95.0, None)
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &other_active,
                "/wt/other",
                ExecutionStatus::Running,
                101.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &target_active,
                "/wt/target",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &other_done,
                "/wt/other-done",
                ExecutionStatus::Running,
                80.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(&other_done, TerminalExecutionStatus::Aborted, 85.0, None)
            .await
            .unwrap();

        let active_target: Vec<_> = store
            .list_active()
            .await
            .unwrap()
            .into_iter()
            .filter(|execution| execution.worktree_path == "/wt/target")
            .collect();
        let completed_target: Vec<_> = store
            .list_completed()
            .await
            .into_iter()
            .filter(|execution| execution.worktree_path == "/wt/target")
            .collect();

        assert_eq!(active_target.len(), 1);
        assert_eq!(active_target[0].execution_id, target_active);
        assert_eq!(completed_target.len(), 1);
        assert_eq!(completed_target[0].execution_id, target_done);
        assert_eq!(completed_target[0].status, ExecutionStatus::Failed);
    }

    /// Spec issues-1011 finding 9: `cancel_reservation` は active から外し、metadata ファイルも削除する。
    /// completed 一覧には現れない。
    #[tokio::test]
    async fn cancel_reservation_removes_active_and_metadata_file() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let path = execution_file_path(tmp.path(), &execution_id);
        assert!(path.exists());

        store.cancel_reservation(&execution_id).await.unwrap();

        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_execution_by_worktree("/wt/a").await, None);
        assert!(!path.exists(), "metadata file must be removed");
        // completed 一覧にも現れない（terminal entry を残さない）
        assert!(store.list_completed().await.is_empty());
    }

    /// active reservation は Running / WaitingApproval だけを受け付ける。
    #[tokio::test]
    async fn register_active_rejects_non_active_status() {
        let store = ExecutionStore::new_in_memory_for_tests();
        for terminal in [
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::Aborted,
            ExecutionStatus::Interrupted,
        ] {
            let execution = make_execution(&test_uuid(1), "/wt/a", terminal, 100.0);
            let err = store
                .register_active_execution(execution)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ExecutionStoreError::NonActiveStatusInActiveSet { status, .. } if status == terminal),
                "non-active status must be rejected, got: {err:?}"
            );
        }
        assert_eq!(store.active_len().await, 0);
    }

    /// Spec issues-1011 finding 10: `update_active` は execution_id を変更しようとした場合に拒否する。
    /// 違反時は in-memory state を rollback し、永続化に進まない。
    #[tokio::test]
    async fn update_active_rejects_execution_id_mutation_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let result = store
            .update_active(&execution_id, |r| {
                r.execution_id = test_uuid(2);
            })
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::ImmutableFieldChanged { ref field, .. }) if field == "execution_id"
        ));
        // rollback: 元の execution_id のままで active 維持
        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);
    }

    /// Spec issues-1011 finding 10: `update_active` は worktree_path 変更も拒否する。
    #[tokio::test]
    async fn update_active_rejects_worktree_path_mutation_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let result = store
            .update_active(&execution_id, |r| {
                r.worktree_path = "/wt/b".to_string();
            })
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::ImmutableFieldChanged { ref field, .. }) if field == "worktree_path"
        ));
        // by_worktree index は元の path を保持
        assert_eq!(
            store.resolve_execution_by_worktree("/wt/a").await,
            Some(execution_id.clone())
        );
        assert_eq!(store.resolve_execution_by_worktree("/wt/b").await, None);
    }

    /// `update_active` は finished / interrupted 遷移を拒否し、専用 transition API を要求する。
    #[tokio::test]
    async fn update_active_rejects_terminal_transition_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        for terminal in [
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::Aborted,
            ExecutionStatus::Interrupted,
        ] {
            let result = store
                .update_active(&execution_id, |r| {
                    r.status = terminal;
                })
                .await;
            assert!(
                matches!(result, Err(ExecutionStoreError::NonActiveNotAllowedInUpdate { .. })),
                "non-active transition via update_active must be rejected for {terminal:?}, got: {result:?}"
            );
            // active のまま
            let active = store.list_active().await.unwrap();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].status, ExecutionStatus::Running);
        }
    }

    /// Spec issues-1011 finding 9: `list_completed` は workflow_executions/ 配下の symlink を拒否する。
    /// 外部入力境界として resolve_worktree_by_execution と同等の検証レベルで揃える。
    #[cfg(unix)]
    #[tokio::test]
    async fn list_completed_rejects_symlink_entries() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // 正規 terminal execution を 1 件
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                101.0,
                None,
            )
            .await
            .unwrap();

        // 攻撃: workflow_executions/ 配下に別 path の metadata への symlink を置く
        let executions_dir = executions_dir(tmp.path());
        let outside = tmp.path().join("outside.json");
        let attacker = make_execution(
            "00000000-0000-0000-0000-0000000000ff",
            "/wt/forged",
            ExecutionStatus::Completed,
            50.0,
        );
        fs::write(&outside, serde_json::to_string(&attacker).unwrap()).unwrap();
        std::os::unix::fs::symlink(
            &outside,
            executions_dir.join("00000000-0000-0000-0000-0000000000ff.json"),
        )
        .unwrap();

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1, "symlink entry must be skipped");
        assert_eq!(completed[0].execution_id, execution_id);
    }

    #[tokio::test]
    async fn persist_metadata_does_not_use_predictable_tmp_path() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(10);

        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();

        let predictable_tmp =
            execution_file_path(tmp.path(), &execution_id).with_extension("json.tmp");
        assert!(!predictable_tmp.exists());
    }

    // ---- [05] read-only API: list_executions / get_execution ----

    /// Rule [05]: 外部 caller は execution_id を主語として workflow execution を観測できる
    /// （単一 execution の summary metadata を観測する）。get_execution は active / terminal の
    /// いずれであっても summary を返す。
    #[tokio::test]
    async fn get_execution_returns_summary_for_active_and_terminal_executions() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active_id = test_uuid(20);
        let done_id = test_uuid(21);
        store
            .register_active_execution(make_execution(
                &active_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &done_id,
                "/wt/b",
                ExecutionStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(&done_id, TerminalExecutionStatus::Completed, 95.0, None)
            .await
            .unwrap();

        let active_summary = store.get_execution(&active_id).await.unwrap();
        assert_eq!(active_summary.execution_id, active_id);
        assert_eq!(active_summary.status, ExecutionStatus::Running);
        assert_eq!(active_summary.worktree_path, "/wt/a");

        let terminal_summary = store.get_execution(&done_id).await.unwrap();
        assert_eq!(terminal_summary.execution_id, done_id);
        assert_eq!(terminal_summary.status, ExecutionStatus::Completed);
        assert_eq!(terminal_summary.completed_at, Some(95.0));
    }

    /// Rule [05]: 観測対象として存在しない execution_id は明示的に「該当 execution なし」として扱われる。
    #[tokio::test]
    async fn get_execution_returns_none_for_unknown_execution_id() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let result = store.get_execution(&test_uuid(99)).await;
        assert!(result.is_none());
    }

    /// Spec [05]: get_execution は path traversal 対策として非 UUID execution_id を拒否する。
    #[tokio::test]
    async fn get_execution_rejects_non_uuid_execution_id() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let result = store.get_execution("../etc/passwd").await;
        assert!(result.is_none());
    }

    /// Rule [05]: list_executions は active と terminal を統合し、active を先頭・以降は完了時刻
    /// 降順で返す。status filter で active のみ / terminal のみに絞り込める。
    #[tokio::test]
    async fn list_executions_returns_active_and_terminal_with_status_filter() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active_id = test_uuid(30);
        let done_id = test_uuid(31);
        store
            .register_active_execution(make_execution(
                &active_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &done_id,
                "/wt/b",
                ExecutionStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_execution(&done_id, TerminalExecutionStatus::Completed, 95.0, None)
            .await
            .unwrap();

        let all = store.list_executions(ExecutionListFilter::default()).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].execution_id, active_id);
        assert_eq!(all[1].execution_id, done_id);

        let active_only = store
            .list_executions(ExecutionListFilter {
                status: Some(ExecutionStatusFilter::Active),
                worktree_path: None,
            })
            .await;
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].execution_id, active_id);

        let terminal_only = store
            .list_executions(ExecutionListFilter {
                status: Some(ExecutionStatusFilter::Terminal),
                worktree_path: None,
            })
            .await;
        assert_eq!(terminal_only.len(), 1);
        assert_eq!(terminal_only[0].execution_id, done_id);
    }

    /// Rule [05]: list_executions の worktree filter は指定 worktree の execution のみを返す。
    #[tokio::test]
    async fn list_executions_with_worktree_filter_returns_matching_executions_only() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let execution_a = test_uuid(40);
        let execution_b = test_uuid(41);
        store
            .register_active_execution(make_execution(
                &execution_a,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &execution_b,
                "/wt/b",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();

        let filtered = store
            .list_executions(ExecutionListFilter {
                status: None,
                worktree_path: Some("/wt/a".to_string()),
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].execution_id, execution_a);
    }

    /// Spec issues-1011 finding 12: `complete_execution` は `TerminalExecutionStatus` のみを受け付ける。
    /// 型レベルで非 terminal status の受け渡しを禁止していることを `From` 経路で確認する。
    #[tokio::test]
    async fn terminal_execution_status_converts_to_corresponding_execution_status() {
        assert_eq!(
            ExecutionStatus::from(TerminalExecutionStatus::Completed),
            ExecutionStatus::Completed
        );
        assert_eq!(
            ExecutionStatus::from(TerminalExecutionStatus::Failed),
            ExecutionStatus::Failed
        );
        assert_eq!(
            ExecutionStatus::from(TerminalExecutionStatus::Aborted),
            ExecutionStatus::Aborted
        );
    }

    /// 起動時 recovery: active orphan と stale projection 修復対象の Interrupted を列挙する。
    /// terminal な metadata は混じらず、orphan candidate は active 状態だけに絞れる。
    #[tokio::test]
    async fn list_non_terminal_metadata_returns_only_non_terminal_executions_from_disk() {
        let tmp = TempDir::new().unwrap();
        // 前回プロセスが残した状態を、独立した ExecutionStore を経由して disk に書く。
        {
            let prev = ExecutionStore::new_in_memory_for_tests();
            prev.set_data_dir(tmp.path().to_path_buf()).await;
            prev.register_active_execution(make_execution(
                &test_uuid(1),
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
            prev.register_active_execution(make_execution(
                &test_uuid(2),
                "/wt/b",
                ExecutionStatus::WaitingApproval,
                101.0,
            ))
            .await
            .unwrap();
            prev.register_active_execution(make_execution(
                &test_uuid(3),
                "/wt/c",
                ExecutionStatus::Running,
                102.0,
            ))
            .await
            .unwrap();
            prev.complete_execution(
                &test_uuid(3),
                TerminalExecutionStatus::Completed,
                103.0,
                None,
            )
            .await
            .unwrap();
            prev.register_active_execution(make_execution(
                &test_uuid(4),
                "/wt/d",
                ExecutionStatus::Running,
                104.0,
            ))
            .await
            .unwrap();
            prev.interrupt_execution(
                &test_uuid(4),
                ExecutionInterruptionReason::Stop,
                Some("node-1".to_string()),
                105.0,
            )
            .await
            .unwrap();
        }

        // 起動直後を模擬: 別 ExecutionStore で同じ data_dir を見る（in-memory active は空）。
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let mut orphans = store.list_non_terminal_metadata().await;
        orphans.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));
        let ids: Vec<&str> = orphans.iter().map(|r| r.execution_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                test_uuid(1).as_str(),
                test_uuid(2).as_str(),
                test_uuid(4).as_str()
            ]
        );
        assert_eq!(orphans[2].status, ExecutionStatus::Interrupted);
        assert_eq!(
            orphans
                .iter()
                .filter(|execution| execution.status.is_active())
                .map(|execution| execution.execution_id.as_str())
                .collect::<Vec<_>>(),
            vec![test_uuid(1).as_str(), test_uuid(2).as_str()]
        );
    }

    #[tokio::test]
    async fn interrupt_execution_releases_active_reservation_and_persists_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(41);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/interrupted",
                ExecutionStatus::WaitingApproval,
                10.0,
            ))
            .await
            .unwrap();

        let checkpoint = store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Stop,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();

        assert_eq!(store.active_len().await, 0);
        assert_eq!(checkpoint.status, ExecutionStatus::Interrupted);
        assert_eq!(checkpoint.completed_at, None);
        assert_eq!(
            checkpoint.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(checkpoint.resume_from_node.as_deref(), Some("node-1"));
        assert_eq!(checkpoint.current_node, None);
        let persisted = store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted, checkpoint);
    }

    #[tokio::test]
    async fn resume_reservation_can_be_rolled_back_to_interrupted_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(42);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/resume",
                ExecutionStatus::Running,
                10.0,
            ))
            .await
            .unwrap();
        store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Crash,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();

        let reservation = store
            .reserve_interrupted_for_resume(&execution_id, 30.0)
            .await
            .unwrap();
        assert_eq!(store.active_len().await, 0);
        assert_eq!(reservation.running.status, ExecutionStatus::Running);
        assert_eq!(reservation.running.current_node.as_deref(), Some("node-1"));
        assert_eq!(reservation.running.interruption_reason, None);
        assert_eq!(
            store
                .get_execution_record(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Interrupted,
            "metadata remains Interrupted until ExecutionResumed is committed"
        );
        assert!(matches!(
            store
                .reserve_interrupted_for_abort(&execution_id, 31.0)
                .await,
            Err(ExecutionStoreError::TransitionInProgress { .. })
        ));

        store
            .rollback_resume_reservation(reservation)
            .await
            .unwrap();
        assert_eq!(store.active_len().await, 0);
        let restored = store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.status, ExecutionStatus::Interrupted);
        assert_eq!(
            restored.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(restored.resume_from_node.as_deref(), Some("node-1"));
    }

    #[tokio::test]
    async fn resume_reservation_commits_running_only_after_the_event_boundary() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(47);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/resume-commit",
                ExecutionStatus::Running,
                10.0,
            ))
            .await
            .unwrap();
        store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Crash,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();

        let reservation = store
            .reserve_interrupted_for_resume(&execution_id, 30.0)
            .await
            .unwrap();
        let running = store.commit_resume_reservation(&reservation).await.unwrap();

        assert_eq!(running.status, ExecutionStatus::Running);
        assert_eq!(store.active_len().await, 1);
        assert_eq!(
            store
                .get_execution_record(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Running
        );
    }

    #[tokio::test]
    async fn resume_reservation_rejects_an_active_worktree_conflict() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let interrupted_id = test_uuid(43);
        let active_id = test_uuid(44);
        store
            .register_active_execution(make_execution(
                &interrupted_id,
                "/wt/shared",
                ExecutionStatus::Running,
                10.0,
            ))
            .await
            .unwrap();
        store
            .interrupt_execution(
                &interrupted_id,
                ExecutionInterruptionReason::Stale,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();
        store
            .register_active_execution(make_execution(
                &active_id,
                "/wt/shared",
                ExecutionStatus::Running,
                25.0,
            ))
            .await
            .unwrap();

        assert!(matches!(
            store
                .reserve_interrupted_for_resume(&interrupted_id, 30.0)
                .await,
            Err(ExecutionStoreError::WorktreeAlreadyActive {
                existing_execution_id,
                ..
            }) if existing_execution_id == active_id
        ));
    }

    #[tokio::test]
    async fn interrupted_abort_reservation_commits_finished_status() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(45);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/abort-interrupted",
                ExecutionStatus::Running,
                10.0,
            ))
            .await
            .unwrap();
        store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Stop,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();

        let reservation = store
            .reserve_interrupted_for_abort(&execution_id, 30.0)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_execution_record(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Interrupted,
            "event append 前の reservation は persisted metadata を変更しない"
        );
        assert!(matches!(
            store
                .reserve_interrupted_for_resume(&execution_id, 31.0)
                .await,
            Err(ExecutionStoreError::TransitionInProgress { .. })
        ));
        let aborted = store.commit_interrupted_abort(reservation).await.unwrap();

        assert_eq!(aborted.status, ExecutionStatus::Aborted);
        assert_eq!(aborted.completed_at, Some(30.0));
        assert_eq!(aborted.interruption_reason, None);
        assert_eq!(aborted.resume_from_node, None);
        assert_eq!(
            store
                .get_execution_record(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Aborted
        );
    }

    #[tokio::test]
    async fn interrupted_abort_reservation_rolls_back_when_event_append_fails() {
        let tmp = TempDir::new().unwrap();
        let store = ExecutionStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let execution_id = test_uuid(46);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/abort-rollback",
                ExecutionStatus::Running,
                10.0,
            ))
            .await
            .unwrap();
        store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Orphan,
                Some("node-1".to_string()),
                20.0,
            )
            .await
            .unwrap();

        let reservation = store
            .reserve_interrupted_for_abort(&execution_id, 30.0)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_execution_record(&execution_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Interrupted,
            "event append 失敗前の metadata は rollback 不要な Interrupted のまま"
        );
        store.rollback_interrupted_abort(reservation).await.unwrap();

        let restored = store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.status, ExecutionStatus::Interrupted);
        assert_eq!(restored.completed_at, None);
        assert_eq!(
            restored.interruption_reason,
            Some(ExecutionInterruptionReason::Orphan)
        );
        assert_eq!(restored.resume_from_node.as_deref(), Some("node-1"));

        // rollback releases the compare-and-transition guard.
        let reservation = store
            .reserve_interrupted_for_abort(&execution_id, 40.0)
            .await
            .unwrap();
        store.commit_interrupted_abort(reservation).await.unwrap();
    }
}
