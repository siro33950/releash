use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;

/// ワークフロー実行ログ（`WorkflowEvent` の NDJSON 書き込み・読み込み）。
///
/// 旧 `WorkflowLogEvent` 列挙体は [04] で `WorkflowEvent` に完全置換された。
/// 旧 NDJSON 在庫は破棄前提（互換 wrapper は導入しない）。
///
/// [04] spec 責務配置: 本モジュールは NDJSON の append/read のみを担う永続化アダプタ。
/// `WorkflowEvent` 列から `WorkflowState` への projection は `event_projection.rs` 側
/// (`reconstruct_state_from_events`) に置く。
pub struct WorkflowEventLog {
    log_dir: PathBuf,
}

static LOG_FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct CachedWorkflowLog {
    len: u64,
    modified: SystemTime,
    events: Arc<Vec<WorkflowEvent>>,
}

static LOG_FILE_CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedWorkflowLog>>> =
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
            log_dir: data_dir.join("workflow_logs"),
        }
    }

    fn log_path(&self, run_id: &str) -> PathBuf {
        self.log_dir.join(format!("{run_id}.ndjson"))
    }

    pub(crate) fn gc_delete_paths(&self, run_id: &str) -> Vec<PathBuf> {
        vec![
            self.log_path(run_id),
            self.log_dir.join(format!("{run_id}.json")),
        ]
    }

    /// イベントをNDJSON形式でログファイルに追記する。
    ///
    /// [08] production の単発 append は `write_log_required` → `append_batch` 経路に
    /// 集約された。本ヘルパは log 単体のセットアップが必要なテストからのみ呼ばれる。
    #[cfg(test)]
    pub fn append(&self, event: &WorkflowEvent) -> Result<(), String> {
        self.append_batch(std::slice::from_ref(event))
    }

    /// 複数 event を atomic な commit point として追記する。
    ///
    /// [04] spec『event 列と domain state の整合』Rule: 同一 command 受理サイクル内で
    /// 複数の required event を発行する必要がある場合（approval abort: ApprovalResolved +
    /// RunAborted など）、2 段の `append` だと 2 本目失敗時に 1 本目だけが NDJSON に
    /// 残り state rollback と event log が分裂する。本メソッドは serialize 結果を 1 本の
    /// バッファに連結したうえで、既存ログ + 追記分を同一ディレクトリの一時ファイルへ
    /// 書き出し、最後に rename する。既存ログファイルを直接変更しないため、write_all /
    /// sync / rename のどこで失敗しても command 受理前の NDJSON を破壊しない。
    ///
    /// 入力 event は全て同一 run_id に属する必要がある。空の入力は no-op。
    pub fn append_batch(&self, events: &[WorkflowEvent]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let run_id = events[0].run_id().to_string();
        validate_log_run_id(&run_id)?;
        for event in &events[1..] {
            if event.run_id() != run_id {
                return Err(format!(
                    "append_batch requires uniform run_id (got {} and {})",
                    run_id,
                    event.run_id()
                ));
            }
        }

        fs::create_dir_all(&self.log_dir).map_err(|e| format!("Failed to create log dir: {e}"))?;

        let mut buffer = String::new();
        for event in events {
            let json = serde_json::to_string(event)
                .map_err(|e| format!("Failed to serialize event: {e}"))?;
            buffer.push_str(&json);
            buffer.push('\n');
        }

        let path = self.log_path(&run_id);
        let lock = log_file_lock(&path);
        let _guard = lock.lock();
        let old_cache_key = workflow_log_cache_key(&path);
        let existing = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("Failed to read existing log file: {e}")),
        };
        let temp_path = self.create_temp_log_path(&path)?;
        let write_result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|e| format!("Failed to create temp log file: {e}"))?;
            file.write_all(&existing)
                .map_err(|e| format!("Failed to copy existing log into temp file: {e}"))?;
            file.write_all(buffer.as_bytes())
                .map_err(|e| format!("Failed to write log batch to temp file: {e}"))?;
            file.sync_all()
                .map_err(|e| format!("Failed to sync temp log file: {e}"))?;
            fs::rename(&temp_path, &path).map_err(|e| format!("Failed to commit log file: {e}"))?;
            Ok(())
        })();
        if let Err(e) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(e);
        }
        update_workflow_log_cache_after_append(&path, old_cache_key, existing.is_empty(), events);
        Ok(())
    }

    fn create_temp_log_path(&self, path: &Path) -> Result<PathBuf, String> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Failed to derive log temp file name".to_string())?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("System clock error while creating temp log path: {e}"))?
            .as_nanos();
        Ok(path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos)))
    }

    /// 指定された run_id の NDJSON ログを読み込み、イベント一覧を返す。
    pub fn read_log(&self, run_id: &str) -> Result<Vec<WorkflowEvent>, String> {
        validate_log_run_id(run_id)?;
        let path = self.log_path(run_id);
        if !path.exists() {
            return Ok(vec![]);
        }

        if let Some((len, modified)) = workflow_log_cache_key(&path) {
            if let Some(cached) = LOG_FILE_CACHE.lock().get(&path) {
                if cached.len == len && cached.modified == modified {
                    return Ok(cached.events.as_ref().clone());
                }
            }
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read log file: {e}"))?;
        let mut events = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event: WorkflowEvent =
                serde_json::from_str(line).map_err(|e| format!("Failed to parse log line: {e}"))?;
            events.push(event);
        }
        if let Some((len, modified)) = workflow_log_cache_key(&path) {
            LOG_FILE_CACHE.lock().insert(
                path,
                CachedWorkflowLog {
                    len,
                    modified,
                    events: Arc::new(events.clone()),
                },
            );
        }
        Ok(events)
    }

    /// 指定worktreeに属する run_id を返す（旧 `list_workflow_executions` Tauri command の
    /// バックエンド。production からは廃止済みだが、過去 NDJSON の worktree フィルタを
    /// 維持するため、テスト用補助メソッドとして温存する）。
    /// 各NDJSONファイルの1行目（RunStarted）のworktree_pathと照合する。
    #[cfg(test)]
    pub fn list_run_ids_for_worktree(&self, worktree_path: &str) -> Result<Vec<String>, String> {
        if !self.log_dir.exists() {
            return Ok(vec![]);
        }

        let mut ids = Vec::new();
        let entries =
            fs::read_dir(&self.log_dir).map_err(|e| format!("Failed to read log dir: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "ndjson") {
                continue;
            }
            let Some(stem) = path.file_stem() else {
                continue;
            };
            let run_id = stem.to_string_lossy().to_string();

            // 1行目を読んでworktree_pathを照合
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(first_line) = content.lines().next() {
                    if let Ok(WorkflowEvent::RunStarted {
                        worktree_path: wt, ..
                    }) = serde_json::from_str(first_line)
                    {
                        if wt == worktree_path {
                            ids.push(run_id);
                        }
                    }
                }
            }
        }
        Ok(ids)
    }
}

