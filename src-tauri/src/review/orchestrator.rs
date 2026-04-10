use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::pty::oneshot::{OneShotPtyManager, OneShotStatus};
use crate::pty::PtyOutput;
use crate::review_prompt::PerFileReviewTask;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[allow(dead_code)]
    Idle,
    Running,
    Completed,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileReviewStatus {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReviewState {
    pub file_path: String,
    pub status: FileReviewStatus,
    pub pty_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewProgress {
    pub done: usize,
    pub total: usize,
    pub error_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewSessionStatus {
    pub status: ReviewStatus,
    pub file_states: Vec<FileReviewState>,
    pub progress: ReviewProgress,
}

struct ReviewSession {
    worktree_path: String,
    command_template: String,
    task_queue: VecDeque<PerFileReviewTask>,
    file_states: Vec<FileReviewState>,
    concurrency: usize,
    pty_to_file: HashMap<u64, usize>,
    active_count: usize,
    done_count: usize,
    total_count: usize,
    error_count: usize,
    cancelled: bool,
}

struct OrchestratorState {
    sessions: HashMap<String, ReviewSession>,
    pty_to_session: HashMap<u64, String>,
}

pub struct ReviewOrchestrator {
    state: Mutex<OrchestratorState>,
}

impl ReviewOrchestrator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(OrchestratorState {
                sessions: HashMap::new(),
                pty_to_session: HashMap::new(),
            }),
        }
    }

    /// Register Tauri event listeners for PTY status changes and output.
    /// Must be called once during app setup.
    pub fn register_listeners(app: &AppHandle) {
        let app_status = app.clone();
        app.listen("oneshot-pty-status-changed", move |event| {
            if let Ok(info) =
                serde_json::from_str::<crate::pty::oneshot::OneShotPtyInfo>(event.payload())
            {
                let is_terminal = matches!(
                    info.status,
                    OneShotStatus::Completed
                        | OneShotStatus::Error
                        | OneShotStatus::Timeout
                        | OneShotStatus::Cancelled
                );
                if !is_terminal {
                    return;
                }
                if let Some(orchestrator) = app_status.try_state::<Arc<ReviewOrchestrator>>() {
                    orchestrator.handle_pty_completed(&app_status, info.pty_id, &info.status);
                }
            }
        });

        let app_output = app.clone();
        app.listen("pty-output", move |event| {
            if let Ok(payload) = serde_json::from_str::<PtyOutput>(event.payload()) {
                if let Some(orchestrator) = app_output.try_state::<Arc<ReviewOrchestrator>>() {
                    orchestrator.handle_pty_output(&app_output, payload.pty_id, &payload.data);
                }
            }
        });
    }

    pub fn start_review(
        &self,
        app: &AppHandle,
        worktree_path: &str,
        command_template: &str,
        concurrency: usize,
        tasks: Vec<PerFileReviewTask>,
    ) -> String {
        let session_id = worktree_path.to_string();
        let concurrency = concurrency.max(1);

        // Cancel existing review for this worktree if any
        let existing_ptys: Vec<u64> = {
            let mut state = self.state.lock();
            if let Some(existing) = state.sessions.remove(&session_id) {
                let ptys: Vec<u64> = existing.pty_to_file.keys().copied().collect();
                for &pty_id in &ptys {
                    state.pty_to_session.remove(&pty_id);
                }
                ptys
            } else {
                Vec::new()
            }
        };

        if !existing_ptys.is_empty() {
            if let Some(pty_mgr) = app.try_state::<Arc<OneShotPtyManager>>() {
                for pty_id in existing_ptys {
                    let _ = pty_mgr.cancel(app, pty_id);
                }
            }
        }

        let file_states: Vec<FileReviewState> = tasks
            .iter()
            .map(|t| FileReviewState {
                file_path: t.file_path.clone(),
                status: FileReviewStatus::Pending,
                pty_id: None,
            })
            .collect();

        let total = tasks.len();
        let session = ReviewSession {
            worktree_path: worktree_path.to_string(),
            command_template: command_template.to_string(),
            task_queue: VecDeque::from(tasks),
            file_states,
            concurrency,
            pty_to_file: HashMap::new(),
            active_count: 0,
            done_count: 0,
            total_count: total,
            error_count: 0,
            cancelled: false,
        };

        {
            let mut state = self.state.lock();
            state.sessions.insert(session_id.clone(), session);
        }

        self.emit_state_changed(app, &session_id);
        self.spawn_next_batch(app, &session_id, concurrency);

        session_id
    }

    pub fn cancel_review(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        let pty_ids: Vec<u64> = {
            let mut state = self.state.lock();
            let ptys = {
                let session = state
                    .sessions
                    .get_mut(session_id)
                    .ok_or("Review session not found")?;
                session.cancelled = true;
                session.task_queue.clear();
                let ptys: Vec<u64> = session.pty_to_file.keys().copied().collect();
                session.pty_to_file.clear();
                session.active_count = 0;
                ptys
            };
            for pty_id in &ptys {
                state.pty_to_session.remove(pty_id);
            }
            ptys
        };

        if let Some(pty_mgr) = app.try_state::<Arc<OneShotPtyManager>>() {
            for pty_id in pty_ids {
                let _ = pty_mgr.cancel(app, pty_id);
            }
        }

        self.emit_state_changed(app, session_id);
        Ok(())
    }

    pub fn get_status(&self, session_id: &str) -> Option<ReviewSessionStatus> {
        let state = self.state.lock();
        let session = state.sessions.get(session_id)?;
        Some(build_status(session))
    }

    pub fn reset(&self, session_id: &str) {
        let mut state = self.state.lock();
        if let Some(session) = state.sessions.remove(session_id) {
            for pty_id in session.pty_to_file.keys() {
                state.pty_to_session.remove(pty_id);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal event handlers
    // -----------------------------------------------------------------------

    fn handle_pty_completed(&self, app: &AppHandle, pty_id: u64, status: &OneShotStatus) {
        let is_error = matches!(
            status,
            OneShotStatus::Error | OneShotStatus::Timeout | OneShotStatus::Cancelled
        );

        let (session_id, should_spawn_next) = {
            let mut state = self.state.lock();

            let session_id = match state.pty_to_session.remove(&pty_id) {
                Some(id) => id,
                None => return, // Not tracked by any review session
            };

            let session = match state.sessions.get_mut(&session_id) {
                Some(s) => s,
                None => return,
            };

            if session.cancelled {
                return;
            }

            if let Some(&file_idx) = session.pty_to_file.get(&pty_id) {
                session.file_states[file_idx].status = if is_error {
                    FileReviewStatus::Error
                } else {
                    FileReviewStatus::Done
                };
            }
            session.pty_to_file.remove(&pty_id);
            session.active_count = session.active_count.saturating_sub(1);
            session.done_count += 1;
            if is_error {
                session.error_count += 1;
            }

            let should_spawn =
                session.done_count < session.total_count && !session.task_queue.is_empty();

            (session_id, should_spawn)
        };

        self.emit_state_changed(app, &session_id);

        if should_spawn_next {
            self.spawn_next_batch(app, &session_id, 1);
        }
    }

    fn handle_pty_output(&self, app: &AppHandle, pty_id: u64, data: &str) {
        let emit_info = {
            let state = self.state.lock();
            state.pty_to_session.get(&pty_id).and_then(|session_id| {
                state.sessions.get(session_id).and_then(|session| {
                    session.pty_to_file.get(&pty_id).map(|&file_idx| {
                        (
                            session_id.clone(),
                            session.file_states[file_idx].file_path.clone(),
                        )
                    })
                })
            })
        };

        if let Some((session_id, file_path)) = emit_info {
            let _ = app.emit(
                "review-file-output",
                serde_json::json!({
                    "review_session_id": session_id,
                    "file_path": file_path,
                    "data": data,
                }),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn spawn_next_batch(&self, app: &AppHandle, session_id: &str, count: usize) {
        for _ in 0..count {
            if !self.spawn_single_task(app, session_id) {
                break;
            }
        }
    }

    /// Spawn a single task from the queue. Returns `true` if a task was popped
    /// (even if spawn failed), `false` if no task was available.
    fn spawn_single_task(&self, app: &AppHandle, session_id: &str) -> bool {
        // 1. Pop task from queue (lock held briefly)
        let spawn_info = {
            let mut state = self.state.lock();
            let session = match state.sessions.get_mut(session_id) {
                Some(s) if !s.cancelled && s.active_count < s.concurrency => s,
                _ => return false,
            };
            session.task_queue.pop_front().map(|task| {
                (
                    task,
                    session.command_template.clone(),
                    session.worktree_path.clone(),
                )
            })
        };

        let (task, command_template, worktree_path) = match spawn_info {
            Some(info) => info,
            None => return false,
        };

        // 2. Build command and spawn PTY (NO LOCK HELD — spawn_oneshot emits events)
        let command = build_command(&command_template, &task.prompt);
        let label = format!("review:{}", task.file_path);
        let pty_mgr = match app.try_state::<Arc<OneShotPtyManager>>() {
            Some(m) => m,
            None => {
                self.mark_file_error(app, session_id, &task.file_path);
                return true;
            }
        };

        match pty_mgr.spawn_oneshot(app, &command, &worktree_path, &label, None) {
            Ok(info) => {
                // 3. Register pty_id mapping (re-acquire lock)
                {
                    let mut state = self.state.lock();
                    if let Some(session) = state.sessions.get_mut(session_id) {
                        if session.cancelled {
                            let _ = pty_mgr.cancel(app, info.pty_id);
                            return false;
                        }
                        if let Some(idx) = session
                            .file_states
                            .iter()
                            .position(|f| f.file_path == task.file_path)
                        {
                            session.file_states[idx].status = FileReviewStatus::Running;
                            session.file_states[idx].pty_id = Some(info.pty_id);
                            session.pty_to_file.insert(info.pty_id, idx);
                            session.active_count += 1;
                            state
                                .pty_to_session
                                .insert(info.pty_id, session_id.to_string());
                        }
                    }
                }

                // 4. Handle race condition: PTY may have completed during spawn
                if let Some(current) = pty_mgr.get_status(info.pty_id) {
                    if matches!(
                        current.status,
                        OneShotStatus::Completed
                            | OneShotStatus::Error
                            | OneShotStatus::Timeout
                            | OneShotStatus::Cancelled
                    ) {
                        self.handle_pty_completed(app, info.pty_id, &current.status);
                    }
                }

                self.emit_state_changed(app, session_id);
                true
            }
            Err(_) => {
                self.mark_file_error(app, session_id, &task.file_path);
                true
            }
        }
    }

    fn mark_file_error(&self, app: &AppHandle, session_id: &str, file_path: &str) {
        {
            let mut state = self.state.lock();
            if let Some(session) = state.sessions.get_mut(session_id) {
                if let Some(idx) = session
                    .file_states
                    .iter()
                    .position(|f| f.file_path == file_path)
                {
                    session.file_states[idx].status = FileReviewStatus::Error;
                }
                session.done_count += 1;
                session.error_count += 1;
            }
        }
        self.emit_state_changed(app, session_id);
    }

    fn emit_state_changed(&self, app: &AppHandle, session_id: &str) {
        let status = {
            let state = self.state.lock();
            state.sessions.get(session_id).map(build_status)
        };
        if let Some(status) = status {
            let _ = app.emit("review-state-changed", &status);
        }
    }
}

fn build_status(session: &ReviewSession) -> ReviewSessionStatus {
    let status = if session.cancelled {
        ReviewStatus::Cancelled
    } else if session.total_count == 0 {
        ReviewStatus::Completed
    } else if session.done_count >= session.total_count {
        if session.error_count > 0 {
            ReviewStatus::Error
        } else {
            ReviewStatus::Completed
        }
    } else {
        ReviewStatus::Running
    };

    ReviewSessionStatus {
        status,
        file_states: session.file_states.clone(),
        progress: ReviewProgress {
            done: session.done_count,
            total: session.total_count,
            error_count: session.error_count,
        },
    }
}

fn shell_escape_prompt(prompt: &str) -> String {
    prompt
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

fn build_command(template: &str, prompt: &str) -> String {
    let escaped = shell_escape_prompt(prompt);
    template.replace("{prompt}", &escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_handles_special_chars() {
        let input = r#"Hello "world" $HOME `echo`\n"#;
        let escaped = shell_escape_prompt(input);
        assert_eq!(escaped, r#"Hello \"world\" \$HOME \`echo\`\\n"#);
    }

    #[test]
    fn build_command_replaces_prompt() {
        let template = r#"echo "{prompt}" | claude -p"#;
        let prompt = "Review this file";
        let result = build_command(template, prompt);
        assert_eq!(result, r#"echo "Review this file" | claude -p"#);
    }

    #[test]
    fn build_command_escapes_prompt() {
        let template = r#"echo "{prompt}" | claude"#;
        let prompt = r#"say "hello""#;
        let result = build_command(template, prompt);
        assert_eq!(result, r#"echo "say \"hello\"" | claude"#);
    }

    #[test]
    fn build_status_idle_when_empty() {
        let session = ReviewSession {
            worktree_path: "/repo".to_string(),
            command_template: "echo {prompt}".to_string(),
            task_queue: VecDeque::new(),
            file_states: vec![],
            concurrency: 5,
            pty_to_file: HashMap::new(),
            active_count: 0,
            done_count: 0,
            total_count: 0,
            error_count: 0,
            cancelled: false,
        };
        let status = build_status(&session);
        assert_eq!(status.status, ReviewStatus::Completed);
        assert_eq!(status.progress.total, 0);
    }

    #[test]
    fn build_status_running() {
        let session = ReviewSession {
            worktree_path: "/repo".to_string(),
            command_template: "echo {prompt}".to_string(),
            task_queue: VecDeque::new(),
            file_states: vec![
                FileReviewState {
                    file_path: "a.ts".to_string(),
                    status: FileReviewStatus::Running,
                    pty_id: Some(1),
                },
                FileReviewState {
                    file_path: "b.ts".to_string(),
                    status: FileReviewStatus::Pending,
                    pty_id: None,
                },
            ],
            concurrency: 1,
            pty_to_file: HashMap::from([(1, 0)]),
            active_count: 1,
            done_count: 0,
            total_count: 2,
            error_count: 0,
            cancelled: false,
        };
        let status = build_status(&session);
        assert_eq!(status.status, ReviewStatus::Running);
        assert_eq!(status.progress.done, 0);
        assert_eq!(status.progress.total, 2);
    }

    #[test]
    fn build_status_completed() {
        let session = ReviewSession {
            worktree_path: "/repo".to_string(),
            command_template: "echo {prompt}".to_string(),
            task_queue: VecDeque::new(),
            file_states: vec![FileReviewState {
                file_path: "a.ts".to_string(),
                status: FileReviewStatus::Done,
                pty_id: Some(1),
            }],
            concurrency: 1,
            pty_to_file: HashMap::new(),
            active_count: 0,
            done_count: 1,
            total_count: 1,
            error_count: 0,
            cancelled: false,
        };
        let status = build_status(&session);
        assert_eq!(status.status, ReviewStatus::Completed);
    }

    #[test]
    fn build_status_error() {
        let session = ReviewSession {
            worktree_path: "/repo".to_string(),
            command_template: "echo {prompt}".to_string(),
            task_queue: VecDeque::new(),
            file_states: vec![
                FileReviewState {
                    file_path: "a.ts".to_string(),
                    status: FileReviewStatus::Done,
                    pty_id: Some(1),
                },
                FileReviewState {
                    file_path: "b.ts".to_string(),
                    status: FileReviewStatus::Error,
                    pty_id: Some(2),
                },
            ],
            concurrency: 1,
            pty_to_file: HashMap::new(),
            active_count: 0,
            done_count: 2,
            total_count: 2,
            error_count: 1,
            cancelled: false,
        };
        let status = build_status(&session);
        assert_eq!(status.status, ReviewStatus::Error);
        assert_eq!(status.progress.error_count, 1);
    }

    #[test]
    fn build_status_cancelled() {
        let session = ReviewSession {
            worktree_path: "/repo".to_string(),
            command_template: "echo {prompt}".to_string(),
            task_queue: VecDeque::new(),
            file_states: vec![],
            concurrency: 1,
            pty_to_file: HashMap::new(),
            active_count: 0,
            done_count: 0,
            total_count: 2,
            error_count: 0,
            cancelled: true,
        };
        let status = build_status(&session);
        assert_eq!(status.status, ReviewStatus::Cancelled);
    }

    #[test]
    fn orchestrator_new() {
        let orchestrator = ReviewOrchestrator::new();
        assert!(orchestrator.get_status("/repo").is_none());
    }

    #[test]
    fn orchestrator_reset_clears_session() {
        let orchestrator = ReviewOrchestrator::new();
        // Manually insert a session
        {
            let mut state = orchestrator.state.lock();
            state.sessions.insert(
                "/repo".to_string(),
                ReviewSession {
                    worktree_path: "/repo".to_string(),
                    command_template: "echo {prompt}".to_string(),
                    task_queue: VecDeque::new(),
                    file_states: vec![],
                    concurrency: 1,
                    pty_to_file: HashMap::new(),
                    active_count: 0,
                    done_count: 0,
                    total_count: 0,
                    error_count: 0,
                    cancelled: false,
                },
            );
        }
        assert!(orchestrator.get_status("/repo").is_some());
        orchestrator.reset("/repo");
        assert!(orchestrator.get_status("/repo").is_none());
    }
}
