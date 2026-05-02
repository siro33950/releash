use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workflow::schema::Workflow;
use crate::workflow::state::{StepHistoryEntry, TokenUsage, WorkflowExecutionState, WorkflowState};

/// NDJSONログに書き込むイベントの種類。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum WorkflowLogEvent {
    WorkflowStarted {
        execution_id: String,
        workflow_name: String,
        #[serde(default)]
        workflow_file_stem: String,
        worktree_path: String,
        #[serde(default)]
        workflow_definition: Option<Workflow>,
        timestamp: f64,
    },
    StepStarted {
        execution_id: String,
        workflow_name: String,
        step_name: String,
        execution_count: u32,
        timestamp: f64,
    },
    StepCompleted {
        execution_id: String,
        workflow_name: String,
        step_name: String,
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    },
    StepFailed {
        execution_id: String,
        workflow_name: String,
        step_name: String,
        reason: String,
        timestamp: f64,
    },
    WorkflowCompleted {
        execution_id: String,
        workflow_name: String,
        total_token_usage: TokenUsage,
        timestamp: f64,
    },
    WorkflowFailed {
        execution_id: String,
        workflow_name: String,
        reason: String,
        timestamp: f64,
    },
    WorkflowAborted {
        execution_id: String,
        workflow_name: String,
        timestamp: f64,
    },
}

/// ワークフロー実行ログの書き込み・読み込み。
pub struct WorkflowEventLog {
    log_dir: PathBuf,
}