fn workflow_log_cache_key(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some((metadata.len(), modified))
}

fn update_workflow_log_cache_after_append(
    path: &Path,
    old_cache_key: Option<(u64, SystemTime)>,
    existing_was_empty: bool,
    appended: &[WorkflowEvent],
) {
    let Some((len, modified)) = workflow_log_cache_key(path) else {
        LOG_FILE_CACHE.lock().remove(path);
        return;
    };

    let mut cache = LOG_FILE_CACHE.lock();
    let next_events = match cache.get(path) {
        Some(cached)
            if old_cache_key.is_some_and(|(old_len, old_modified)| {
                cached.len == old_len && cached.modified == old_modified
            }) =>
        {
            let mut events = cached.events.as_ref().clone();
            events.extend_from_slice(appended);
            Some(events)
        }
        _ if existing_was_empty => Some(appended.to_vec()),
        _ => None,
    };

    if let Some(events) = next_events {
        cache.insert(
            path.to_path_buf(),
            CachedWorkflowLog {
                len,
                modified,
                events: Arc::new(events),
            },
        );
    } else {
        cache.remove(path);
    }
}

fn validate_log_run_id(run_id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(run_id).is_ok() {
        return Ok(());
    }
    Err("workflow log run_id must be UUID".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::{ApprovalDecisionRecord, CollectedOutputEntry};
    use crate::adaptor::gateway::workflow::event_projection::reconstruct_state_from_events;
    use crate::adaptor::gateway::workflow::schema::Workflow;
    use crate::adaptor::gateway::workflow::state::{TokenUsage, WorkflowExecutionState};
    use tempfile::TempDir;

    /// 旧 `WorkflowEventLog::reconstruct_state` テスト経路の代替ヘルパー。
    /// log.rs の責務縮退（[04]: NDJSON adapter に閉じる）に伴い、再構築は
    /// gateway の `event_projection::reconstruct_state_from_events` を経由する。テストの
    /// 表現を変えないために本ファイル内ヘルパーとして残す。
    ///
    /// [04] schema 境界: 復元用の `Workflow` は `RunStarted.workflow_definition` snapshot
    /// からのみ取り出す。本ヘルパーは workflow を引数で受け取らない。
    fn reconstruct_state_via_log(
        log: &WorkflowEventLog,
        run_id: &str,
    ) -> Result<Option<crate::adaptor::gateway::workflow::state::WorkflowState>, String> {
        let events = log.read_log(run_id)?;
        reconstruct_state_from_events(run_id, &events)
    }

    /// テスト用の最小 Workflow。
    fn minimal_workflow_for_log(name: &str) -> Workflow {
        use crate::adaptor::gateway::workflow::schema::{
            FacetRefs, NodeDefinition, NodeKind, SessionSpec,
        };
        Workflow {
            variables: Default::default(),
            name: name.to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "step1".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("do".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
        }
    }

    /// [04] 旧 NDJSON（旧 `workflow_started` 語彙、`execution_id` フィールド）は
    /// 新 `WorkflowEvent` deserialize で必ず失敗する（互換 wrapper を持たないことの担保）。
    #[test]
    fn legacy_ndjson_with_old_event_tag_fails_to_deserialize() {
        let legacy_line = r#"{"event":"workflow_started","execution_id":"old-1","workflow_name":"legacy","workflow_file_stem":"legacy","worktree_path":"/repo","workflow_definition":{"name":"legacy","description":"","builtin":false,"nodes":[]},"timestamp":1000.0}"#;
        let result: Result<WorkflowEvent, _> = serde_json::from_str(legacy_line);
        assert!(
            result.is_err(),
            "旧 event tag (workflow_started) は新 schema で必ず deserialize 失敗する"
        );
    }

    /// [02] schema 境界: 旧 NDJSON で `workflow_definition.steps` を含む WorkflowStarted は
    /// 新 schema（`workflow_definition.nodes` + `deny_unknown_fields`）として deserialize できない。
    #[test]
    fn old_ndjson_with_legacy_steps_shape_fails_to_deserialize() {
        let legacy_line = r#"{"event":"run_started","run_id":"old-1","workflow_name":"legacy","workflow_file_stem":"legacy","worktree_path":"/repo","workflow_definition":{"name":"legacy","description":"","builtin":false,"steps":[{"name":"x","mode":"auto","instruction":"x"}]},"timestamp":1000.0}"#;
        let result: Result<WorkflowEvent, _> = serde_json::from_str(legacy_line);
        assert!(
            result.is_err(),
            "旧 workflow_definition.steps を含む NDJSON は新 schema で deserialize 失敗する"
        );
    }

    /// 旧 `workflow_definition.steps` を含む NDJSON は listing / reconstruction の対象外となる。
    #[test]
    fn list_run_ids_excludes_legacy_steps_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        fs::create_dir_all(&log.log_dir).unwrap();
        let legacy_path = log.log_path("00000000-0000-0000-0000-000000000901");
        fs::write(
            &legacy_path,
            r#"{"event":"run_started","run_id":"00000000-0000-0000-0000-000000000901","workflow_name":"old","workflow_file_stem":"old","worktree_path":"/repo","workflow_definition":{"name":"old","description":"","builtin":false,"steps":[{"name":"x","mode":"auto","instruction":"x"}]},"timestamp":1000.0}"#,
        )
        .unwrap();
        // read_log は parse 失敗を返す（旧 shape は復元対象外）
        assert!(log
            .read_log("00000000-0000-0000-0000-000000000901")
            .is_err());
        // listing も除外（listing は parse 失敗 line をスキップする）
        let ids = log.list_run_ids_for_worktree("/repo").unwrap();
        assert!(!ids
            .iter()
            .any(|i| i == "00000000-0000-0000-0000-000000000901"));
    }

    /// 旧 NDJSON が listing から除外されること（read_log 失敗 ⇒ list_run_ids_for_worktree でも対象外）。
    #[test]
    fn list_run_ids_for_worktree_excludes_legacy_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        // worktree 一致する旧 NDJSON を直接書き込む（旧 event tag）
        fs::create_dir_all(&log.log_dir).unwrap();
        let legacy_path = log.log_path("legacy");
        fs::write(
            &legacy_path,
            r#"{"event":"workflow_started","execution_id":"legacy","workflow_name":"old","workflow_file_stem":"old","worktree_path":"/repo","timestamp":1000.0}"#,
        )
        .unwrap();

        // 新 NDJSON も同じ worktree で書き込む
        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000902".to_string(),
            workflow_name: "new".to_string(),
            workflow_file_stem: "new".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: minimal_workflow_for_log("new"),
            timestamp: 1001.0,
        })
        .unwrap();

        let ids = log.list_run_ids_for_worktree("/repo").unwrap();
        assert!(
            !ids.iter().any(|i| i == "legacy"),
            "旧 NDJSON は listing 対象から除外される"
        );
        assert!(ids
            .iter()
            .any(|i| i == "00000000-0000-0000-0000-000000000902"));
    }

    /// [04] atomic batch append: 複数 event を 1 回の write でまとめて append すれば、
    /// partial commit（最初の event のみ NDJSON に残る）を構造的に排除できることを
    /// 担保する。append_batch 経由で書き込んだ 2 件は read_log で順序通り 2 件として
    /// 読み出される。
    #[test]
    fn append_batch_writes_all_events_atomically_in_order() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000700";
        let events = vec![
            WorkflowEvent::ApprovalResolved {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                decision: ApprovalDecisionRecord::Abort,
                comment: None,
                timestamp: 1000.0,
            },
            WorkflowEvent::RunAborted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: None,
                timestamp: 1000.0,
            },
        ];
        log.append_batch(&events)
            .expect("batch append should succeed");
        let read_back = log.read_log(run_id).unwrap();
        assert_eq!(
            read_back.len(),
            2,
            "両方の event が atomic に append される"
        );
        assert!(matches!(
            read_back[0],
            WorkflowEvent::ApprovalResolved { .. }
        ));
        assert!(matches!(read_back[1], WorkflowEvent::RunAborted { .. }));
    }

    #[test]
    fn read_log_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000701";

        log.append(&WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Approve,
            comment: None,
            timestamp: 1.0,
        })
        .unwrap();
        assert_eq!(log.read_log(run_id).unwrap().len(), 1);

        log.append(&WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            aborted_step: None,
            timestamp: 2.0,
        })
        .unwrap();

        let read_back = log.read_log(run_id).unwrap();
        assert_eq!(read_back.len(), 2);
        assert!(matches!(
            read_back[0],
            WorkflowEvent::ApprovalResolved { timestamp: 1.0, .. }
        ));
        assert!(matches!(
            read_back[1],
            WorkflowEvent::RunAborted { timestamp: 2.0, .. }
        ));
    }

    /// [04] atomic batch append: 入力 event が異なる run_id を含む場合は append 前に
    /// Err を返し、partial 書き込みを起こさない（構造的不変条件）。
    #[test]
    fn append_batch_rejects_mixed_run_ids_before_writing() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let events = vec![
            WorkflowEvent::ApprovalResolved {
                run_id: "00000000-0000-0000-0000-000000000903".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                decision: ApprovalDecisionRecord::Approve,
                comment: None,
                timestamp: 1.0,
            },
            WorkflowEvent::RunAborted {
                run_id: "00000000-0000-0000-0000-000000000904".to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: None,
                timestamp: 2.0,
            },
        ];
        let err = log.append_batch(&events).unwrap_err();
        assert!(
            err.contains("uniform run_id"),
            "異 run_id 混在は uniform run_id エラーで拒否される: {err}"
        );
        // どちらの run_id の NDJSON も生成されていない
        assert!(log
            .read_log("00000000-0000-0000-0000-000000000903")
            .unwrap()
            .is_empty());
        assert!(log
            .read_log("00000000-0000-0000-0000-000000000904")
            .unwrap()
            .is_empty());
    }

    /// [04] atomic batch append: 空入力は no-op として Ok を返す（NDJSON は生成されない）。
    #[test]
    fn append_batch_with_empty_input_is_noop() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        log.append_batch(&[]).expect("empty input is no-op Ok");
        assert!(log
            .read_log("00000000-0000-0000-0000-000000000905")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn append_and_read_log() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());

        let event1 = WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: minimal_workflow_for_log("test"),
            timestamp: 1000.0,
        };
        let event2 = WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        };
        let event3 = WorkflowEvent::NodeCompleted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            result: Some("done".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
            structured_output: None,
            run_index: None,
            timestamp: 1002.0,
        };

        log.append(&event1).unwrap();
        log.append(&event2).unwrap();
        log.append(&event3).unwrap();

        let events = log
            .read_log("00000000-0000-0000-0000-000000000906")
            .unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn read_nonexistent_log_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let events = log
            .read_log("00000000-0000-0000-0000-000000000916")
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn list_run_ids_for_worktree_empty() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let ids = log.list_run_ids_for_worktree("/repo").unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn list_run_ids_for_worktree_filters_by_path() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000908".to_string(),
            workflow_name: "wf-a".to_string(),
            workflow_file_stem: "wf-a".to_string(),
            worktree_path: "/repo-1".to_string(),
            workflow_definition: minimal_workflow_for_log("test"),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000909".to_string(),
            workflow_name: "wf-b".to_string(),
            workflow_file_stem: "wf-b".to_string(),
            worktree_path: "/repo-2".to_string(),
            workflow_definition: minimal_workflow_for_log("test"),
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000910".to_string(),
            workflow_name: "wf-c".to_string(),
            workflow_file_stem: "wf-c".to_string(),
            worktree_path: "/repo-1".to_string(),
            workflow_definition: minimal_workflow_for_log("test"),
            timestamp: 1002.0,
        })
        .unwrap();

        let mut ids = log.list_run_ids_for_worktree("/repo-1").unwrap();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "00000000-0000-0000-0000-000000000908",
                "00000000-0000-0000-0000-000000000910"
            ]
        );

        let ids2 = log.list_run_ids_for_worktree("/repo-2").unwrap();
        assert_eq!(ids2, vec!["00000000-0000-0000-0000-000000000909"]);

        let ids3 = log.list_run_ids_for_worktree("/other").unwrap();
        assert!(ids3.is_empty());
    }

    #[test]
    fn event_serde_all_variants() {
        let events = vec![
            WorkflowEvent::RunStarted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                workflow_definition: minimal_workflow_for_log("test"),
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                execution_count: 1,
                timestamp: 2.0,
            },
            WorkflowEvent::WorkflowStallObserved {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                chat_session_id: "sess-1".to_string(),
                step_name: "s1".to_string(),
                run_index: 1,
                turn_phase: "streaming".to_string(),
                idle_secs: 180,
                signal_count: 1,
                cap_reached: false,
                timestamp: 2.5,
            },
            WorkflowEvent::WorkflowStallCleared {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                chat_session_id: "sess-1".to_string(),
                timestamp: 2.6,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: None,
                timestamp: 3.0,
            },
            WorkflowEvent::NodeFailed {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                reason: "error".to_string(),
                failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 4.0,
            },
            WorkflowEvent::ApprovalRequested {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                timestamp: 4.5,
            },
            WorkflowEvent::ApprovalResolved {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                decision: ApprovalDecisionRecord::Approve,
                comment: None,
                timestamp: 4.7,
            },
            WorkflowEvent::RunCompleted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                },
                timestamp: 5.0,
            },
            WorkflowEvent::RunFailed {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                reason: "failed".to_string(),
                failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 6.0,
            },
            WorkflowEvent::RunAborted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                aborted_step: None,
                timestamp: 7.0,
            },
            WorkflowEvent::OutputCollected {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "collect".to_string(),
                node_outputs: vec![CollectedOutputEntry {
                    node_name: "s1".to_string(),
                    result: Some("LGTM".to_string()),
                    structured_output: None,
                }],
                reduce_strategy: "AnyNeedsFix".to_string(),
                reduce_result: Some("LGTM".to_string()),
                reduce_structured_output: None,
                timestamp: 8.0,
            },
            WorkflowEvent::ParallelStarted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["arch-review".to_string(), "security-review".to_string()],
                timestamp: 9.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "arch-review".to_string(),
                session_id: "sess-1".to_string(),
                execution_count: 1,
                timestamp: 10.0,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "arch-review".to_string(),
                result: None,
                session_id: "sess-1".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: 50,
                    output_tokens: 25,
                }),
                structured_output: None,
                run_index: 0,
                state: "completed".to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 11.0,
            },
            WorkflowEvent::ParallelCompleted {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                aggregate_result: "then".to_string(),
                timestamp: 12.0,
            },
            WorkflowEvent::ContractRepairRequested {
                run_id: "00000000-0000-0000-0000-000000000911".to_string(),
                workflow_name: "wf".to_string(),
                node_name: "s1".to_string(),
                run_index: 1,
                request_id: Some("00000000-0000-0000-0000-000000000912".to_string()),
                attempt: 1,
                violation_reason: "missing_field".to_string(),
                timestamp: 13.0,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let back: WorkflowEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    fn make_agent_node(
        name: &str,
        instruction: &str,
    ) -> crate::adaptor::gateway::workflow::schema::NodeDefinition {
        use crate::adaptor::gateway::workflow::schema::{
            FacetRefs, NodeDefinition, NodeKind, SessionSpec,
        };
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some(instruction.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_approval_node(
        name: &str,
        instruction: &str,
    ) -> crate::adaptor::gateway::workflow::schema::NodeDefinition {
        use crate::adaptor::gateway::workflow::schema::{
            FacetRefs, NodeDefinition, NodeKind, SessionGate, SessionSpec,
        };
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some(instruction.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_test_workflow() -> Workflow {
        use crate::adaptor::gateway::workflow::schema::{CycleGuard, TransitionRule};
        let mut plan = make_agent_node("plan", "plan");
        let mut implement = make_agent_node("implement", "implement");
        implement.transition_rules = vec![TransitionRule {
            r#match: "review".to_string(),
            next: "review".to_string(),
        }];
        let mut review = make_approval_node("review", "review");
        review.cycle_guard = Some(CycleGuard {
            max_iterations: 3,
            on_exhausted: None,
        });
        let _ = &mut plan;
        Workflow {
            variables: Default::default(),
            name: "test-wf".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![plan, implement, review],
        }
    }

    #[test]
    fn reconstruct_state_empty_log() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let result =
            reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000916").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn reconstruct_state_completed_workflow() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: wf.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            result: Some("done".to_string()),
            session_id: Some("sess-plan".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
            structured_output: Some(serde_json::json!({"text": "plan output text"})),
            run_index: Some(1),
            timestamp: 1002.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "implement".to_string(),
            execution_count: 1,
            timestamp: 1003.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "implement".to_string(),
            result: None,
            session_id: Some("sess-impl".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: None,
            timestamp: 1004.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::OutputCollected {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "collect".to_string(),
            node_outputs: vec![CollectedOutputEntry {
                node_name: "plan".to_string(),
                result: Some("done".to_string()),
                structured_output: None,
            }],
            reduce_strategy: "Last".to_string(),
            reduce_result: Some("done".to_string()),
            reduce_structured_output: None,
            timestamp: 1004.5,
        })
        .unwrap();
        log.append(&WorkflowEvent::RunCompleted {
            run_id: "00000000-0000-0000-0000-000000000906".to_string(),
            workflow_name: "test-wf".to_string(),
            total_token_usage: TokenUsage {
                input_tokens: 200,
                output_tokens: 100,
            },
            timestamp: 1005.0,
        })
        .unwrap();

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000906")
            .unwrap()
            .unwrap();
        assert_eq!(state.execution_id, "00000000-0000-0000-0000-000000000906");
        assert_eq!(state.state, WorkflowExecutionState::Completed);
        assert_eq!(state.step_history.len(), 2);
        assert_eq!(
            state.step_history[0].session_id,
            Some("sess-plan".to_string())
        );
        assert_eq!(
            state.step_history[0].structured_output,
            Some(serde_json::json!({"text": "plan output text"}))
        );
        assert_eq!(state.step_history[0].run_index, 1);
        assert!(state.step_outputs.contains_key("plan"));
        assert_eq!(
            state.step_outputs["plan"].structured_output,
            Some(serde_json::json!({"text": "plan output text"}))
        );
        assert_eq!(
            state.step_history[1].session_id,
            Some("sess-impl".to_string())
        );
        assert_eq!(state.total_token_usage.input_tokens, 200);
        assert_eq!(state.step_states["plan"], "completed");
        assert_eq!(state.step_states["implement"], "completed");
        assert_eq!(state.step_states["review"], "pending");
        assert_eq!(state.started_at, 1000.0);
        assert_eq!(state.updated_at, 1005.0);
    }

    #[test]
    fn reconstruct_state_failed_workflow() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000907".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: wf.clone(),
            timestamp: 2000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000907".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 2001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeFailed {
            run_id: "00000000-0000-0000-0000-000000000907".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "plan".to_string(),
            reason: "exit code 1".to_string(),
            failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 2002.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::RunFailed {
            run_id: "00000000-0000-0000-0000-000000000907".to_string(),
            workflow_name: "test-wf".to_string(),
            reason: "step failed".to_string(),
            failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 2003.0,
        })
        .unwrap();

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000907")
            .unwrap()
            .unwrap();
        assert_eq!(
            state.state,
            WorkflowExecutionState::Failed {
                reason: "step failed".to_string(),
                kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
            }
        );
        assert_eq!(state.step_states["plan"], "failed");
        assert_eq!(state.step_states["implement"], "pending");
        // make_test_workflow() の nodes が snapshot 経由で復元されていること
        assert_eq!(state.total_steps, wf.nodes.len());
    }

    /// 並列ブロックを含むワークフローのNDJSON復元テスト。
    /// ParallelCompleted + NodeCompleted で親ステップが重複しないことを検証する。
    #[test]
    fn reconstruct_state_parallel_block_no_duplicate_history() {
        use crate::adaptor::gateway::workflow::schema::{
            FacetRefs, FanoutSpec, InterimChild, NodeDefinition, NodeKind, ParallelAggregate,
        };

        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());

        let make_child = |name: &str, instruction: &str| InterimChild {
            name: name.to_string(),
            facets: FacetRefs {
                instruction: Some(instruction.to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let parallel_review = NodeDefinition {
            name: "parallel-review".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                parallel_children: vec![
                    make_child("arch-review", "arch"),
                    make_child("security-review", "security"),
                ],
                aggregate: Some(ParallelAggregate {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "_complete".to_string(),
                    r#else: "_complete".to_string(),
                }),
            }),
            ..NodeDefinition::default()
        };
        let wf = Workflow {
            variables: Default::default(),
            name: "parallel-wf".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![make_agent_node("plan", "plan"), parallel_review],
        };

        let events = vec![
            WorkflowEvent::RunStarted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                workflow_file_stem: "parallel-wf".to_string(),
                worktree_path: "/repo".to_string(),
                workflow_definition: wf.clone(),
                timestamp: 1000.0,
            },
            WorkflowEvent::NodeStarted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                node_name: "plan".to_string(),
                execution_count: 1,
                timestamp: 1001.0,
            },
            WorkflowEvent::NodeCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                node_name: "plan".to_string(),
                result: Some("done".to_string()),
                session_id: Some("sess-plan".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                }),
                structured_output: Some(serde_json::json!({"text": "plan output"})),
                run_index: Some(1),
                timestamp: 1002.0,
            },
            WorkflowEvent::ParallelStarted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_names: vec!["arch-review".to_string(), "security-review".to_string()],
                timestamp: 1003.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "arch-review".to_string(),
                session_id: "sess-arch".to_string(),
                execution_count: 1,
                timestamp: 1004.0,
            },
            WorkflowEvent::ParallelChildStarted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "security-review".to_string(),
                session_id: "sess-sec".to_string(),
                execution_count: 1,
                timestamp: 1004.0,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "arch-review".to_string(),
                result: Some("LGTM".to_string()),
                session_id: "sess-arch".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: 200,
                    output_tokens: 100,
                }),
                structured_output: Some(serde_json::json!({"verdict": "LGTM"})),
                run_index: 1,
                state: "completed".to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1005.0,
            },
            WorkflowEvent::ParallelChildCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                child_node_name: "security-review".to_string(),
                result: Some("LGTM".to_string()),
                session_id: "sess-sec".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: 150,
                    output_tokens: 75,
                }),
                structured_output: Some(serde_json::json!({"verdict": "LGTM"})),
                run_index: 1,
                state: "completed".to_string(),
                failure_kind: None,
                failure_disposition: None,
                timestamp: 1006.0,
            },
            WorkflowEvent::ParallelCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                parent_node_name: "parallel-review".to_string(),
                aggregate_result: "then".to_string(),
                timestamp: 1007.0,
            },
            // engine.rsのwrite_last_step_completed_logが親ステップのNodeCompletedを出力
            WorkflowEvent::NodeCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                node_name: "parallel-review".to_string(),
                result: Some("then".to_string()),
                session_id: None,
                token_usage: Some(TokenUsage {
                    input_tokens: 350,
                    output_tokens: 175,
                }),
                structured_output: None,
                run_index: Some(1),
                timestamp: 1007.0,
            },
            WorkflowEvent::RunCompleted {
                run_id: "00000000-0000-0000-0000-000000000912".to_string(),
                workflow_name: "parallel-wf".to_string(),
                total_token_usage: TokenUsage {
                    input_tokens: 450,
                    output_tokens: 225,
                },
                timestamp: 1008.0,
            },
        ];

        for event in &events {
            log.append(event).unwrap();
        }

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000912")
            .unwrap()
            .unwrap();

        assert_eq!(
            state.step_history.len(),
            2,
            "step_history should have exactly 2 entries, not duplicated"
        );
        assert_eq!(state.step_history[0].step_name, "plan");
        assert_eq!(state.step_history[1].step_name, "parallel-review");
        assert_eq!(state.step_history[1].result, Some("then".to_string()));
        assert_eq!(state.step_history[1].structured_output, None);

        assert_eq!(state.current_step_name, "parallel-review");
        assert_eq!(state.current_step_index, 1);

        assert!(state.step_outputs.contains_key("arch-review"));
        assert!(state.step_outputs.contains_key("security-review"));

        assert!(state.active_parallel_steps.is_empty());

        assert_eq!(state.state, WorkflowExecutionState::Completed);
    }

    #[test]
    fn reconstruct_state_reject_comment_preserved() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000913".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: wf.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000913".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "review".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: "00000000-0000-0000-0000-000000000913".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "review".to_string(),
            result: Some("reject".to_string()),
            session_id: Some("sess-review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 200,
                output_tokens: 80,
            }),
            structured_output: None,
            run_index: Some(1),
            timestamp: 1002.0,
        })
        .unwrap();

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000913")
            .unwrap()
            .unwrap();

        assert_eq!(state.step_history.len(), 1);
        assert_eq!(state.step_history[0].step_name, "review");
        assert_eq!(state.step_history[0].result, Some("reject".to_string()));
        assert!(state.step_history[0].structured_output.is_none());

        assert!(!state.step_outputs.contains_key("review"));
    }

    /// [04] ApprovalRequested projection: 承認待ちに到達すると state が
    /// WaitingApproval に切り替わり、current_step_name / current_step_index が
    /// approval 対象 node に揃う。updated_at は ApprovalRequested の timestamp に
    /// 進む。観測者が承認待ち run を識別できる境界を担保する。
    #[test]
    fn reconstruct_state_approval_requested_sets_waiting_approval() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000914".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: wf.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000914".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "review".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::ApprovalRequested {
            run_id: "00000000-0000-0000-0000-000000000914".to_string(),
            workflow_name: "test-wf".to_string(),
            node_name: "review".to_string(),
            timestamp: 1002.0,
        })
        .unwrap();

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000914")
            .unwrap()
            .unwrap();
        assert_eq!(state.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(state.current_step_name, "review");
        // workflow.nodes は plan/implement/review の順なので review の index は 2。
        assert_eq!(state.current_step_index, 2);
        assert_eq!(state.updated_at, 1002.0);
    }

    /// [04] approval 承認 → 次 node 開始: ApprovalRequested で WaitingApproval に
    /// 切り替わった後、ApprovalResolved → NodeCompleted → NodeStarted の順に
    /// 進むと、exec_state は次 node 開始時に Running へ復元される。
    /// projection のバグ「WaitingApproval が固定される」回帰防止。
    #[test]
    fn reconstruct_state_node_started_after_approval_resets_to_running() {
        use crate::adaptor::gateway::workflow::schema::TransitionRule;

        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        // approval node のあとに後続 node を続ける workflow を作る。
        let mut review = make_approval_node("review", "review");
        review.transition_rules = vec![TransitionRule {
            r#match: "approve".to_string(),
            next: "ship".to_string(),
        }];
        let wf = Workflow {
            variables: Default::default(),
            name: "approval-then-next".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![review, make_agent_node("ship", "ship")],
        };

        log.append(&WorkflowEvent::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            workflow_file_stem: "approval-then-next".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: wf.clone(),
            timestamp: 2000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            node_name: "review".to_string(),
            execution_count: 1,
            timestamp: 2001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::ApprovalRequested {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            node_name: "review".to_string(),
            timestamp: 2002.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::ApprovalResolved {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Approve,
            comment: None,
            timestamp: 2003.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            node_name: "review".to_string(),
            result: Some("approve".to_string()),
            session_id: Some("sess-review".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 2004.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: "00000000-0000-0000-0000-000000000915".to_string(),
            workflow_name: "approval-then-next".to_string(),
            node_name: "ship".to_string(),
            execution_count: 1,
            timestamp: 2005.0,
        })
        .unwrap();

        let state = reconstruct_state_via_log(&log, "00000000-0000-0000-0000-000000000915")
            .unwrap()
            .unwrap();
        assert_eq!(
            state.state,
            WorkflowExecutionState::Running,
            "次 node 開始で WaitingApproval は Running に復元される"
        );
        assert_eq!(state.current_step_name, "ship");
        assert_eq!(state.current_step_index, 1);
    }
}
