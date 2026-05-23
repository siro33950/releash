//! Run Store: workflow 実行（`WorkflowRun`）の active/completed 管理を担う。
//!
//! 役割:
//! - active な run を `run_id` キーの in-memory map で管理し、worktree_path → run_id の
//!   secondary index を提供する。
//! - run metadata を `workflow_runs/{run_id}.json` として永続化し、completed run の一覧を
//!   ファイルシステムから列挙できるようにする。
//! - 状態遷移ロジックは持たず、engine からの「開始通知」「終了通知」を受けて反映するのみ。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// `WorkflowRun` のライフサイクル状態。
///
/// 既存 `WorkflowExecutionState` (`Running` / `WaitingApproval` / `Completed` /
/// `Failed` / `Aborted`) と語彙を一致させる。`waiting_approval` の serde 表現も同じく
/// `waiting_approval` で統一する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

impl RunStatus {
    /// 終了状態（terminal）かどうか。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Aborted
        )
    }
}

/// `complete_run` への入力を terminal status のみに制約する型。
///
/// Spec issues-1011 finding 12: release build でも非 terminal status を `complete_run` に
/// 渡せないように型レベルで強制する。`From` で `RunStatus` への変換を提供する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRunStatus {
    Completed,
    Failed,
    Aborted,
}

impl From<TerminalRunStatus> for RunStatus {
    fn from(t: TerminalRunStatus) -> Self {
        match t {
            TerminalRunStatus::Completed => RunStatus::Completed,
            TerminalRunStatus::Failed => RunStatus::Failed,
            TerminalRunStatus::Aborted => RunStatus::Aborted,
        }
    }
}

/// workflow 起動経路（UI / CLI / remote / agent 等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    DesktopUi,
    Remote,
    Cli,
    Agent,
}

/// [05] `list_workflow_runs` の status filter。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusFilter {
    Active,
    Terminal,
}

/// [05] `list_workflow_runs` の filter 入力。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RunStatusFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// 1 回の workflow 実行インスタンス。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: RunStatus,
    pub worktree_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_name: Option<String>,
    pub trigger_source: TriggerSource,
    pub started_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

/// `WorkflowRun` の一覧用サマリ。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub workflow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: RunStatus,
    pub worktree_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_name: Option<String>,
    pub trigger_source: TriggerSource,
    pub started_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

impl From<&WorkflowRun> for WorkflowRunSummary {
    fn from(run: &WorkflowRun) -> Self {
        Self {
            run_id: run.run_id.clone(),
            workflow_name: run.workflow_name.clone(),
            task: run.task.clone(),
            status: run.status,
            worktree_path: run.worktree_path.clone(),
            current_node_name: run.current_node_name.clone(),
            trigger_source: run.trigger_source,
            started_at: run.started_at,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
            error_reason: run.error_reason.clone(),
        }
    }
}

/// Run metadata 永続化のサブディレクトリ名。
const RUNS_SUBDIR: &str = "workflow_runs";
const MAX_RUN_METADATA_BYTES: u64 = 256 * 1024;

fn runs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(RUNS_SUBDIR)
}

fn run_file_path(data_dir: &Path, run_id: &str) -> PathBuf {
    runs_dir(data_dir).join(format!("{run_id}.json"))
}

/// `run_id` を UUID として検証する。Run Store のすべての lookup/read 経路で path traversal
/// を防ぐ目的で利用する（Spec issues-1011: 信頼境界・run_id の形式検証）。
fn is_valid_run_id(run_id: &str) -> bool {
    uuid::Uuid::parse_str(run_id).is_ok()
}

/// `path` が `runs_dir` の直下にあり、ファイル名のステムが `run_id` と一致することを検証する
/// （canonicalize 後の prefix 一致 + metadata.run_id == 渡された run_id の二重検査）。
fn is_within_runs_dir(runs_dir: &Path, path: &Path) -> bool {
    let canonical_runs_dir = match fs::canonicalize(runs_dir) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let canonical_path = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    canonical_path
        .parent()
        .is_some_and(|parent| parent == canonical_runs_dir)
}