impl WorkflowEventLog {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            log_dir: data_dir.join("workflow_logs"),
        }
    }

    fn log_path(&self, execution_id: &str) -> PathBuf {
        self.log_dir.join(format!("{execution_id}.ndjson"))
    }

    /// イベントをNDJSON形式でログファイルに追記する。
    pub fn append(&self, event: &WorkflowLogEvent) -> Result<(), String> {
        fs::create_dir_all(&self.log_dir).map_err(|e| format!("Failed to create log dir: {e}"))?;

        let execution_id = match event {
            WorkflowLogEvent::WorkflowStarted { execution_id, .. }
            | WorkflowLogEvent::StepStarted { execution_id, .. }
            | WorkflowLogEvent::StepCompleted { execution_id, .. }
            | WorkflowLogEvent::StepFailed { execution_id, .. }
            | WorkflowLogEvent::WorkflowCompleted { execution_id, .. }
            | WorkflowLogEvent::WorkflowFailed { execution_id, .. }
            | WorkflowLogEvent::WorkflowAborted { execution_id, .. } => execution_id,
        };

        let path = self.log_path(execution_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Failed to open log file: {e}"))?;

        let json =
            serde_json::to_string(event).map_err(|e| format!("Failed to serialize event: {e}"))?;
        writeln!(file, "{json}").map_err(|e| format!("Failed to write log: {e}"))?;
        Ok(())
    }

    /// 指定された実行IDのNDJSONログを読み込み、イベント一覧を返す。
    pub fn read_log(&self, execution_id: &str) -> Result<Vec<WorkflowLogEvent>, String> {
        let path = self.log_path(execution_id);
        if !path.exists() {
            return Ok(vec![]);
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read log file: {e}"))?;
        let mut events = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event: WorkflowLogEvent =
                serde_json::from_str(line).map_err(|e| format!("Failed to parse log line: {e}"))?;
            events.push(event);
        }
        Ok(events)
    }

    /// ログファイルを読み込み、ワークフロー定義からWorkflowStateを再構築する。
    #[cfg(test)]
    pub fn reconstruct_state(
        &self,
        execution_id: &str,
        workflow: &Workflow,
    ) -> Result<Option<WorkflowState>, String> {
        let events = self.read_log(execution_id)?;
        Self::reconstruct_state_from_events(execution_id, &events, workflow)
    }

    /// 既にパース済みのイベント列からWorkflowStateを再構築する。
    pub fn reconstruct_state_from_events(
        execution_id: &str,
        events: &[WorkflowLogEvent],
        workflow: &Workflow,
    ) -> Result<Option<WorkflowState>, String> {
        if events.is_empty() {
            return Ok(None);
        }

        let mut started_at = 0.0;
        let mut updated_at = 0.0;
        let mut step_history: Vec<StepHistoryEntry> = Vec::new();
        let mut step_execution_counts: HashMap<String, u32> = HashMap::new();
        let mut total_token_usage = TokenUsage::default();
        let mut exec_state = WorkflowExecutionState::Running;
        let mut current_step_name = workflow
            .steps
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let mut current_step_index = 0usize;
        let mut workflow_name = String::new();

        for event in events {
            match event {
                WorkflowLogEvent::WorkflowStarted {
                    timestamp,
                    workflow_name: wn,
                    ..
                } => {
                    started_at = *timestamp;
                    updated_at = *timestamp;
                    workflow_name = wn.clone();
                }
                WorkflowLogEvent::StepStarted {
                    step_name,
                    execution_count,
                    timestamp,
                    ..
                } => {
                    current_step_name = step_name.clone();
                    current_step_index = workflow
                        .steps
                        .iter()
                        .position(|s| s.name == *step_name)
                        .unwrap_or(0);
                    step_execution_counts.insert(step_name.clone(), *execution_count);
                    updated_at = *timestamp;
                }
                WorkflowLogEvent::StepCompleted {
                    step_name,
                    result,
                    session_id,
                    token_usage,
                    timestamp,
                    ..
                } => {
                    step_history.push(StepHistoryEntry {
                        step_name: step_name.clone(),
                        completed_at: *timestamp,
                        result: result.clone(),
                        session_id: session_id.clone(),
                        token_usage: token_usage.clone(),
                    });
                    if let Some(ref usage) = token_usage {
                        total_token_usage.add(usage);
                    }
                    updated_at = *timestamp;
                }
                WorkflowLogEvent::StepFailed {
                    reason, timestamp, ..
                } => {
                    exec_state = WorkflowExecutionState::Failed {
                        reason: reason.clone(),
                    };
                    updated_at = *timestamp;
                }
                WorkflowLogEvent::WorkflowCompleted {
                    total_token_usage: tu,
                    timestamp,
                    ..
                } => {
                    exec_state = WorkflowExecutionState::Completed;
                    total_token_usage = tu.clone();
                    updated_at = *timestamp;
                }
                WorkflowLogEvent::WorkflowFailed {
                    reason, timestamp, ..
                } => {
                    exec_state = WorkflowExecutionState::Failed {
                        reason: reason.clone(),
                    };
                    updated_at = *timestamp;
                }
                WorkflowLogEvent::WorkflowAborted { timestamp, .. } => {
                    exec_state = WorkflowExecutionState::Aborted;
                    updated_at = *timestamp;
                }
            }
        }

        let step_states = crate::workflow::state::compute_step_states(
            workflow,
            current_step_index,
            &exec_state,
            &step_history,
        );

        Ok(Some(WorkflowState {
            execution_id: execution_id.to_string(),
            workflow_name,
            chat_session_id: None,
            state: exec_state,
            current_step_index,
            current_step_name,
            current_session_id: None,
            total_steps: workflow.steps.len(),
            step_history,
            step_execution_counts,
            workflow_definition: workflow.clone(),
            total_token_usage,
            step_states,
            started_at,
            updated_at,
        }))
    }

    /// 指定worktreeに属する実行IDを返す。
    /// 各NDJSONファイルの1行目（WorkflowStarted）のworktree_pathと照合する。
    pub fn list_execution_ids_for_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<String>, String> {
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
            let execution_id = stem.to_string_lossy().to_string();

            // 1行目を読んでworktree_pathを照合
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(first_line) = content.lines().next() {
                    if let Ok(WorkflowLogEvent::WorkflowStarted {
                        worktree_path: wt, ..
                    }) = serde_json::from_str(first_line)
                    {
                        if wt == worktree_path {
                            ids.push(execution_id);
                        }
                    }
                }
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_and_read_log() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());

        let event1 = WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: None,
            timestamp: 1000.0,
        };
        let event2 = WorkflowLogEvent::StepStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        };
        let event3 = WorkflowLogEvent::StepCompleted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            result: Some("done".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
            timestamp: 1002.0,
        };

        log.append(&event1).unwrap();
        log.append(&event2).unwrap();
        log.append(&event3).unwrap();

        let events = log.read_log("exec-1").unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn read_nonexistent_log_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let events = log.read_log("nonexistent").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn list_execution_ids_for_worktree_empty() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let ids = log.list_execution_ids_for_worktree("/repo").unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn list_execution_ids_for_worktree_filters_by_path() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());

        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-a".to_string(),
            workflow_name: "wf-a".to_string(),
            workflow_file_stem: "wf-a".to_string(),
            worktree_path: "/repo-1".to_string(),
            workflow_definition: None,
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-b".to_string(),
            workflow_name: "wf-b".to_string(),
            workflow_file_stem: "wf-b".to_string(),
            worktree_path: "/repo-2".to_string(),
            workflow_definition: None,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-c".to_string(),
            workflow_name: "wf-c".to_string(),
            workflow_file_stem: "wf-c".to_string(),
            worktree_path: "/repo-1".to_string(),
            workflow_definition: None,
            timestamp: 1002.0,
        })
        .unwrap();

        let mut ids = log.list_execution_ids_for_worktree("/repo-1").unwrap();
        ids.sort();
        assert_eq!(ids, vec!["exec-a", "exec-c"]);

        let ids2 = log.list_execution_ids_for_worktree("/repo-2").unwrap();
        assert_eq!(ids2, vec!["exec-b"]);

        let ids3 = log.list_execution_ids_for_worktree("/other").unwrap();
        assert!(ids3.is_empty());
    }

    #[test]
    fn event_serde_all_variants() {
        let events = vec![
            WorkflowLogEvent::WorkflowStarted {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                workflow_definition: None,
                timestamp: 1.0,
            },
            WorkflowLogEvent::StepStarted {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                step_name: "s1".to_string(),
                execution_count: 1,
                timestamp: 2.0,
            },
            WorkflowLogEvent::StepCompleted {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                step_name: "s1".to_string(),
                result: None,
                session_id: None,
                token_usage: None,
                timestamp: 3.0,
            },
            WorkflowLogEvent::StepFailed {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                step_name: "s1".to_string(),
                reason: "error".to_string(),
                timestamp: 4.0,
            },
            WorkflowLogEvent::WorkflowCompleted {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                total_token_usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                },
                timestamp: 5.0,
            },
            WorkflowLogEvent::WorkflowFailed {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                reason: "failed".to_string(),
                timestamp: 6.0,
            },
            WorkflowLogEvent::WorkflowAborted {
                execution_id: "e1".to_string(),
                workflow_name: "wf".to_string(),
                timestamp: 7.0,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let back: WorkflowLogEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    fn make_test_workflow() -> Workflow {
        use crate::workflow::schema::{CycleGuard, Step, StepPrompt, TransitionRule};
        Workflow {
            name: "test-wf".to_string(),
            description: "".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    name: "plan".to_string(),
                    mode: crate::workflow::schema::StepMode::Auto,
                    prompt: StepPrompt::inline(""),
                    rules: vec![],
                    cycle_guard: None,
                },
                Step {
                    name: "implement".to_string(),
                    mode: crate::workflow::schema::StepMode::Auto,
                    prompt: StepPrompt::inline(""),
                    rules: vec![TransitionRule {
                        r#match: "review".to_string(),
                        next: "review".to_string(),
                    }],
                    cycle_guard: None,
                },
                Step {
                    name: "review".to_string(),
                    mode: crate::workflow::schema::StepMode::Approval,
                    prompt: StepPrompt::inline(""),
                    rules: vec![],
                    cycle_guard: Some(CycleGuard { max_iterations: 3 }),
                },
            ],
        }
    }

    #[test]
    fn reconstruct_state_empty_log() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();
        let result = log.reconstruct_state("nonexistent", &wf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn reconstruct_state_completed_workflow() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let wf = make_test_workflow();

        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: None,
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepCompleted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            result: Some("done".to_string()),
            session_id: Some("sess-plan".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
            timestamp: 1002.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "implement".to_string(),
            execution_count: 1,
            timestamp: 1003.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepCompleted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "implement".to_string(),
            result: None,
            session_id: Some("sess-impl".to_string()),
            token_usage: None,
            timestamp: 1004.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::WorkflowCompleted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            total_token_usage: TokenUsage {
                input_tokens: 200,
                output_tokens: 100,
            },
            timestamp: 1005.0,
        })
        .unwrap();

        let state = log.reconstruct_state("exec-1", &wf).unwrap().unwrap();
        assert_eq!(state.execution_id, "exec-1");
        assert_eq!(state.state, WorkflowExecutionState::Completed);
        assert_eq!(state.step_history.len(), 2);
        assert_eq!(
            state.step_history[0].session_id,
            Some("sess-plan".to_string())
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

        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: "exec-2".to_string(),
            workflow_name: "test-wf".to_string(),
            workflow_file_stem: "test-wf".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: None,
            timestamp: 2000.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepStarted {
            execution_id: "exec-2".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            execution_count: 1,
            timestamp: 2001.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepFailed {
            execution_id: "exec-2".to_string(),
            workflow_name: "test-wf".to_string(),
            step_name: "plan".to_string(),
            reason: "exit code 1".to_string(),
            timestamp: 2002.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::WorkflowFailed {
            execution_id: "exec-2".to_string(),
            workflow_name: "test-wf".to_string(),
            reason: "step failed".to_string(),
            timestamp: 2003.0,
        })
        .unwrap();

        let state = log.reconstruct_state("exec-2", &wf).unwrap().unwrap();
        assert_eq!(
            state.state,
            WorkflowExecutionState::Failed {
                reason: "step failed".to_string()
            }
        );
        assert_eq!(state.step_states["plan"], "failed");
        assert_eq!(state.step_states["implement"], "pending");
    }
}