/// `workflow_runs/{run_id}.json` の検証済みローダ。Spec issues-1011 line 130:
/// 外部入力として、以下の条件を満たさないものは破損エントリとして扱う:
/// - ファイル名 stem が UUID 形式
/// - metadata.run_id がファイル名 stem と一致
///
/// list / reverse lookup の両経路でこの loader を共有することで、検証ロジックを 1 箇所に
/// 集約する（Spec issues-1011 finding 11: list_completed と resolve_worktree_by_run の検証
/// レベルの分散を解消）。
fn load_validated_run_file(path: &Path) -> Result<WorkflowRun, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "missing file stem".to_string())?;
    if !is_valid_run_id(stem) {
        return Err(format!("invalid run_id in filename: {stem}"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("stat: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err("metadata file must not be a symlink".to_string());
    }
    if metadata.len() > MAX_RUN_METADATA_BYTES {
        return Err(format!(
            "metadata file too large: {} bytes (max {MAX_RUN_METADATA_BYTES})",
            metadata.len()
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let run: WorkflowRun = serde_json::from_str(&text).map_err(|e| format!("deserialize: {e}"))?;
    if run.run_id != stem {
        return Err(format!(
            "metadata.run_id ({}) does not match filename stem ({stem})",
            run.run_id
        ));
    }
    Ok(run)
}

fn load_validated_metadata_entry(runs_dir: &Path, path: &Path) -> Result<WorkflowRun, String> {
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
    if !is_within_runs_dir(runs_dir, path) {
        return Err("metadata path is outside workflow_runs/".to_string());
    }
    load_validated_run_file(path)
}

/// [05] API / CLI 共通の projection helper。`Vec<WorkflowRun>` に filter（status /
/// worktree_path）を適用し、active を先頭・以降は完了時刻降順で並べた
/// `Vec<WorkflowRunSummary>` を返す。
///
/// `RunStore::list_runs`（API 経路）と CLI の file-direct 経路の双方が同じ projection
/// に揃うことで観測ロジックの divergence を防ぐ（spec [05] API / CLI の意味的等価性境界）。
pub fn project_runs_to_summaries(
    runs: Vec<WorkflowRun>,
    filter: &RunListFilter,
) -> Vec<WorkflowRunSummary> {
    let mut summaries: Vec<WorkflowRunSummary> = runs
        .into_iter()
        .filter(|run| match filter.status {
            Some(RunStatusFilter::Active) => !run.status.is_terminal(),
            Some(RunStatusFilter::Terminal) => run.status.is_terminal(),
            None => true,
        })
        .filter(|run| match filter.worktree_path.as_deref() {
            Some(wt) => run.worktree_path == wt,
            None => true,
        })
        .map(|run| WorkflowRunSummary::from(&run))
        .collect();
    sort_summaries_active_first(&mut summaries);
    summaries
}

/// `Vec<WorkflowRunSummary>` を「active を先頭・以降は completed_at（無ければ updated_at）
/// の降順」で並び替える。projection helper と RunStore::list_runs から共通で使う。
pub(crate) fn sort_summaries_active_first(summaries: &mut [WorkflowRunSummary]) {
    summaries.sort_by(|a, b| {
        let a_active = !a.status.is_terminal();
        let b_active = !b.status.is_terminal();
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

/// [05] CLI: `workflow_runs/` 配下の active 状態の run から、現在 running 中の
/// workflow_name の集合を導出する。CLI の `workflow list` で `is_running` を
/// 反映するために使用する（spec [05] Rule: 観測経路は API と CLI で等価な手段を提供する）。
///
/// API 側は engine の in-memory active map から `running_workflow_names()` で
/// 集合を取るが、CLI は file-direct で同等の集合を導出する。
pub fn running_workflow_names_from_metadata(data_dir: &Path) -> HashSet<String> {
    iter_valid_run_metadata(data_dir)
        .into_iter()
        .filter(|run| !run.status.is_terminal())
        .map(|run| run.workflow_name)
        .collect()
}

pub(crate) fn iter_valid_run_metadata(data_dir: &Path) -> Vec<WorkflowRun> {
    let runs_dir = runs_dir(data_dir);
    if !runs_dir.exists() {
        return Vec::new();
    }
    let entries = match fs::read_dir(&runs_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("RunStore: failed to read runs dir: {e}");
            return Vec::new();
        }
    };
    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_validated_metadata_entry(&runs_dir, &path) {
            Ok(run) => runs.push(run),
            Err(e) => {
                log::warn!(
                    "RunStore: skip corrupted run metadata at {}: {e}",
                    path.display()
                );
            }
        }
    }
    runs
}

/// Run Store の in-memory state。`active` と `by_worktree` を単一 Mutex で保護することで、
/// 重複チェックと挿入を原子的に行う（Spec Rule: 同一 worktree への並行登録不整合を防ぐ）。
struct RunStoreInner {
    active: HashMap<String, WorkflowRun>,
    by_worktree: HashMap<String, String>,
}

impl RunStoreInner {
    fn new() -> Self {
        Self {
            active: HashMap::new(),
            by_worktree: HashMap::new(),
        }
    }

    /// `run_id` をキーに `active` / `by_worktree` の両方から削除する補助関数。
    /// `by_worktree` の entry は `worktree_path` から逆引きするため、active から
    /// 取り出した `worktree_path` のみを対象に削除する。
    fn remove_run(&mut self, run_id: &str) -> Option<WorkflowRun> {
        let removed = self.active.remove(run_id)?;
        if self
            .by_worktree
            .get(&removed.worktree_path)
            .is_some_and(|id| id == run_id)
        {
            self.by_worktree.remove(&removed.worktree_path);
        }
        Some(removed)
    }

    /// `complete_run` / `update_active` の永続化失敗時の rollback で、`previous` スナップショットを
    /// `active` / `by_worktree` に再投入する。
    ///
    /// `complete_run` は `remove_run` 実行と persist の間で Mutex を解放するため、その間に同一
    /// `worktree_path` へ別 run が `register_active` で割り当てられる可能性がある。その状態で
    /// 無条件に再投入すると以下の不変条件が壊れる:
    /// - `active` 内に同一 `worktree_path` を持つ run が 2 件存在する
    /// - `by_worktree` と `active` の双方向整合が崩れる（`by_worktree` は 1 件のみ）
    ///
    /// そのため、再投入前に以下を検査し、いずれかが競合する場合は再投入をスキップして false を
    /// 返す。呼出側は warn ログを出して PersistFailed を返すことで、不変条件を保ったまま rollback
    /// を諦める（永続化失敗で失われた状態は呼出元の上位経路で対応する）。
    /// - `by_worktree[previous.worktree_path]` が `previous.run_id` 以外を指している
    /// - `active` に既に `previous.run_id` が存在する
    fn try_reinsert_after_persist_failure(&mut self, previous: WorkflowRun) -> bool {
        let worktree_conflict = self
            .by_worktree
            .get(&previous.worktree_path)
            .is_some_and(|id| id != &previous.run_id);
        let run_id_conflict = self.active.contains_key(&previous.run_id);
        if worktree_conflict || run_id_conflict {
            return false;
        }
        self.by_worktree
            .insert(previous.worktree_path.clone(), previous.run_id.clone());
        self.active.insert(previous.run_id.clone(), previous);
        true
    }
}

/// Run Store: active set + 永続化された run metadata の双方を管理する。
///
/// active な run は in-memory map（`active: HashMap<run_id, WorkflowRun>`）と
/// secondary index（`by_worktree: HashMap<worktree_path, run_id>`）として保持する。
/// 終了済み run は `workflow_runs/{run_id}.json` から列挙する。
pub struct RunStore {
    inner: Mutex<RunStoreInner>,
    data_dir: Mutex<Option<PathBuf>>,
    allow_in_memory_without_data_dir: bool,
}

#[derive(Clone)]
struct RunMetadataStore {
    data_dir: PathBuf,
}

impl RunMetadataStore {
    fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    async fn persist(&self, run: WorkflowRun) -> Result<(), String> {
        persist_metadata(self.data_dir.clone(), run).await
    }

    async fn remove(&self, run_id: String) -> Result<(), String> {
        remove_metadata_file(self.data_dir.clone(), run_id).await
    }

    async fn list_valid(&self) -> Vec<WorkflowRun> {
        let dir = self.data_dir.clone();
        match tokio::task::spawn_blocking(move || iter_valid_run_metadata(&dir)).await {
            Ok(runs) => runs,
            Err(e) => {
                log::warn!("RunStore: failed to join metadata listing task: {e}");
                Vec::new()
            }
        }
    }
}

impl Default for RunStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RunStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RunStoreInner::new()),
            data_dir: Mutex::new(None),
            allow_in_memory_without_data_dir: false,
        }
    }

    #[cfg(test)]
    pub fn new_in_memory_for_tests() -> Self {
        Self {
            inner: Mutex::new(RunStoreInner::new()),
            data_dir: Mutex::new(None),
            allow_in_memory_without_data_dir: true,
        }
    }

    /// データディレクトリを設定する。アプリ起動時の setup から 1 度だけ呼ぶ。
    pub async fn set_data_dir(&self, dir: PathBuf) {
        let mut guard = self.data_dir.lock().await;
        *guard = Some(dir);
    }

    async fn data_dir(&self) -> Option<PathBuf> {
        self.data_dir.lock().await.clone()
    }

    async fn persistence_dir(&self) -> Result<Option<PathBuf>, RunStoreError> {
        match self.data_dir().await {
            Some(dir) => Ok(Some(dir)),
            None if self.allow_in_memory_without_data_dir => Ok(None),
            None => Err(RunStoreError::DataDirNotConfigured),
        }
    }

    async fn metadata_store(&self) -> Result<Option<RunMetadataStore>, RunStoreError> {
        Ok(self.persistence_dir().await?.map(RunMetadataStore::new))
    }

    /// 新規 run を active として登録し、metadata を初期保存する。
    /// 既に同一 worktree に別 run_id の active run が存在する場合は `Err` を返す。
    /// 既に同一 run_id を別 worktree_path で登録しようとした場合も `Err` を返す
    /// （古い by_worktree index が孤立しないように同一 critical section で拒否する）。
    ///
    /// 重複チェックと active map 更新だけを Mutex 内で行い、metadata 永続化は
    /// `spawn_blocking` 経由で実行する。永続化失敗時は同一 snapshot の場合だけ rollback する。
    pub async fn register_active(&self, run: WorkflowRun) -> Result<(), RunStoreError> {
        if !is_valid_run_id(&run.run_id) {
            return Err(RunStoreError::InvalidRunId {
                run_id: run.run_id.clone(),
            });
        }
        // Spec issues-1011 finding 11: terminal status の active 登録を型レベル相当の
        // runtime guard で禁止する。active 集合の不変条件（is_active な run のみが
        // active に存在する）を API 境界で強制し、`update_active` の typed invariant と
        // 整合させる。
        if run.status.is_terminal() {
            return Err(RunStoreError::TerminalStatusInActiveSet {
                run_id: run.run_id.clone(),
                status: run.status,
            });
        }
        let metadata_store = self.metadata_store().await?;
        {
            let mut inner = self.inner.lock().await;
            // 同一 run_id が別 worktree で既に登録されている場合は不整合（古い by_worktree が
            // 孤立する原因）なので拒否する。
            if let Some(existing) = inner.active.get(&run.run_id) {
                if existing.worktree_path != run.worktree_path {
                    return Err(RunStoreError::RunIdWorktreeMismatch {
                        run_id: run.run_id.clone(),
                        existing_worktree_path: existing.worktree_path.clone(),
                        new_worktree_path: run.worktree_path.clone(),
                    });
                }
            }
            // 同一 worktree に別 run_id の active run があれば拒否する。
            if let Some(existing_run_id) = inner.by_worktree.get(&run.worktree_path) {
                if existing_run_id != &run.run_id {
                    return Err(RunStoreError::WorktreeAlreadyActive {
                        worktree_path: run.worktree_path.clone(),
                        existing_run_id: existing_run_id.clone(),
                    });
                }
            }
            inner
                .by_worktree
                .insert(run.worktree_path.clone(), run.run_id.clone());
            inner.active.insert(run.run_id.clone(), run.clone());
        }
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(run.clone()).await {
                let mut inner = self.inner.lock().await;
                if inner
                    .active
                    .get(&run.run_id)
                    .is_some_and(|active| active == &run)
                {
                    inner.remove_run(&run.run_id);
                }
                return Err(RunStoreError::PersistFailed {
                    run_id: run.run_id,
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// command rollback 専用: mutation 前の active snapshot を in-memory Run Store に戻す。
    ///
    /// 通常の `register_active` は metadata 永続化に失敗すると in-memory 挿入も取り消す。
    /// しかし command 受理サイクルの rollback では、失敗原因が Run Store 永続化先そのものの
    /// 障害である場合でも、少なくとも process 内の active projection は mutation 前 snapshot
    /// に戻す必要がある。metadata persist は best-effort として試み、失敗は Err で返すが、
    /// in-memory snapshot は保持する。
    pub(crate) async fn restore_active_snapshot_for_rollback(
        &self,
        run: WorkflowRun,
    ) -> Result<(), RunStoreError> {
        if !is_valid_run_id(&run.run_id) {
            return Err(RunStoreError::InvalidRunId {
                run_id: run.run_id.clone(),
            });
        }
        if run.status.is_terminal() {
            return Err(RunStoreError::TerminalStatusInActiveSet {
                run_id: run.run_id.clone(),
                status: run.status,
            });
        }
        let metadata_store = self.metadata_store().await?;
        {
            let mut inner = self.inner.lock().await;
            if let Some(existing) = inner.active.get(&run.run_id) {
                if existing.worktree_path != run.worktree_path {
                    return Err(RunStoreError::RunIdWorktreeMismatch {
                        run_id: run.run_id.clone(),
                        existing_worktree_path: existing.worktree_path.clone(),
                        new_worktree_path: run.worktree_path.clone(),
                    });
                }
            }
            if let Some(existing_run_id) = inner.by_worktree.get(&run.worktree_path) {
                if existing_run_id != &run.run_id {
                    return Err(RunStoreError::WorktreeAlreadyActive {
                        worktree_path: run.worktree_path.clone(),
                        existing_run_id: existing_run_id.clone(),
                    });
                }
            }
            inner
                .by_worktree
                .insert(run.worktree_path.clone(), run.run_id.clone());
            inner.active.insert(run.run_id.clone(), run.clone());
        }
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(run.clone()).await {
                return Err(RunStoreError::PersistFailed {
                    run_id: run.run_id,
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// active run の現在 node / status / updated_at を更新する（状態遷移ではない属性更新含む）。
    /// `mutator` は in-memory の run を直接書き換える。metadata 永続化は Mutex を解放してから
    /// `spawn_blocking` 経由で行い、永続化失敗時は同一 snapshot の場合だけ rollback する。
    ///
    /// 永続化失敗時は in-memory 側の変更を rollback して `Err` を返す。
    /// Spec issues-1011 finding 4: `run_id` は UUID 形式である必要がある。
    async fn update_active<F>(&self, run_id: &str, mutator: F) -> Result<(), RunStoreError>
    where
        F: FnOnce(&mut WorkflowRun),
    {
        if !is_valid_run_id(run_id) {
            return Err(RunStoreError::InvalidRunId {
                run_id: run_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        let (previous, updated) = {
            let mut inner = self.inner.lock().await;
            let Some(run) = inner.active.get_mut(run_id) else {
                // 対象が存在しない場合は no-op（呼出元の状態遷移後 race を許容する）。
                return Ok(());
            };
            let previous = run.clone();
            mutator(run);
            // Spec issues-1011 finding 10: typed invariant guard。
            // 呼出側が run_id / worktree_path / terminal status を変更しないことを mutation 後に
            // 必ず再検証する。違反時は in-memory state を rollback し、永続化に進まない。
            if run.run_id != previous.run_id {
                *run = previous.clone();
                return Err(RunStoreError::ImmutableFieldChanged {
                    run_id: previous.run_id,
                    field: "run_id".to_string(),
                });
            }
            if run.worktree_path != previous.worktree_path {
                *run = previous.clone();
                return Err(RunStoreError::ImmutableFieldChanged {
                    run_id: previous.run_id,
                    field: "worktree_path".to_string(),
                });
            }
            if run.status.is_terminal() {
                *run = previous.clone();
                return Err(RunStoreError::TerminalNotAllowedInUpdate {
                    run_id: previous.run_id,
                });
            }
            (previous, run.clone())
        };
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(updated.clone()).await {
                let mut inner = self.inner.lock().await;
                if let Some(run) = inner.active.get_mut(run_id) {
                    if *run == updated {
                        *run = previous;
                    }
                }
                return Err(RunStoreError::PersistFailed {
                    run_id: run_id.to_string(),
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// engine の active snapshot から Run Store の active projection を同期する。
    pub async fn sync_active_projection(
        &self,
        run_id: &str,
        status: RunStatus,
        current_node_name: Option<String>,
        updated_at: f64,
    ) -> Result<(), RunStoreError> {
        self.update_active(run_id, |run| {
            run.status = status;
            run.current_node_name = current_node_name;
            run.updated_at = updated_at;
        })
        .await
    }

    /// active run の現在値を rollback 用 snapshot として取得する。
    pub async fn active_run_snapshot(&self, run_id: &str) -> Option<WorkflowRun> {
        let inner = self.inner.lock().await;
        inner.active.get(run_id).cloned()
    }

    /// active run を terminal 状態に遷移させ、active set から除外する。metadata は更新して残す。
    /// in-memory mutation と persist を分離し、active map の Mutex を同期ファイル I/O 中に
    /// 保持しない（Spec issues-1011: Run Store 永続化責務分離）。
    /// 永続化失敗時は in-memory active set への再投入による rollback を試みる。
    /// Spec issues-1011 finding 4: `run_id` は UUID 形式である必要がある。
    /// Spec issues-1011 finding 12: terminal 制約は型レベル（`TerminalRunStatus`）で強制する。
    pub async fn complete_run(
        &self,
        run_id: &str,
        status: TerminalRunStatus,
        completed_at: f64,
        error_reason: Option<String>,
    ) -> Result<(), RunStoreError> {
        if !is_valid_run_id(run_id) {
            return Err(RunStoreError::InvalidRunId {
                run_id: run_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        let (previous, completed) = {
            let mut inner = self.inner.lock().await;
            let Some(run) = inner.remove_run(run_id) else {
                return Ok(());
            };
            let previous = run.clone();
            let mut completed = run;
            completed.status = status.into();
            completed.completed_at = Some(completed_at);
            completed.updated_at = completed_at;
            completed.error_reason = error_reason;
            (previous, completed)
        };
        if let Some(store) = metadata_store {
            if let Err(e) = store.persist(completed).await {
                // rollback: terminal 化を取り消し、active set / by_worktree に戻す。
                // lock 解放区間に同一 worktree_path / run_id へ別 run が register_active
                // されている場合は、再投入により不変条件（同一 worktree につき active は最大 1 件・
                // by_worktree と active の双方向整合）が壊れるため、競合検出時は再投入を諦める。
                let mut inner = self.inner.lock().await;
                let previous_run_id = previous.run_id.clone();
                if !inner.try_reinsert_after_persist_failure(previous) {
                    log::warn!(
                        "RunStore: skip rollback reinsertion for {previous_run_id} due to concurrent active conflict"
                    );
                }
                return Err(RunStoreError::PersistFailed {
                    run_id: run_id.to_string(),
                    reason: e,
                });
            }
        }
        Ok(())
    }

    /// active set から該当 run を取り除き、永続化された metadata ファイルも削除する。
    /// `complete_run` と異なり terminal metadata は残さず、reservation 状態を完全に
    /// 撤回する用途で使う（Spec issues-1011 finding 9: start_workflow の rollback で
    /// 失敗した reservation を撤回するため、terminal entry を completed 一覧に残さない）。
    pub async fn cancel_reservation(&self, run_id: &str) -> Result<(), RunStoreError> {
        if !is_valid_run_id(run_id) {
            return Err(RunStoreError::InvalidRunId {
                run_id: run_id.to_string(),
            });
        }
        let metadata_store = self.metadata_store().await?;
        // Spec issues-1011 finding 7: metadata file 削除を先に試み、成功後にのみ
        // in-memory active / by_worktree を消す。remove_file 失敗時は active reservation を
        // 維持したまま Err を返し、孤立した metadata を残さない（呼出側の fallback
        // complete_run が対象を引けるようにする）。
        if let Some(store) = metadata_store {
            if let Err(e) = store.remove(run_id.to_string()).await {
                log::warn!("RunStore: failed to remove reservation metadata for {run_id}: {e}");
                return Err(RunStoreError::PersistFailed {
                    run_id: run_id.to_string(),
                    reason: e,
                });
            }
        }
        let mut inner = self.inner.lock().await;
        inner.remove_run(run_id);
        Ok(())
    }

    /// active な run を一覧する（`WorkflowRunSummary` で返す）。
    pub async fn list_active(&self) -> Vec<WorkflowRunSummary> {
        let inner = self.inner.lock().await;
        let mut runs: Vec<WorkflowRunSummary> = inner
            .active
            .values()
            .map(WorkflowRunSummary::from)
            .collect();
        runs.sort_by(|a, b| {
            b.started_at
                .partial_cmp(&a.started_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        runs
    }

    /// 終了済み（completed / failed / aborted）の run を一覧する。
    /// 検証済み loader（`load_validated_run_file`）を経由するため、ファイル名 stem が
    /// UUID 形式でない、もしくは metadata.run_id が一致しないエントリは破損として
    /// warn ログのうえスキップする（Spec issues-1011 line 130 / Rule 7 / finding 11）。
    ///
    /// 本メソッドは観測経路の primary entry ではない（spec [05]: production 観測は
    /// `list_runs` に集約され `project_runs_to_summaries` を経由する）。テストでの
    /// metadata file 直読を検証するための補助 API として温存する。
    #[allow(dead_code)]
    pub async fn list_completed(&self) -> Vec<WorkflowRunSummary> {
        let Some(store) = self.metadata_store().await.ok().flatten() else {
            return Vec::new();
        };
        let active_ids: std::collections::HashSet<String> = {
            let inner = self.inner.lock().await;
            inner.active.keys().cloned().collect()
        };
        let metadata_runs = store.list_valid().await;
        let mut summaries: Vec<WorkflowRunSummary> = metadata_runs
            .into_iter()
            .filter(|run| run.status.is_terminal() && !active_ids.contains(&run.run_id))
            .map(|run| WorkflowRunSummary::from(&run))
            .collect();
        summaries.sort_by(|a, b| {
            let a_key = a.completed_at.unwrap_or(a.started_at);
            let b_key = b.completed_at.unwrap_or(b.started_at);
            b_key
                .partial_cmp(&a_key)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        summaries
    }

    /// テスト専用: worktree_path 限定の active+terminal 一覧（合成順）を返す。
    /// production 経路は `list_runs(RunListFilter { worktree_path: Some(..), .. })` を使う。
    #[cfg(test)]
    pub async fn list_for_worktree(&self, worktree_path: &str) -> Vec<WorkflowRunSummary> {
        self.list_runs(RunListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })
        .await
    }

    /// [05] read-only API: active / terminal を含む全 run summary を、optional な
    /// status / worktree filter を適用して返す。filter なしの場合は全件を返す。
    /// 並び順は active を先頭・以降は完了時刻降順とする。
    ///
    /// 観測経路は file metadata（`workflow_runs/{run_id}.json`）全件を一次 source とし、
    /// in-memory active map に存在する run はその snapshot で de-dupe / 上書きする
    /// （in-memory 側が状態遷移時点で先行するため）。最終的に CLI と同じ
    /// `project_runs_to_summaries` を経由することで観測ロジックの divergence を防ぐ
    /// （spec [05] API / CLI の意味的等価性境界, 観測値の整合性境界: list / get の
    /// データソース統一）。
    pub async fn list_runs(&self, filter: RunListFilter) -> Vec<WorkflowRunSummary> {
        let active_runs: HashMap<String, WorkflowRun> = {
            let inner = self.inner.lock().await;
            inner.active.clone()
        };
        let file_runs = match self.metadata_store().await {
            Ok(Some(store)) => store.list_valid().await,
            _ => Vec::new(),
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut combined: Vec<WorkflowRun> =
            Vec::with_capacity(file_runs.len() + active_runs.len());
        for run in active_runs.values() {
            if seen.insert(run.run_id.clone()) {
                combined.push(run.clone());
            }
        }
        for run in file_runs {
            if seen.insert(run.run_id.clone()) {
                combined.push(run);
            }
        }
        project_runs_to_summaries(combined, &filter)
    }

    /// [05] read-only API: 単一 run の summary を取得する。
    /// active map → terminal metadata file の順で lookup する。`run_id` は UUID 形式
    /// として検証する（path traversal 対策）。
    pub async fn get_run(&self, run_id: &str) -> Option<WorkflowRunSummary> {
        self.get_run_record(run_id)
            .await
            .map(|run| WorkflowRunSummary::from(&run))
    }

    pub(crate) async fn get_run_record(&self, run_id: &str) -> Option<WorkflowRun> {
        {
            let inner = self.inner.lock().await;
            if let Some(run) = inner.active.get(run_id) {
                return Some(run.clone());
            }
        }
        if !is_valid_run_id(run_id) {
            log::warn!("RunStore: rejected non-UUID run_id in get_run");
            return None;
        }
        let dir = self.data_dir().await?;
        let path = run_file_path(&dir, run_id);
        if !path.exists() {
            return None;
        }
        match load_validated_metadata_entry(&runs_dir(&dir), &path) {
            Ok(run) => Some(run),
            Err(e) => {
                log::warn!(
                    "RunStore: failed to load run metadata at {} for get_run: {e}",
                    path.display()
                );
                None
            }
        }
    }

    /// worktree_path から active な run_id を解決する。
    pub async fn resolve_run_by_worktree(&self, worktree_path: &str) -> Option<String> {
        let inner = self.inner.lock().await;
        inner.by_worktree.get(worktree_path).cloned()
    }

    /// run_id から worktree_path を解決する。
    /// active な run のみならず、終了済み run も `workflow_runs/{run_id}.json` から
    /// metadata を読み込んで返す（Spec Rule 4: 実行インスタンスから worktree を解決する
    /// 対象は active に限定されない）。
    ///
    /// path traversal 対策として、ディスクへフォールバックする場合のみ `run_id` を
    /// UUID として検証し、解決後のパスが `workflow_runs/` 直下にあり、metadata 内の
    /// `run_id` フィールドが引数と一致することを二重検査する（Spec issues-1011: 信頼境界）。
    /// in-memory active map の lookup は外部入力を file system に渡さないため検証を要求しない。
    pub async fn resolve_worktree_by_run(&self, run_id: &str) -> Option<String> {
        {
            let inner = self.inner.lock().await;
            if let Some(run) = inner.active.get(run_id) {
                return Some(run.worktree_path.clone());
            }
        }
        // ディスクへフォールバックする経路のみ run_id を UUID として検証する。
        if !is_valid_run_id(run_id) {
            log::warn!("RunStore: rejected non-UUID run_id in resolve_worktree_by_run");
            return None;
        }
        let dir = self.data_dir().await?;
        let path = run_file_path(&dir, run_id);
        if !path.exists() {
            return None;
        }
        match load_validated_metadata_entry(&runs_dir(&dir), &path) {
            Ok(run) => Some(run.worktree_path),
            Err(e) => {
                log::warn!(
                    "RunStore: failed to load run metadata at {} for reverse lookup: {e}",
                    path.display()
                );
                None
            }
        }
    }

    /// テスト・engine 内部のリストア経路から、active run の attribute を直接設定する補助。
    /// 通常は `register_active` / `update_active` / `complete_run` を経由すること。
    #[cfg(test)]
    pub async fn active_len(&self) -> usize {
        self.inner.lock().await.active.len()
    }

    /// 起動時 recovery 用: disk 上 `workflow_runs/` 配下の non-terminal な metadata を列挙する。
    /// 前回起動中に terminal event が記録されないまま終了した run を引き当てるための一次 source。
    /// in-memory active map には依存しない（起動直後に呼ばれる前提）。
    pub async fn list_non_terminal_metadata(&self) -> Vec<WorkflowRun> {
        let Some(dir) = self.data_dir().await else {
            return Vec::new();
        };
        let runs = tokio::task::spawn_blocking(move || iter_valid_run_metadata(&dir))
            .await
            .unwrap_or_else(|e| {
                log::warn!("RunStore: failed to join non-terminal metadata listing task: {e}");
                Vec::new()
            });
        runs.into_iter()
            .filter(|run| !run.status.is_terminal())
            .collect()
    }

    /// 起動時 recovery 用: 指定 run の metadata を Aborted に書き換えて永続化する。
    /// `complete_run` と異なり in-memory active map には触れない（前回起動時の orphan は
    /// 当該プロセスの active map に存在しないため）。idempotent: 既に terminal な metadata に
    /// 対しても呼べるが、呼び出し側で `list_non_terminal_metadata` の結果のみを渡す想定。
    pub async fn force_complete_orphan_to_aborted(
        &self,
        mut run: WorkflowRun,
        completed_at: f64,
        error_reason: Option<String>,
    ) -> Result<(), RunStoreError> {
        let run_id_for_err = run.run_id.clone();
        let Some(store) = self.metadata_store().await? else {
            return Ok(());
        };
        run.status = RunStatus::Aborted;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        run.error_reason = error_reason;
        store
            .persist(run)
            .await
            .map_err(|reason| RunStoreError::PersistFailed {
                run_id: run_id_for_err,
                reason,
            })
    }
}

async fn persist_metadata(dir: PathBuf, run: WorkflowRun) -> Result<(), String> {
    tokio::task::spawn_blocking(move || persist_metadata_sync(&dir, &run))
        .await
        .map_err(|e| format!("metadata persist task failed: {e}"))?
}

/// metadata を `workflow_runs/{run_id}.json` に永続化する（同期 I/O）。
/// async RunStore API からは `spawn_blocking` 経由で呼び出し、active map の Mutex を
/// ファイル I/O 中に保持しない。
fn persist_metadata_sync(dir: &Path, run: &WorkflowRun) -> Result<(), String> {
    let runs_dir = runs_dir(dir);
    if let Err(e) = fs::create_dir_all(&runs_dir) {
        log::warn!("RunStore: failed to create runs dir: {e}");
        return Err(format!("create runs dir: {e}"));
    }
    let path = run_file_path(dir, &run.run_id);
    let json = match serde_json::to_string_pretty(run) {
        Ok(j) => j,
        Err(e) => {
            log::warn!("RunStore: failed to serialize run {}: {e}", run.run_id);
            return Err(format!("serialize: {e}"));
        }
    };
    if let Err(e) = atomic_write(&path, &json) {
        log::warn!("RunStore: failed to write {}: {e}", path.display());
        return Err(format!("write {}: {e}", path.display()));
    }
    Ok(())
}

async fn remove_metadata_file(dir: PathBuf, run_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = run_file_path(&dir, &run_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("metadata remove task failed: {e}"))?
}

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
pub enum RunStoreError {
    #[error("RunStore data_dir is not configured")]
    DataDirNotConfigured,
    #[error("worktree {worktree_path} already has active run {existing_run_id}")]
    WorktreeAlreadyActive {
        worktree_path: String,
        existing_run_id: String,
    },
    #[error(
        "run_id {run_id} is already active on worktree {existing_worktree_path} \
         and cannot be re-registered to {new_worktree_path}"
    )]
    RunIdWorktreeMismatch {
        run_id: String,
        existing_worktree_path: String,
        new_worktree_path: String,
    },
    #[error("failed to persist run {run_id} metadata: {reason}")]
    PersistFailed { run_id: String, reason: String },
    #[error("invalid run_id format (must be UUID): {run_id}")]
    InvalidRunId { run_id: String },
    #[error("cannot register run {run_id} into active set with terminal status: {status:?}")]
    TerminalStatusInActiveSet { run_id: String, status: RunStatus },
    #[error("update_active for {run_id} attempted to change immutable field {field}")]
    ImmutableFieldChanged { run_id: String, field: String },
    #[error("update_active for {run_id} cannot transition to terminal status; use complete_run")]
    TerminalNotAllowedInUpdate { run_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// テスト内で使う安定 UUID。register_active/update_active/complete_run の API 境界で
    /// UUID 形式検証が走るため、テスト識別子は UUID にする（Spec issues-1011 finding 4）。
    fn test_uuid(seed: u8) -> String {
        let bytes = [seed; 16];
        uuid::Uuid::from_bytes(bytes).to_string()
    }

    fn make_run(run_id: &str, worktree: &str, status: RunStatus, started_at: f64) -> WorkflowRun {
        WorkflowRun {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: Some("do thing".to_string()),
            status,
            worktree_path: worktree.to_string(),
            current_node_name: Some("node-1".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at,
            updated_at: started_at,
            completed_at: None,
            error_reason: None,
        }
    }

    /// Rule: workflow を 1 回起動するたびに、その実行は固有の識別子で記録される
    #[tokio::test]
    async fn register_active_records_run_with_unique_id_and_persists() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_id = test_uuid(1);
        let run = make_run(&run_id, "/wt/a", RunStatus::Running, 100.0);
        store.register_active(run.clone()).await.unwrap();

        assert_eq!(store.active_len().await, 1);
        let active = store.list_active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);

        let path = run_file_path(tmp.path(), &run_id);
        assert!(path.exists(), "metadata must be persisted at {path:?}");
    }

    #[tokio::test]
    async fn production_store_requires_data_dir_for_mutating_operations() {
        let store = RunStore::new();
        let run_id = test_uuid(1);
        let err = store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap_err();
        assert!(matches!(err, RunStoreError::DataDirNotConfigured));

        let err = store
            .complete_run(&run_id, TerminalRunStatus::Completed, 101.0, None)
            .await
            .unwrap_err();
        assert!(matches!(err, RunStoreError::DataDirNotConfigured));
    }

    /// Rule: worktree と実行インスタンスは双方向に解決できる
    #[tokio::test]
    async fn bidirectional_lookup_between_run_and_worktree() {
        let store = RunStore::new_in_memory_for_tests();
        let run_id = test_uuid(1);
        let run = make_run(&run_id, "/wt/a", RunStatus::Running, 100.0);
        store.register_active(run).await.unwrap();

        assert_eq!(
            store.resolve_run_by_worktree("/wt/a").await,
            Some(run_id.clone())
        );
        assert_eq!(
            store.resolve_worktree_by_run(&run_id).await,
            Some("/wt/a".to_string())
        );
    }

    /// Rule: 同一 worktree に進行中の実行が存在する間は、新たな workflow 起動は拒否される
    #[tokio::test]
    async fn second_active_run_on_same_worktree_is_rejected() {
        let store = RunStore::new_in_memory_for_tests();
        let run_id_1 = test_uuid(1);
        let run_id_2 = test_uuid(2);
        store
            .register_active(make_run(&run_id_1, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        let err = store
            .register_active(make_run(&run_id_2, "/wt/a", RunStatus::Running, 101.0))
            .await
            .unwrap_err();
        match err {
            RunStoreError::WorktreeAlreadyActive {
                existing_run_id, ..
            } => {
                assert_eq!(existing_run_id, run_id_1);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        // 既存 active が継続している
        assert_eq!(store.active_len().await, 1);
        assert_eq!(store.resolve_run_by_worktree("/wt/a").await, Some(run_id_1));
    }

    /// Spec issues-1011 finding 6: 同一 run_id を別 worktree_path で再登録しようとすると
    /// `RunIdWorktreeMismatch` で拒否される。古い by_worktree index が孤立しない。
    #[tokio::test]
    async fn register_active_rejects_same_run_id_with_different_worktree() {
        let store = RunStore::new_in_memory_for_tests();
        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        let err = store
            .register_active(make_run(&run_id, "/wt/b", RunStatus::Running, 101.0))
            .await
            .unwrap_err();
        match err {
            RunStoreError::RunIdWorktreeMismatch {
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
            store.resolve_run_by_worktree("/wt/a").await,
            Some(run_id.clone())
        );
        assert_eq!(store.resolve_run_by_worktree("/wt/b").await, None);
        assert_eq!(store.active_len().await, 1);
    }

    /// Rule: 進行中の実行と終了した実行を区別して一覧できる
    #[tokio::test]
    async fn list_active_and_completed_are_separated() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_active = test_uuid(10);
        let run_done = test_uuid(11);
        store
            .register_active(make_run(&run_active, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .register_active(make_run(&run_done, "/wt/b", RunStatus::Running, 90.0))
            .await
            .unwrap();
        store
            .complete_run(&run_done, TerminalRunStatus::Completed, 95.0, None)
            .await
            .unwrap();

        let active = store.list_active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_active);
        assert_eq!(active[0].workflow_name, "wf");
        assert_eq!(active[0].worktree_path, "/wt/a");
        assert_eq!(active[0].started_at, 100.0);
        assert_eq!(active[0].updated_at, 100.0);
        assert_eq!(active[0].trigger_source, TriggerSource::DesktopUi);
        assert_eq!(active[0].task.as_deref(), Some("do thing"));

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].run_id, run_done);
        assert_eq!(completed[0].workflow_name, "wf");
        assert_eq!(completed[0].worktree_path, "/wt/b");
        assert_eq!(completed[0].started_at, 90.0);
        assert_eq!(completed[0].updated_at, 95.0);
        assert_eq!(completed[0].trigger_source, TriggerSource::DesktopUi);
        assert_eq!(completed[0].task.as_deref(), Some("do thing"));
        assert_eq!(completed[0].status, RunStatus::Completed);
        assert_eq!(completed[0].completed_at, Some(95.0));

        let metadata: WorkflowRun = serde_json::from_str(
            &fs::read_to_string(run_file_path(tmp.path(), &run_done)).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.workflow_name, "wf");
        assert_eq!(metadata.worktree_path, "/wt/b");
        assert_eq!(metadata.started_at, 90.0);
        assert_eq!(metadata.updated_at, 95.0);
        assert_eq!(metadata.trigger_source, TriggerSource::DesktopUi);
        assert_eq!(metadata.task.as_deref(), Some("do thing"));
    }

    /// [05] list_runs / list_for_worktree は active を先頭、以降は完了時刻降順で
    /// 返す（spec [05] read-only API 並び順）。worktree filter で対象 worktree のみに
    /// 絞り込まれる。
    #[tokio::test]
    async fn list_for_worktree_combines_filters_and_sorts_runs() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active = test_uuid(30);
        let completed_new = test_uuid(31);
        let completed_other = test_uuid(32);
        store
            .register_active(make_run(&completed_new, "/wt/a", RunStatus::Running, 90.0))
            .await
            .unwrap();
        store
            .complete_run(&completed_new, TerminalRunStatus::Completed, 120.0, None)
            .await
            .unwrap();
        store
            .register_active(make_run(&active, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .register_active(make_run(
                &completed_other,
                "/wt/b",
                RunStatus::Running,
                110.0,
            ))
            .await
            .unwrap();
        store
            .complete_run(&completed_other, TerminalRunStatus::Completed, 130.0, None)
            .await
            .unwrap();

        let runs = store.list_for_worktree("/wt/a").await;
        let ids: Vec<_> = runs.iter().map(|run| run.run_id.as_str()).collect();
        // active が先頭、以降は完了時刻降順（spec [05] 並び順）。
        assert_eq!(ids, vec![active.as_str(), completed_new.as_str()]);
    }

    /// 終了 status は completed / failed / aborted の 3 つを含む
    #[tokio::test]
    async fn completed_listing_includes_all_terminal_statuses() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_c = test_uuid(20);
        let run_f = test_uuid(21);
        let run_a = test_uuid(22);
        for (id, status) in [
            (&run_c, TerminalRunStatus::Completed),
            (&run_f, TerminalRunStatus::Failed),
            (&run_a, TerminalRunStatus::Aborted),
        ] {
            store
                .register_active(make_run(
                    id,
                    &format!("/wt/{id}"),
                    RunStatus::Running,
                    100.0,
                ))
                .await
                .unwrap();
            store
                .complete_run(id, status, 101.0, Some("reason".to_string()))
                .await
                .unwrap();
        }

        let completed = store.list_completed().await;
        let ids: std::collections::HashSet<&str> =
            completed.iter().map(|r| r.run_id.as_str()).collect();
        assert!(ids.contains(run_c.as_str()));
        assert!(ids.contains(run_f.as_str()));
        assert!(ids.contains(run_a.as_str()));
    }

    /// Rule: 既に進行している worktree 上の実行は、新たな識別子を採番せずそのまま実行インスタンスとして扱われる
    ///
    /// Run Store は採番しないことを確認する。同じ run_id で再登録すれば（engine 側で
    /// `execution_id` を昇格させる経路に相当）通る。
    #[tokio::test]
    async fn register_with_same_run_id_for_same_worktree_is_idempotent() {
        let store = RunStore::new_in_memory_for_tests();
        let run = make_run(&test_uuid(1), "/wt/a", RunStatus::Running, 100.0);
        store.register_active(run.clone()).await.unwrap();
        // 同一 run_id の再登録は許容（idempotent）
        store.register_active(run).await.unwrap();
        assert_eq!(store.active_len().await, 1);
    }

    /// Rule: 永続化された実行 metadata の一部が破損していても、実行インスタンスの一覧は継続して提供される
    #[tokio::test]
    async fn list_completed_skips_corrupted_entries() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_ok = test_uuid(1);
        store
            .register_active(make_run(&run_ok, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .complete_run(&run_ok, TerminalRunStatus::Completed, 101.0, None)
            .await
            .unwrap();

        // 破損ファイル: JSON でない
        let runs_dir = runs_dir(tmp.path());
        fs::write(runs_dir.join("broken.json"), "not a json").unwrap();
        // 破損ファイル: ファイル名 stem が UUID でない
        fs::write(
            runs_dir.join("not-a-uuid.json"),
            serde_json::to_string(&make_run(
                "00000000-0000-0000-0000-000000000099",
                "/wt/forged",
                RunStatus::Completed,
                1.0,
            ))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            runs_dir.join(format!("{}.json", test_uuid(8))),
            "x".repeat((MAX_RUN_METADATA_BYTES + 1) as usize),
        )
        .unwrap();
        // 破損ファイル: filename stem と metadata.run_id が不一致
        let mismatch_uuid_path = test_uuid(7);
        let mismatch_meta = make_run(
            "00000000-0000-0000-0000-000000000088",
            "/wt/mismatch",
            RunStatus::Completed,
            1.0,
        );
        fs::write(
            runs_dir.join(format!("{mismatch_uuid_path}.json")),
            serde_json::to_string(&mismatch_meta).unwrap(),
        )
        .unwrap();

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].run_id, run_ok);
    }

    #[tokio::test]
    async fn sync_active_projection_mutates_in_memory_and_persists() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .sync_active_projection(
                &run_id,
                RunStatus::WaitingApproval,
                Some("approval-step".to_string()),
                110.0,
            )
            .await
            .unwrap();

        let active = store.list_active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        assert_eq!(
            active[0].current_node_name,
            Some("approval-step".to_string())
        );

        let path = run_file_path(tmp.path(), &run_id);
        let saved: WorkflowRun = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.status, RunStatus::WaitingApproval);
        assert_eq!(saved.updated_at, 110.0);
    }

    /// Rule 4: 終了済み run でも実行インスタンスから worktree を解決できる。
    /// active から外れて metadata だけが残る状況で reverse lookup が機能することを保証する。
    /// path traversal 対策で disk fallback は UUID 形式のみ許容するため、UUID を使う。
    #[tokio::test]
    async fn resolve_worktree_by_run_falls_back_to_persisted_metadata_for_terminal_runs() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let run_id = uuid::Uuid::new_v4().to_string();

        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .complete_run(&run_id, TerminalRunStatus::Completed, 105.0, None)
            .await
            .unwrap();

        // active からは消えているが、永続化済み metadata から解決できる
        assert_eq!(store.resolve_run_by_worktree("/wt/a").await, None);
        assert_eq!(
            store.resolve_worktree_by_run(&run_id).await,
            Some("/wt/a".to_string())
        );
    }

    /// path traversal 対策: disk fallback の lookup では非 UUID の run_id を拒否する
    #[tokio::test]
    async fn resolve_worktree_by_run_rejects_non_uuid_run_id_on_disk_fallback() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // active には存在しない、disk にも存在しない id を投げる
        assert_eq!(store.resolve_worktree_by_run("../etc/passwd").await, None);
        assert_eq!(store.resolve_worktree_by_run("not-a-uuid").await, None);
    }

    /// Spec issues-1011: 同一 worktree への並行 `register_active` で active / by_worktree が
    /// 整合する。Mutex で重複チェックと挿入を 1 critical section に閉じているので、
    /// レース後の状態は「ちょうど 1 つ active」かつ「by_worktree の entry が 1 つ」になる。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_active_on_same_worktree_keeps_active_and_by_worktree_consistent() {
        let store = std::sync::Arc::new(RunStore::new_in_memory_for_tests());
        let mut handles = Vec::new();
        // 8 並列で同一 worktree に異なる run_id (UUID) で register_active を試みる。
        for i in 0..8 {
            let store_cloned = std::sync::Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let run = make_run(
                    &test_uuid(i),
                    "/wt/race",
                    RunStatus::Running,
                    100.0 + i as f64,
                );
                store_cloned.register_active(run).await
            }));
        }
        let mut ok_count = 0usize;
        let mut conflict_count = 0usize;
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => ok_count += 1,
                Err(RunStoreError::WorktreeAlreadyActive { .. }) => conflict_count += 1,
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
        assert_eq!(
            ok_count, 1,
            "exactly one register_active must succeed under concurrent contention"
        );
        assert_eq!(conflict_count, 7);
        // 結果状態: active は 1 つだけ、by_worktree も 1 entry のみ。
        assert_eq!(store.active_len().await, 1);
        let resolved = store.resolve_run_by_worktree("/wt/race").await;
        assert!(resolved.is_some());
    }

    /// path traversal 対策: 攻撃者が metadata.run_id を別 id に偽装してもパス指定 run_id と
    /// 一致しないと None になる。
    #[tokio::test]
    async fn resolve_worktree_by_run_rejects_metadata_with_mismatched_run_id() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let attacker_uuid = uuid::Uuid::new_v4().to_string();
        let runs_dir = runs_dir(tmp.path());
        fs::create_dir_all(&runs_dir).unwrap();
        // metadata 内の run_id が path の run_id と一致しないファイルを置く
        let other_uuid = uuid::Uuid::new_v4().to_string();
        let metadata = make_run(&other_uuid, "/wt/forged", RunStatus::Completed, 1.0);
        let path = run_file_path(tmp.path(), &attacker_uuid);
        fs::write(&path, serde_json::to_string(&metadata).unwrap()).unwrap();

        assert_eq!(store.resolve_worktree_by_run(&attacker_uuid).await, None);
    }

    #[tokio::test]
    async fn resolve_worktree_by_run_returns_none_for_unknown_run_id() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        assert_eq!(store.resolve_worktree_by_run("missing").await, None);
    }

    /// G4: 永続化失敗時に in-memory state が rollback されること。
    /// `data_dir` を読み取り専用ファイルにすることで `persist_metadata` を強制失敗させ、
    /// `register_active` が `Err(PersistFailed)` を返し active set / by_worktree が空になる
    /// ことを検証する。
    #[tokio::test]
    async fn register_active_rolls_back_on_persist_failure() {
        let tmp = TempDir::new().unwrap();
        // data_dir に指定したパスを既存のファイルにしてしまうと、`runs_dir = path.join("workflow_runs")`
        // の create_dir_all がファイル衝突で失敗する。
        let data_dir = tmp.path().join("data");
        // data_dir 自体はファイルとして作る（mkdir できない状況を作る）
        fs::write(&data_dir, "not a dir").unwrap();

        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(data_dir.clone()).await;
        let result = store
            .register_active(make_run(&test_uuid(1), "/wt/x", RunStatus::Running, 100.0))
            .await;
        assert!(matches!(result, Err(RunStoreError::PersistFailed { .. })));
        // rollback により active / by_worktree は空のまま
        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_run_by_worktree("/wt/x").await, None);
    }

    /// Spec issues-1011 finding 4: RunStore API 境界で run_id UUID 検証が走る。
    /// 非 UUID 形式の run_id は register_active / update_active / complete_run で
    /// `InvalidRunId` として拒否される（command 層への漏れを防ぐ二重防御）。
    #[tokio::test]
    async fn run_store_api_boundary_rejects_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // register_active は非 UUID を拒否する
        let bad = make_run("not-a-uuid", "/wt/x", RunStatus::Running, 100.0);
        assert!(matches!(
            store.register_active(bad).await,
            Err(RunStoreError::InvalidRunId { .. })
        ));
        assert_eq!(store.active_len().await, 0);

        // update_active も非 UUID を拒否する
        assert!(matches!(
            store.update_active("../etc/passwd", |_| {}).await,
            Err(RunStoreError::InvalidRunId { .. })
        ));

        // complete_run も非 UUID を拒否する
        assert!(matches!(
            store
                .complete_run("not-a-uuid", TerminalRunStatus::Completed, 1.0, None)
                .await,
            Err(RunStoreError::InvalidRunId { .. })
        ));
    }

    #[tokio::test]
    async fn complete_run_removes_from_active_and_sets_terminal_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .complete_run(
                &run_id,
                TerminalRunStatus::Failed,
                105.0,
                Some("boom".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_run_by_worktree("/wt/a").await, None);

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, RunStatus::Failed);
        assert_eq!(completed[0].error_reason.as_deref(), Some("boom"));
        assert_eq!(completed[0].completed_at, Some(105.0));
    }

    /// `complete_run` の rollback で、競合がない場合は previous が active / by_worktree に
    /// 戻されることを検証する（既存挙動の回帰テスト）。
    ///
    /// `data_dir` を「ファイル」にすることで永続化を強制失敗させる。`register_active` は
    /// data_dir 設定前に行うため、最初の登録は in-memory のみで成功する。
    #[tokio::test]
    async fn complete_run_reinserts_previous_on_persist_failure_without_conflict() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(data_dir.clone()).await;

        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();

        // data_dir をファイルに差し替えて persist を強制失敗させる
        fs::remove_dir_all(&data_dir).unwrap();
        fs::write(&data_dir, "blocking").unwrap();

        let result = store
            .complete_run(&run_id, TerminalRunStatus::Failed, 200.0, None)
            .await;
        assert!(matches!(result, Err(RunStoreError::PersistFailed { .. })));

        // 競合がないので rollback により active / by_worktree に previous が戻っている
        assert_eq!(store.active_len().await, 1);
        let resolved = store.resolve_run_by_worktree("/wt/a").await;
        assert_eq!(resolved.as_deref(), Some(run_id.as_str()));
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: 競合なしのケース。
    /// previous がそのまま `active` / `by_worktree` に再投入され、true を返す。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_succeeds_without_conflict() {
        let mut inner = RunStoreInner::new();
        let previous = make_run(&test_uuid(1), "/wt/a", RunStatus::Running, 100.0);
        let prev_run_id = previous.run_id.clone();

        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(ok);
        assert_eq!(inner.active.len(), 1);
        assert!(inner.active.contains_key(&prev_run_id));
        assert_eq!(
            inner.by_worktree.get("/wt/a").map(String::as_str),
            Some(prev_run_id.as_str())
        );
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: by_worktree が別 run_id に
    /// 取られているケース（concurrent register_active が同一 worktree へ別 run を割り当てた状況）。
    /// 再投入をスキップし false を返す。`active` / `by_worktree` の状態は変更されない。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_skips_on_worktree_conflict() {
        let mut inner = RunStoreInner::new();
        // 競合状態を構築: 別 run_id (run2) が同一 worktree に紐づいている
        let other_run_id = test_uuid(2);
        let other_run = make_run(&other_run_id, "/wt/shared", RunStatus::Running, 150.0);
        inner.active.insert(other_run_id.clone(), other_run);
        inner
            .by_worktree
            .insert("/wt/shared".to_string(), other_run_id.clone());

        // previous (run1) を再投入しようとしても、worktree が他の run に占有されているため拒否
        let previous = make_run(&test_uuid(1), "/wt/shared", RunStatus::Running, 100.0);
        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(!ok);
        // 既存 (other_run) のみが残る。previous は混入しない。
        assert_eq!(inner.active.len(), 1);
        assert!(inner.active.contains_key(&other_run_id));
        assert_eq!(
            inner.by_worktree.get("/wt/shared").map(String::as_str),
            Some(other_run_id.as_str())
        );
    }

    /// `try_reinsert_after_persist_failure` の単体テスト: active に同一 run_id が
    /// 既に存在するケース（理論上は起きにくいが防御的に拒否する）。
    /// 再投入をスキップし false を返す。
    #[tokio::test]
    async fn try_reinsert_after_persist_failure_skips_on_run_id_conflict() {
        let mut inner = RunStoreInner::new();
        let run_id = test_uuid(1);
        // 既に同一 run_id が active に存在する状況を構築
        let existing = make_run(&run_id, "/wt/elsewhere", RunStatus::Running, 150.0);
        inner.active.insert(run_id.clone(), existing);
        inner
            .by_worktree
            .insert("/wt/elsewhere".to_string(), run_id.clone());

        // 同一 run_id を別 worktree (/wt/a) で再投入しようとしても拒否
        let previous = make_run(&run_id, "/wt/a", RunStatus::Running, 100.0);
        let ok = inner.try_reinsert_after_persist_failure(previous);

        assert!(!ok);
        // 既存 entry はそのまま、/wt/a の by_worktree は作られない
        assert_eq!(inner.active.len(), 1);
        assert_eq!(
            inner.active.get(&run_id).map(|r| r.worktree_path.as_str()),
            Some("/wt/elsewhere")
        );
        assert_eq!(inner.by_worktree.get("/wt/a"), None);
    }

    #[tokio::test]
    async fn stale_active_projection_after_complete_does_not_overwrite_terminal_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .complete_run(&run_id, TerminalRunStatus::Completed, 105.0, None)
            .await
            .unwrap();

        store
            .sync_active_projection(
                &run_id,
                RunStatus::Running,
                Some("stale".to_string()),
                106.0,
            )
            .await
            .unwrap();

        assert!(store.list_active().await.is_empty());
        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, RunStatus::Completed);
        assert_eq!(completed[0].completed_at, Some(105.0));

        let saved: WorkflowRun =
            serde_json::from_str(&fs::read_to_string(run_file_path(tmp.path(), &run_id)).unwrap())
                .unwrap();
        assert_eq!(saved.status, RunStatus::Completed);
        assert_eq!(saved.completed_at, Some(105.0));
        assert_eq!(saved.current_node_name.as_deref(), Some("node-1"));
    }

    #[tokio::test]
    async fn active_and_completed_lists_expose_worktree_scoped_target_runs() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let target_active = test_uuid(1);
        let other_active = test_uuid(2);
        let target_done = test_uuid(3);
        let other_done = test_uuid(4);
        store
            .register_active(make_run(
                &target_done,
                "/wt/target",
                RunStatus::Running,
                90.0,
            ))
            .await
            .unwrap();
        store
            .complete_run(&target_done, TerminalRunStatus::Failed, 95.0, None)
            .await
            .unwrap();
        store
            .register_active(make_run(
                &other_active,
                "/wt/other",
                RunStatus::Running,
                101.0,
            ))
            .await
            .unwrap();
        store
            .register_active(make_run(
                &target_active,
                "/wt/target",
                RunStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        store
            .register_active(make_run(
                &other_done,
                "/wt/other-done",
                RunStatus::Running,
                80.0,
            ))
            .await
            .unwrap();
        store
            .complete_run(&other_done, TerminalRunStatus::Aborted, 85.0, None)
            .await
            .unwrap();

        let active_target: Vec<_> = store
            .list_active()
            .await
            .into_iter()
            .filter(|run| run.worktree_path == "/wt/target")
            .collect();
        let completed_target: Vec<_> = store
            .list_completed()
            .await
            .into_iter()
            .filter(|run| run.worktree_path == "/wt/target")
            .collect();

        assert_eq!(active_target.len(), 1);
        assert_eq!(active_target[0].run_id, target_active);
        assert_eq!(completed_target.len(), 1);
        assert_eq!(completed_target[0].run_id, target_done);
        assert_eq!(completed_target[0].status, RunStatus::Failed);
    }

    /// Spec issues-1011 finding 9: `cancel_reservation` は active から外し、metadata ファイルも削除する。
    /// completed 一覧には現れない。
    #[tokio::test]
    async fn cancel_reservation_removes_active_and_metadata_file() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        let path = run_file_path(tmp.path(), &run_id);
        assert!(path.exists());

        store.cancel_reservation(&run_id).await.unwrap();

        assert_eq!(store.active_len().await, 0);
        assert_eq!(store.resolve_run_by_worktree("/wt/a").await, None);
        assert!(!path.exists(), "metadata file must be removed");
        // completed 一覧にも現れない（terminal entry を残さない）
        assert!(store.list_completed().await.is_empty());
    }

    /// Spec issues-1011 finding 11: `register_active` は terminal status の active 登録を
    /// 拒否する。`update_active` が terminal を許容しないこと（finding 10）と整合する。
    #[tokio::test]
    async fn register_active_rejects_terminal_status() {
        let store = RunStore::new_in_memory_for_tests();
        for terminal in [RunStatus::Completed, RunStatus::Failed, RunStatus::Aborted] {
            let run = make_run(&test_uuid(1), "/wt/a", terminal, 100.0);
            let err = store.register_active(run).await.unwrap_err();
            assert!(
                matches!(err, RunStoreError::TerminalStatusInActiveSet { status, .. } if status == terminal),
                "terminal status must be rejected, got: {err:?}"
            );
        }
        assert_eq!(store.active_len().await, 0);
    }

    /// Spec issues-1011 finding 10: `update_active` は run_id を変更しようとした場合に拒否する。
    /// 違反時は in-memory state を rollback し、永続化に進まない。
    #[tokio::test]
    async fn update_active_rejects_run_id_mutation_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        let result = store
            .update_active(&run_id, |r| {
                r.run_id = test_uuid(2);
            })
            .await;
        assert!(matches!(
            result,
            Err(RunStoreError::ImmutableFieldChanged { ref field, .. }) if field == "run_id"
        ));
        // rollback: 元の run_id のままで active 維持
        let active = store.list_active().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
    }

    /// Spec issues-1011 finding 10: `update_active` は worktree_path 変更も拒否する。
    #[tokio::test]
    async fn update_active_rejects_worktree_path_mutation_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        let result = store
            .update_active(&run_id, |r| {
                r.worktree_path = "/wt/b".to_string();
            })
            .await;
        assert!(matches!(
            result,
            Err(RunStoreError::ImmutableFieldChanged { ref field, .. }) if field == "worktree_path"
        ));
        // by_worktree index は元の path を保持
        assert_eq!(
            store.resolve_run_by_worktree("/wt/a").await,
            Some(run_id.clone())
        );
        assert_eq!(store.resolve_run_by_worktree("/wt/b").await, None);
    }

    /// Spec issues-1011 finding 10: `update_active` は terminal 遷移を拒否する
    /// （complete_run 専用経路を経由させる）。
    #[tokio::test]
    async fn update_active_rejects_terminal_transition_and_rolls_back() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        for terminal in [RunStatus::Completed, RunStatus::Failed, RunStatus::Aborted] {
            let result = store
                .update_active(&run_id, |r| {
                    r.status = terminal;
                })
                .await;
            assert!(
                matches!(result, Err(RunStoreError::TerminalNotAllowedInUpdate { .. })),
                "terminal transition via update_active must be rejected for {terminal:?}, got: {result:?}"
            );
            // active のまま
            let active = store.list_active().await;
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].status, RunStatus::Running);
        }
    }

    /// Spec issues-1011 finding 9: `list_completed` は workflow_runs/ 配下の symlink を拒否する。
    /// 外部入力境界として resolve_worktree_by_run と同等の検証レベルで揃える。
    #[cfg(unix)]
    #[tokio::test]
    async fn list_completed_rejects_symlink_entries() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        // 正規 terminal run を 1 件
        let run_id = test_uuid(1);
        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .complete_run(&run_id, TerminalRunStatus::Completed, 101.0, None)
            .await
            .unwrap();

        // 攻撃: workflow_runs/ 配下に別 path の metadata への symlink を置く
        let runs_dir = runs_dir(tmp.path());
        let outside = tmp.path().join("outside.json");
        let attacker = make_run(
            "00000000-0000-0000-0000-0000000000ff",
            "/wt/forged",
            RunStatus::Completed,
            50.0,
        );
        fs::write(&outside, serde_json::to_string(&attacker).unwrap()).unwrap();
        std::os::unix::fs::symlink(
            &outside,
            runs_dir.join("00000000-0000-0000-0000-0000000000ff.json"),
        )
        .unwrap();

        let completed = store.list_completed().await;
        assert_eq!(completed.len(), 1, "symlink entry must be skipped");
        assert_eq!(completed[0].run_id, run_id);
    }

    #[tokio::test]
    async fn persist_metadata_does_not_use_predictable_tmp_path() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let run_id = test_uuid(10);

        store
            .register_active(make_run(&run_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();

        let predictable_tmp = run_file_path(tmp.path(), &run_id).with_extension("json.tmp");
        assert!(!predictable_tmp.exists());
    }

    // ---- [05] read-only API: list_runs / get_run ----

    /// Rule [05]: 外部 caller は run_id を主語として workflow run を観測できる
    /// （単一 run の summary metadata を観測する）。get_run は active / terminal の
    /// いずれであっても summary を返す。
    #[tokio::test]
    async fn get_run_returns_summary_for_active_and_terminal_runs() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active_id = test_uuid(20);
        let done_id = test_uuid(21);
        store
            .register_active(make_run(&active_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .register_active(make_run(&done_id, "/wt/b", RunStatus::Running, 90.0))
            .await
            .unwrap();
        store
            .complete_run(&done_id, TerminalRunStatus::Completed, 95.0, None)
            .await
            .unwrap();

        let active_summary = store.get_run(&active_id).await.unwrap();
        assert_eq!(active_summary.run_id, active_id);
        assert_eq!(active_summary.status, RunStatus::Running);
        assert_eq!(active_summary.worktree_path, "/wt/a");

        let terminal_summary = store.get_run(&done_id).await.unwrap();
        assert_eq!(terminal_summary.run_id, done_id);
        assert_eq!(terminal_summary.status, RunStatus::Completed);
        assert_eq!(terminal_summary.completed_at, Some(95.0));
    }

    /// Rule [05]: 観測対象として存在しない run_id は明示的に「該当 run なし」として扱われる。
    #[tokio::test]
    async fn get_run_returns_none_for_unknown_run_id() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let result = store.get_run(&test_uuid(99)).await;
        assert!(result.is_none());
    }

    /// Spec [05]: get_run は path traversal 対策として非 UUID run_id を拒否する。
    #[tokio::test]
    async fn get_run_rejects_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let result = store.get_run("../etc/passwd").await;
        assert!(result.is_none());
    }

    /// Rule [05]: list_runs は active と terminal を統合し、active を先頭・以降は完了時刻
    /// 降順で返す。status filter で active のみ / terminal のみに絞り込める。
    #[tokio::test]
    async fn list_runs_returns_active_and_terminal_with_status_filter() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let active_id = test_uuid(30);
        let done_id = test_uuid(31);
        store
            .register_active(make_run(&active_id, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .register_active(make_run(&done_id, "/wt/b", RunStatus::Running, 90.0))
            .await
            .unwrap();
        store
            .complete_run(&done_id, TerminalRunStatus::Completed, 95.0, None)
            .await
            .unwrap();

        let all = store.list_runs(RunListFilter::default()).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].run_id, active_id);
        assert_eq!(all[1].run_id, done_id);

        let active_only = store
            .list_runs(RunListFilter {
                status: Some(RunStatusFilter::Active),
                worktree_path: None,
            })
            .await;
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].run_id, active_id);

        let terminal_only = store
            .list_runs(RunListFilter {
                status: Some(RunStatusFilter::Terminal),
                worktree_path: None,
            })
            .await;
        assert_eq!(terminal_only.len(), 1);
        assert_eq!(terminal_only[0].run_id, done_id);
    }

    /// Rule [05]: list_runs の worktree filter は指定 worktree の run のみを返す。
    #[tokio::test]
    async fn list_runs_with_worktree_filter_returns_matching_runs_only() {
        let tmp = TempDir::new().unwrap();
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;

        let run_a = test_uuid(40);
        let run_b = test_uuid(41);
        store
            .register_active(make_run(&run_a, "/wt/a", RunStatus::Running, 100.0))
            .await
            .unwrap();
        store
            .register_active(make_run(&run_b, "/wt/b", RunStatus::Running, 100.0))
            .await
            .unwrap();

        let filtered = store
            .list_runs(RunListFilter {
                status: None,
                worktree_path: Some("/wt/a".to_string()),
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].run_id, run_a);
    }

    /// Spec issues-1011 finding 12: `complete_run` は `TerminalRunStatus` のみを受け付ける。
    /// 型レベルで非 terminal status の受け渡しを禁止していることを `From` 経路で確認する。
    #[tokio::test]
    async fn terminal_run_status_converts_to_corresponding_run_status() {
        assert_eq!(
            RunStatus::from(TerminalRunStatus::Completed),
            RunStatus::Completed
        );
        assert_eq!(
            RunStatus::from(TerminalRunStatus::Failed),
            RunStatus::Failed
        );
        assert_eq!(
            RunStatus::from(TerminalRunStatus::Aborted),
            RunStatus::Aborted
        );
    }

    /// 起動時 recovery: 前回プロセスが書き残した non-terminal metadata のみを列挙する。
    /// terminal な metadata は混じらない。
    #[tokio::test]
    async fn list_non_terminal_metadata_returns_only_non_terminal_runs_from_disk() {
        let tmp = TempDir::new().unwrap();
        // 前回プロセスが残した状態を、独立した RunStore を経由して disk に書く。
        {
            let prev = RunStore::new_in_memory_for_tests();
            prev.set_data_dir(tmp.path().to_path_buf()).await;
            prev.register_active(make_run(&test_uuid(1), "/wt/a", RunStatus::Running, 100.0))
                .await
                .unwrap();
            prev.register_active(make_run(
                &test_uuid(2),
                "/wt/b",
                RunStatus::WaitingApproval,
                101.0,
            ))
            .await
            .unwrap();
            prev.register_active(make_run(&test_uuid(3), "/wt/c", RunStatus::Running, 102.0))
                .await
                .unwrap();
            prev.complete_run(&test_uuid(3), TerminalRunStatus::Completed, 103.0, None)
                .await
                .unwrap();
        }

        // 起動直後を模擬: 別 RunStore で同じ data_dir を見る（in-memory active は空）。
        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let mut orphans = store.list_non_terminal_metadata().await;
        orphans.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        let ids: Vec<&str> = orphans.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, vec![test_uuid(1).as_str(), test_uuid(2).as_str()]);
        assert!(orphans.iter().all(|r| !r.status.is_terminal()));
    }

    /// 起動時 recovery: `force_complete_orphan_to_aborted` が disk metadata を Aborted に
    /// 書き換え、completed_at と error_reason を反映する。
    #[tokio::test]
    async fn force_complete_orphan_to_aborted_persists_aborted_status() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(7);
        {
            let prev = RunStore::new_in_memory_for_tests();
            prev.set_data_dir(tmp.path().to_path_buf()).await;
            prev.register_active(make_run(&run_id, "/wt/x", RunStatus::Running, 100.0))
                .await
                .unwrap();
        }

        let store = RunStore::new_in_memory_for_tests();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let orphan = store
            .list_non_terminal_metadata()
            .await
            .into_iter()
            .next()
            .expect("orphan metadata must be present");
        store
            .force_complete_orphan_to_aborted(orphan, 200.0, None)
            .await
            .unwrap();

        let persisted: WorkflowRun =
            serde_json::from_str(&fs::read_to_string(run_file_path(tmp.path(), &run_id)).unwrap())
                .unwrap();
        assert_eq!(persisted.status, RunStatus::Aborted);
        assert_eq!(persisted.completed_at, Some(200.0));
        assert_eq!(persisted.updated_at, 200.0);
        assert!(persisted.error_reason.is_none());

        // recovery 完了後は list_non_terminal_metadata で再列挙されない（idempotent）。
        let remaining = store.list_non_terminal_metadata().await;
        assert!(remaining.is_empty(), "aborted run must not be re-listed");
    }
}
