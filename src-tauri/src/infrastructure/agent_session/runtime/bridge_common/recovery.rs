use super::external_agent::ExternalBridgeMessageState;
use super::process_registry::{AgentProcess, AgentProcessMap, BridgeState, TurnPhase};
use super::sdk_message::{
    bridge_message_is_stale_for_active_turn, handle_external_bridge_message,
    sdk_error_part_from_message,
};
use super::session_lifecycle::{
    complete_streaming_turn_post_lock, crash_agent_process_for_context_reinject,
    run_turn_complete_transition_locked_with_interrupt, start_pending_message_turn,
    sweep_process_group, take_pending_message, TurnCompletePostOptions, TurnCompleteTransition,
};
use super::session_persistence::{
    persist_context_carry_failed_after_init_error, persist_resume_mismatch_for_reinject,
    requeue_streaming_turn_for_resume_mismatch, session_ready_resume_mismatch,
    streaming_turn_requeue_candidate,
};
use super::shared::{
    backend_runtime_config, build_init_cmd, compose_system_prompt, notify_status_transition,
    resolve_bridge_script, resolve_effective_base_branch_from_port, session_specific_env_overrides,
    write_bridge_command_for_captured_turn, BridgeInitOptions, GENERATION_COUNTER,
};
use super::stream_emit::{emit_session_state_changed, emit_streaming_parts, enqueue_pending_delta};
use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::context_restore::RestoreContextPayload;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;
use crate::usecase::agent_session::event_log::InterruptReason;
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::session::ContextCarryState;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::SessionStore;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{watch, Mutex};

pub(super) const BRIDGE_EOF_ERROR_MESSAGE: &str = "Bridge process exited unexpectedly.";
pub(super) const STALE_EXIT_CODE: i64 = 124;
pub(super) const STALE_TIMEOUT_SECS: u64 = 180;
pub(super) const STALE_RECOVERY_GRACE_SECS: u64 = 10;
pub(super) const WATCHDOG_TICK_SECS: u64 = 5;
pub(super) const STALE_ERROR_MESSAGE: &str =
    "Claude 応答が停止したため中断しました。もう一度お試しください。";

pub(super) fn claude_bridge_watchdog_env_overrides() -> Vec<(&'static str, String)> {
    vec![
        (
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
            (STALE_TIMEOUT_SECS * 1000).to_string(),
        ),
        ("CLAUDE_ENABLE_STREAM_WATCHDOG", "1".to_string()),
        ("CLAUDE_ENABLE_BYTE_WATCHDOG", "1".to_string()),
        ("CLAUDE_CODE_MAX_RETRIES", "10".to_string()),
        ("API_TIMEOUT_MS", "600000".to_string()),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnLivenessTimeout {
    Stale,
}

impl TurnLivenessTimeout {
    pub(crate) fn user_message(self) -> &'static str {
        match self {
            Self::Stale => STALE_ERROR_MESSAGE,
        }
    }
}

pub(super) fn evaluate_turn_liveness(
    turn_phase: TurnPhase,
    last_progress_at: Option<Instant>,
    turn_phase_since: Instant,
    now: Instant,
) -> Option<TurnLivenessTimeout> {
    match turn_phase {
        TurnPhase::Streaming => {
            let base = last_progress_at.unwrap_or(turn_phase_since);
            (now.duration_since(base) > Duration::from_secs(STALE_TIMEOUT_SECS))
                .then_some(TurnLivenessTimeout::Stale)
        }
        TurnPhase::Idle | TurnPhase::WaitingPermission => None,
    }
}

#[derive(Debug)]
pub(crate) struct CleanupGate {
    tx: watch::Sender<bool>,
}

impl CleanupGate {
    pub(crate) fn new(initially_open: bool) -> Self {
        let (tx, _) = watch::channel(initially_open);
        Self { tx }
    }

    pub(crate) fn open(&self) {
        self.tx.send_replace(true);
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        *self.tx.borrow()
    }

    pub(crate) async fn wait_until_open(&self) {
        if *self.tx.borrow() {
            return;
        }

        let mut rx = self.tx.subscribe();
        if *rx.borrow_and_update() {
            return;
        }

        if rx.wait_for(|open| *open).await.is_err() {
            log::warn!("startup orphan cleanup gate closed before opening; continuing spawn");
        }
    }
}

pub(crate) async fn wait_for_startup_orphan_cleanup<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Some(gate) = app
        .try_state::<Arc<CleanupGate>>()
        .map(|state| state.inner().clone())
    else {
        log::warn!("startup orphan cleanup gate state is not registered; continuing spawn");
        return;
    };
    gate.wait_until_open().await;
}

#[cfg(unix)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct OrphanCleanupReport {
    pub(crate) scanned: usize,
    pub(crate) processed: usize,
    pub(crate) skipped: usize,
    pub(crate) failures: usize,
}

pub(super) fn pids_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("pids")
}

#[cfg(unix)]
pub(super) fn validate_session_id_for_path(chat_session_id: &str) -> Result<(), String> {
    if chat_session_id.is_empty()
        || chat_session_id.contains('/')
        || chat_session_id.contains('\\')
        || chat_session_id.contains("..")
        || chat_session_id.contains('\0')
    {
        return Err(format!(
            "Invalid chat_session_id for PID file: {chat_session_id:?}"
        ));
    }
    Ok(())
}

/// On-disk representation of a PID file (v1).
///
/// Records the bridge process group, plus identification of the Releash app
/// instance that owns it. `cleanup_orphan_processes` uses `owner_app_pid` and
/// `owner_start_time` to distinguish "left over from a previous crash of this
/// app instance" from "currently owned by another live Releash instance" — the
/// latter must not be touched (issue #1024).
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
pub(super) struct PidFileV1 {
    pub(crate) version: u32,
    pub(crate) pgid: i32,
    pub(crate) owner_app_pid: u32,
    /// Platform-specific start time of `owner_app_pid`. Used to detect PID
    /// reuse: if the recorded `owner_app_pid` is alive but its start time
    /// differs from the recorded value, the PID was recycled and the file is
    /// stale.
    pub(crate) owner_start_time: u64,
}

/// Linux: read field 22 (`starttime`) of `/proc/{pid}/stat`. Value is the
/// process start time expressed in clock ticks since system boot — stable
/// across queries for the same process.
#[cfg(all(unix, target_os = "linux"))]
pub(super) fn get_process_start_time(pid: u32) -> Result<u64, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("Failed to read /proc/{pid}/stat: {e}"))?;
    // The `comm` field (field 2) can contain spaces and parens; the last ')'
    // marks its end. Fields after are space-separated.
    let rparen = stat
        .rfind(')')
        .ok_or_else(|| format!("Malformed /proc/{pid}/stat: missing ')'"))?;
    let after = stat[rparen + 1..].trim();
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After ')' the next field is `state` (field 3). `starttime` is field 22,
    // so the index into `fields` is 22 - 3 = 19.
    let starttime = fields
        .get(19)
        .ok_or_else(|| format!("/proc/{pid}/stat missing starttime field"))?;
    starttime
        .parse::<u64>()
        .map_err(|e| format!("Failed to parse starttime in /proc/{pid}/stat: {e}"))
}

/// macOS: query `proc_bsdinfo` via `proc_pidinfo(pid, PROC_PIDTBSDINFO, ...)`
/// and combine `pbi_start_tvsec`/`pbi_start_tvusec` into microseconds since
/// epoch. The value is fixed for the lifetime of the process.
#[cfg(all(unix, target_os = "macos"))]
pub(super) fn get_process_start_time(pid: u32) -> Result<u64, String> {
    use std::mem::MaybeUninit;
    let mut info: MaybeUninit<libc::proc_bsdinfo> = MaybeUninit::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            size,
        )
    };
    if ret <= 0 {
        return Err(format!(
            "proc_pidinfo(PROC_PIDTBSDINFO) failed for {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    if ret < size {
        return Err(format!(
            "proc_pidinfo(PROC_PIDTBSDINFO) returned {ret} bytes, expected {size}"
        ));
    }
    let info = unsafe { info.assume_init() };
    Ok(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

/// Unsupported Unix flavor: return an error. Callers treat this as "owner
/// identity unverifiable" and conservatively skip cleanup of unfamiliar files.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn get_process_start_time(_pid: u32) -> Result<u64, String> {
    Err("Unsupported platform for process start_time lookup".to_string())
}

#[cfg(unix)]
pub(super) fn save_pgid(
    app_data_dir: &Path,
    chat_session_id: &str,
    pgid: u32,
) -> Result<(), String> {
    validate_session_id_for_path(chat_session_id)?;
    let dir = pids_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create pids dir: {e}"))?;
    let owner_app_pid = std::process::id();
    // start_time が取得できないプラットフォーム/失敗時は 0 を保存する。
    // cleanup 側では 0 を「未検証」として扱い、live owner なら保守的に skip する
    // （bridge spawn そのものを失敗させない: issue #1024）。
    let owner_start_time = get_process_start_time(owner_app_pid).unwrap_or(0);
    let payload = PidFileV1 {
        version: 1,
        pgid: pgid as i32,
        owner_app_pid,
        owner_start_time,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize PID file: {e}"))?;
    let file = dir.join(format!("{chat_session_id}.pid"));
    // Atomic write: tmp + rename. Avoids leaving a half-written file readable
    // by a concurrent cleanup pass.
    let tmp = dir.join(format!("{chat_session_id}.pid.tmp"));
    std::fs::write(&tmp, json).map_err(|e| format!("Failed to write pid file: {e}"))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("Failed to rename pid file: {e}"))?;
    Ok(())
}

#[cfg(unix)]
pub(super) fn remove_pgid(app_data_dir: &Path, chat_session_id: &str) {
    if validate_session_id_for_path(chat_session_id).is_err() {
        return;
    }
    let file = pids_dir(app_data_dir).join(format!("{chat_session_id}.pid"));
    let _ = std::fs::remove_file(file);
}

#[cfg(unix)]
pub fn cleanup_orphan_processes(app_data_dir: &Path) -> OrphanCleanupReport {
    let mut report = OrphanCleanupReport::default();
    let dir = pids_dir(app_data_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return report,
        Err(e) => {
            report.failures += 1;
            log::warn!("Failed to read startup orphan cleanup PID directory: {e}");
            return report;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                report.failures += 1;
                log::warn!("Failed to read startup orphan cleanup PID directory entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "pid") {
            continue;
        }
        report.scanned += 1;
        let contents = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                report.failures += 1;
                log::warn!("Failed to read startup orphan PID file; counting failure: {e}");
                continue;
            }
        };

        let parsed: PidFileV1 = match serde_json::from_str::<PidFileV1>(contents.trim()) {
            Ok(p) => p,
            Err(_) => {
                // Legacy or unknown format. Conservatively skip — touching it
                // could destroy a live owner's bookkeeping (issue #1024).
                log::warn!("Startup orphan cleanup skipped PID file with unsupported format");
                report.skipped += 1;
                continue;
            }
        };

        if parsed.pgid <= 1 {
            log::warn!("Startup orphan cleanup removed PID file with invalid process group");
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    report.processed += 1;
                }
                Err(e) => {
                    report.failures += 1;
                    log::warn!("Failed to remove invalid startup orphan PID file: {e}");
                }
            }
            continue;
        }

        // Determine whether the recorded owner is still our own previous run
        // (cleanup OK) or a *different* live Releash instance (must skip).
        let owner_pid_i32 = parsed.owner_app_pid as i32;
        let owner_alive = owner_pid_i32 > 1 && unsafe { libc::kill(owner_pid_i32, 0) } == 0;
        if owner_alive {
            match get_process_start_time(parsed.owner_app_pid) {
                Ok(current_start_time) if current_start_time == parsed.owner_start_time => {
                    log::info!("Startup orphan cleanup skipped PID file owned by a live instance");
                    report.skipped += 1;
                    continue;
                }
                Ok(_) => {
                    log::info!(
                        "Startup orphan cleanup found reused owner identity; proceeding with cleanup"
                    );
                }
                Err(_) => {
                    // live owner だが start_time を検証できない: 保守的に skip
                    // する（unsupported プラットフォームや一時的 I/O 失敗で他
                    // インスタンスの bridge を誤殺しないため: issue #1024）。
                    log::warn!(
                        "Startup orphan cleanup skipped PID file because owner identity could not be verified"
                    );
                    report.skipped += 1;
                    continue;
                }
            }
        }

        // Orphan: owner is dead, or PID was reused, or start_time unverifiable.
        let pgid = parsed.pgid;
        let alive = unsafe { libc::killpg(pgid, 0) } == 0;
        if alive {
            log::info!("Startup orphan cleanup is terminating an orphan process group");
            unsafe {
                libc::killpg(pgid, libc::SIGTERM);
            }
            // Give processes time to exit, then force kill
            std::thread::sleep(std::time::Duration::from_secs(2));
            let still_alive = unsafe { libc::killpg(pgid, 0) } == 0;
            if still_alive {
                log::warn!(
                    "Startup orphan cleanup orphan process group did not exit; sending SIGKILL"
                );
                unsafe {
                    libc::killpg(pgid, libc::SIGKILL);
                }
            }
            report.processed += 1;
            if let Err(e) = std::fs::remove_file(&path) {
                report.failures += 1;
                log::warn!("Failed to remove orphan PID file after process cleanup: {e}");
            }
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    report.processed += 1;
                }
                Err(e) => {
                    report.failures += 1;
                    log::warn!("Failed to remove stale startup orphan PID file: {e}");
                }
            }
        }
    }
    report
}

#[derive(Debug, Default)]
pub(super) struct BridgeEofCrashTransition {
    pub(crate) turn_complete: TurnCompleteTransition,
    pub(crate) was_initializing: bool,
    pub(crate) sdk_error_message: Option<String>,
    pub(crate) context_restore_failed_on_init: bool,
    /// Ready/Idle EOF means the process is not reusable. Callers can remove it
    /// immediately only when no pending queue needs to survive until respawn.
    pub(crate) should_evict: bool,
}

pub(super) fn run_bridge_eof_crash_transition_locked<F>(
    generation_matches: bool,
    proc: &mut AgentProcess,
    chat_session_id: &str,
    emit_stream: F,
) -> BridgeEofCrashTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    if !generation_matches {
        return BridgeEofCrashTransition::default();
    }

    let turn_completed = proc.state == BridgeState::Streaming;
    let was_initializing = proc.state == BridgeState::Initializing;
    // Ready/Idle EOF: the turn already completed but the child exited, so the
    // process is no longer reusable and must be evicted before the next send.
    let should_evict = proc.state == BridgeState::Ready && proc.turn_phase == TurnPhase::Idle;
    let sdk_error_message = if turn_completed || was_initializing {
        Some(format!("{}: {BRIDGE_EOF_ERROR_MESSAGE}", proc.backend_id))
    } else {
        None
    };

    if turn_completed {
        let part = MessagePart::Error {
            content: format!("Error: {BRIDGE_EOF_ERROR_MESSAGE}"),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(proc, &[part]);
    }

    let turn_complete = if turn_completed || was_initializing {
        run_turn_complete_transition_locked_with_interrupt(
            proc,
            chat_session_id,
            -1,
            Some(InterruptReason::BridgeCrash),
            Some(format!("Error: {BRIDGE_EOF_ERROR_MESSAGE}")),
            emit_stream,
        )
    } else {
        TurnCompleteTransition::default()
    };
    let context_restore_failed_on_init = !turn_complete.turn_completed
        && was_initializing
        && proc.context_carry_on_ready.take().is_some();

    BridgeEofCrashTransition {
        turn_complete,
        was_initializing,
        sdk_error_message,
        context_restore_failed_on_init,
        should_evict,
    }
}

pub(super) fn finalize_turn_as_timeout_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    timeout: TurnLivenessTimeout,
    emit_stream: F,
) -> TurnCompleteTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    if proc.turn_phase == TurnPhase::Idle {
        return TurnCompleteTransition::default();
    }
    let error_part = MessagePart::Error {
        content: timeout.user_message().to_string(),
        parent_tool_use_id: None,
    };
    proc.streaming_parts.push(error_part.clone());
    enqueue_pending_delta(proc, &[error_part]);
    run_turn_complete_transition_locked_with_interrupt(
        proc,
        chat_session_id,
        STALE_EXIT_CODE,
        Some(InterruptReason::Timeout),
        Some(timeout.user_message().to_string()),
        emit_stream,
    )
}

#[derive(Debug, Default)]
pub(super) struct BridgeErrorTransition {
    pub(crate) turn_complete: TurnCompleteTransition,
    pub(crate) was_initializing: bool,
    pub(crate) context_restore_failed_on_init: bool,
}

pub(super) fn run_bridge_error_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    emit_stream: F,
) -> BridgeErrorTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let was_initializing = proc.state == BridgeState::Initializing;
    if proc.state == BridgeState::Streaming {
        // `accumulate_sdk_message` does not synthesize Error parts; add it here
        // so the crash payload carries the error before the turn-complete flush.
        let part = sdk_error_part_from_message(msg);
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(proc, std::slice::from_ref(&part));
    }
    let turn_complete = run_turn_complete_transition_locked_with_interrupt(
        proc,
        chat_session_id,
        1,
        Some(InterruptReason::BridgeCrash),
        Some(match sdk_error_part_from_message(msg) {
            MessagePart::Error { content, .. } => content,
            _ => "Error: Unknown bridge error".to_string(),
        }),
        emit_stream,
    );
    let context_restore_failed_on_init = !turn_complete.turn_completed
        && was_initializing
        && proc.context_carry_on_ready.take().is_some();

    BridgeErrorTransition {
        turn_complete,
        was_initializing,
        context_restore_failed_on_init,
    }
}

pub(super) fn retire_ready_eof_runtime_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
) -> bool {
    let has_pending_messages = map
        .get(chat_session_id)
        .is_some_and(|proc| !proc.pending_messages.is_empty());

    if has_pending_messages {
        if let Some(proc) = map.get_mut(chat_session_id) {
            // Keep the dead process as a non-user-visible respawn marker so
            // ensure_runtime_for_turn can carry pending_messages into the next
            // runtime through the same path as other crashed replacements.
            proc.state = BridgeState::Crashed;
            proc.turn_phase = TurnPhase::Idle;
        }
        false
    } else {
        map.remove(chat_session_id).is_some()
    }
}

pub(super) fn ready_idle_child_exited(proc: &mut AgentProcess, chat_session_id: &str) -> bool {
    if proc.state != BridgeState::Ready || proc.turn_phase != TurnPhase::Idle {
        return false;
    }

    match proc.child.try_wait() {
        Ok(Some(_status)) => true,
        Ok(None) => false,
        Err(e) => {
            log::warn!("Failed to inspect ready agent process {chat_session_id}: {e}");
            false
        }
    }
}

pub(super) enum RuntimeSpawnDecision {
    Missing,
    Replace(Box<AgentProcess>),
    Reuse,
}

pub(super) fn take_runtime_requiring_spawn_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
) -> RuntimeSpawnDecision {
    if !runtime_requires_spawn_locked(map, chat_session_id) {
        return RuntimeSpawnDecision::Reuse;
    }

    if !map.contains_key(chat_session_id) {
        return RuntimeSpawnDecision::Missing;
    }

    RuntimeSpawnDecision::Replace(Box::new(
        map.remove(chat_session_id)
            .expect("runtime existed when replacement was requested"),
    ))
}

pub(super) fn runtime_requires_spawn_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
) -> bool {
    let Some(proc) = map.get_mut(chat_session_id) else {
        return true;
    };

    if proc.state == BridgeState::Crashed {
        return true;
    }
    if ready_idle_child_exited(proc, chat_session_id) {
        proc.state = BridgeState::Crashed;
        proc.turn_phase = TurnPhase::Idle;
        return true;
    }
    false
}

pub(super) const CLOSE_TIMEOUT_SECS: u64 = 5;

enum RecoveryBridgeMessageAction {
    Delegate,
    Handled,
}

async fn spawn_pending_message_turn_if_ready<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) {
    let Some(pending) = take_pending_message(handles, chat_session_id).await else {
        return;
    };
    let app_p = app.clone();
    let ss_p = Arc::clone(session_store);
    let h_p = Arc::clone(handles);
    let csid_p = chat_session_id.to_string();
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        handle.block_on(async move {
            start_pending_message_turn(&app_p, &h_p, &ss_p, &csid_p, pending).await;
        });
    });
}

async fn session_ready_will_transition_from_initializing(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    msg: &serde_json::Value,
) -> bool {
    let map = handles.lock().await;
    map.get(chat_session_id).is_some_and(|proc| {
        !bridge_message_is_stale_for_active_turn(proc, msg)
            && proc.state == BridgeState::Initializing
    })
}

async fn handle_session_ready_resume_mismatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    msg: &serde_json::Value,
) -> bool {
    let (resume_mismatch, requeue_candidate) = {
        let map = handles.lock().await;
        let Some(proc) = map.get(chat_session_id) else {
            return false;
        };
        if bridge_message_is_stale_for_active_turn(proc, msg) {
            return false;
        }
        let ready_session_id = msg.get("session_id").and_then(|v| v.as_str());
        let resume_mismatch = session_ready_resume_mismatch(
            proc.context_carry_on_ready.as_ref(),
            proc.sdk_session_id.as_deref(),
            ready_session_id,
        );
        let requeue_candidate = if resume_mismatch {
            streaming_turn_requeue_candidate(proc)
        } else {
            None
        };
        (resume_mismatch, requeue_candidate)
    };
    if !resume_mismatch {
        return false;
    }

    let requeued_streaming_turn = if let Some(candidate) = requeue_candidate {
        requeue_streaming_turn_for_resume_mismatch(
            app,
            handles,
            session_store,
            chat_session_id,
            candidate,
        )
        .await
    } else {
        false
    };
    persist_resume_mismatch_for_reinject(app, session_store, chat_session_id);
    crash_agent_process_for_context_reinject(app, handles, chat_session_id).await;
    if requeued_streaming_turn {
        emit_session_state_changed(app, chat_session_id, TurnPhase::Idle, None, false);
        notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);
        spawn_pending_message_turn_if_ready(app, session_store, handles, chat_session_id).await;
    }
    true
}

async fn handle_stdout_bridge_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    msg: &serde_json::Value,
) {
    use tauri::Emitter;

    let error_msg = msg
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown bridge error");
    log::error!("Bridge error [{}]: {}", chat_session_id, error_msg);

    let transition = {
        let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
        let mut map = handles.lock().await;
        map.get_mut(chat_session_id).map(|proc| {
            run_bridge_error_transition_locked(proc, chat_session_id, msg, |mid, parts| {
                emit_streaming_parts(app, chat_session_id, mid, parts.to_vec())
            })
        })
    };

    let _ = app.emit("agent-sdk-message", msg);

    let transition = transition.unwrap_or_default();
    let was_initializing = transition.was_initializing;
    let context_restore_failed_on_init = transition.context_restore_failed_on_init;
    let turn_complete = transition.turn_complete;
    complete_streaming_turn_post_lock(
        app,
        session_store,
        handles,
        chat_session_id,
        turn_complete,
        TurnCompletePostOptions {
            consume_pending: true,
        },
    )
    .await;
    if was_initializing {
        notify_status_transition(
            app,
            session_store,
            chat_session_id,
            TurnPhase::Idle,
            Some(crate::usecase::agent_session::session::SessionState::Error),
        );
    }
    // Init error → clear stale agent_session_id to prevent infinite resume loop
    if was_initializing
        || msg
            .get("clear_session_id")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || msg
            .get("context_carry_failed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        persist_context_carry_failed_after_init_error(
            app,
            session_store,
            chat_session_id,
            was_initializing
                || msg
                    .get("clear_session_id")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            context_restore_failed_on_init
                || msg
                    .get("context_carry_failed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
        );
    }
}

async fn handle_recovery_bridge_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    msg: &serde_json::Value,
) -> RecoveryBridgeMessageAction {
    match msg.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "session_ready" => {
            if handle_session_ready_resume_mismatch(
                app,
                session_store,
                handles,
                chat_session_id,
                msg,
            )
            .await
            {
                RecoveryBridgeMessageAction::Handled
            } else {
                RecoveryBridgeMessageAction::Delegate
            }
        }
        "error" => {
            handle_stdout_bridge_error(app, session_store, handles, chat_session_id, msg).await;
            RecoveryBridgeMessageAction::Handled
        }
        _ => RecoveryBridgeMessageAction::Delegate,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn spawn_bridge_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    backend_id: String,
    session_id: Option<String>,
    cwd: &str,
    permission_mode: String,
    plan_mode: bool,
    selected_model: Option<String>,
    system_prompt: Option<String>,
    restore_context: Option<RestoreContextPayload>,
) -> Result<(), String> {
    let bridge_path = resolve_bridge_script(app, &backend_id)?;
    if !bridge_path.exists() {
        return Err(format!(
            "Bridge script not found: {}",
            bridge_path.display()
        ));
    }

    // spawn 前にパーミッションモードを検証する。Tauri/WS 境界で検証済みのはずだが、
    // 内部経路の保護として二重に弾く（不正値で子プロセス起動を許さない）。
    let initial_permission_mode = permission_mode;
    crate::permission::PermissionMode::parse(&initial_permission_mode)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let data_dir = resolve_data_dir(app)
        .map_err(|e| format!("Failed to resolve data dir for session {chat_session_id}: {e}"))?;

    let mut cmd = Command::new("node");
    cmd.arg(
        bridge_path
            .to_str()
            .ok_or_else(|| "Bridge script path contains invalid UTF-8".to_string())?,
    )
    .current_dir(cwd)
    // Remove Claude Code nesting-detection env vars so the SDK-spawned
    // `claude` CLI does not refuse to start.
    .env_remove("CLAUDECODE")
    .env_remove("CLAUDE_CODE_ENTRYPOINT")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    // spec issues-1054: agent bridge にも起動環境別 alias が解決可能な PATH と
    // `RELEASH_DATA_DIR` を伝搬する（bridge 経由で呼ばれるツールが alias を解決できるように）。
    match crate::path_aliases::prepare_child_env(app.path().app_data_dir().ok()) {
        Ok(env) => {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }
        Err(e) => {
            // 提示する alias と実行環境の不整合を避けるため、wrapper 作成失敗時は
            // bridge 起動を中止する（spec issues-1054「agent 子プロセスへの実行
            // 環境の伝搬」: PATH 経由で alias 解決可能な環境を約束する）。
            return Err(format!(
                "failed to prepare alias child env for agent bridge: {e}"
            ));
        }
    }

    // spec issues-1022 "Agent process environment contract": agent process 自身が
    // 自分の chat_session_id を env 経由で参照できるよう、session 固有 env を
    // pure helper 経由で組み立てて設置する。
    // 周辺入口（agent bridge）は code usecase へ直接依存せず、composition root が注入した
    // narrow port 経由で base 名を解決する。エラーは移行前と同じく None に倒す。
    let base_branch = resolve_effective_base_branch_from_port(app, cwd);
    for (k, v) in session_specific_env_overrides(chat_session_id, base_branch.as_deref()) {
        cmd.env(k, v);
    }
    for (k, v) in claude_bridge_watchdog_env_overrides() {
        cmd.env(k, v);
    }

    #[cfg(unix)]
    // SAFETY: setsid() is async-signal-safe per POSIX.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    wait_for_startup_orphan_cleanup(app).await;
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn node process: {e}"))?;

    #[cfg(unix)]
    let pgid = child.id();
    #[cfg(unix)]
    if let Some(pg) = pgid {
        if let Err(e) = save_pgid(&data_dir, chat_session_id, pg) {
            log::error!("Failed to save PGID file, killing spawned process group: {e}");
            unsafe {
                libc::killpg(pg as libc::pid_t, libc::SIGKILL);
            }
            return Err(format!(
                "Failed to save PGID file for session {chat_session_id}: {e}"
            ));
        }
    }

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture stderr".to_string())?;

    // Send init command（permission_mode は抽象モード文字列を期待）。
    // initial_permission_mode は spawn 前に検証済み（上方参照）。
    // spec issues-1022 "Agent process environment contract": ユーザー指定の
    // system_prompt に Releash CLI の long help を append したものを Agent に渡す。
    let composed_system_prompt = compose_system_prompt(system_prompt);
    let mut init_cmd = build_init_cmd(
        cwd,
        &initial_permission_mode,
        plan_mode,
        &session_id,
        &backend_id,
        BridgeInitOptions {
            system_prompt: composed_system_prompt,
            selected_model: selected_model.as_deref(),
            restore_context: restore_context.as_ref(),
        },
    )?;
    let runtime_config = backend_runtime_config(app, &backend_id);
    if let Some(init_obj) = init_cmd.as_object_mut() {
        for (key, value) in runtime_config.bridge_init_options {
            init_obj.insert(key, value);
        }
    }
    let init_data = format!("{}\n", init_cmd);
    stdin
        .write_all(init_data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write init command: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush init command: {e}"))?;

    // Store process
    let gen_id = GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let context_carry_on_ready = if session_id.is_some() {
        Some(ContextCarryState::Resumed)
    } else if restore_context
        .as_ref()
        .is_some_and(|payload| !payload.prompt_prefix.trim().is_empty())
    {
        Some(ContextCarryState::Reinjected)
    } else {
        None
    };
    {
        let mut map = handles.lock().await;
        map.insert(
            chat_session_id.to_string(),
            AgentProcess {
                stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
                backend_id,
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: session_id,
                context_carry_on_ready,
                child,
                generation_id: gen_id,
                #[cfg(unix)]
                pgid,
                streaming_message_id: None,
                active_turn_token: None,
                turn_latency: None,
                post_turn_message_token: None,
                streaming_parts: Vec::new(),
                turn_event_log: TurnEventLog::default(),
                last_message_id: None,
                post_turn_base_untrusted_message_id: None,
                task_id_map: HashMap::new(),
                pending_messages: VecDeque::new(),
                current_permission_mode: initial_permission_mode.clone(),
                available_models: Vec::new(),
                selected_model,
                last_result_token_usage: None,
                latest_token_usage: None,
                pending_stream_part_count: 0,
                pending_stream_bytes: 0,
                last_stream_emit_at: None,
                streaming_timer_active: false,
                last_progress_at: None,
                turn_phase_since: Instant::now(),
                turn_seq: 0,
                turn_watchdog_active: false,
            },
        );
    }

    // 初期 SessionStatus を AgentStatusCenter に登録（Idle で初期化）
    notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);

    // Spawn stdout reader (process-lifetime)
    let handles_stdout = Arc::clone(handles);
    let session_store_clone = Arc::clone(session_store);
    let app_stdout = app.clone();
    let csid_stdout = chat_session_id.to_string();
    let captured_gen_id = gen_id;
    tokio::spawn(async move {
        use tauri::Emitter;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut message_state = ExternalBridgeMessageState::default();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            if let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(&line) {
                msg["chat_session_id"] = serde_json::Value::String(csid_stdout.clone());

                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let should_start_pending_after_ready = msg_type == "session_ready"
                    && session_ready_will_transition_from_initializing(
                        &handles_stdout,
                        &csid_stdout,
                        &msg,
                    )
                    .await;
                if matches!(
                    handle_recovery_bridge_message(
                        &app_stdout,
                        &session_store_clone,
                        &handles_stdout,
                        &csid_stdout,
                        &msg,
                    )
                    .await,
                    RecoveryBridgeMessageAction::Handled
                ) {
                    continue;
                }
                handle_external_bridge_message(
                    &app_stdout,
                    &session_store_clone,
                    &handles_stdout,
                    &csid_stdout,
                    msg,
                    &mut message_state,
                )
                .await;
                if should_start_pending_after_ready {
                    spawn_pending_message_turn_if_ready(
                        &app_stdout,
                        &session_store_clone,
                        &handles_stdout,
                        &csid_stdout,
                    )
                    .await;
                }
            }
        }
        // EOF — process exited; verify generation to avoid acting on stale events.
        // Streaming 中の終了だけでなく、Initializing (session_ready 前) の終了も
        // AgentStatusCenter に Error として伝搬させる。Initializing の場合は
        // turn_id=-1 を伴う Idle emit は行わない（streaming が無かったため）。
        let (transition, should_remove_pid_file) = {
            let _runtime_guard = acquire_session_runtime_lock(&csid_stdout).await;
            let mut map = handles_stdout.lock().await;
            if let Some(proc) = map.get_mut(&csid_stdout) {
                let generation_matches = proc.generation_id == captured_gen_id;
                let transition = run_bridge_eof_crash_transition_locked(
                    generation_matches,
                    proc,
                    &csid_stdout,
                    |mid, parts| {
                        emit_streaming_parts(&app_stdout, &csid_stdout, mid, parts.to_vec())
                    },
                );
                // Ready/Idle EOF: the completed-but-dead process must be retired so
                // the next send re-spawns instead of writing into a dead runtime.
                let should_remove_pid_file = transition.should_evict
                    && retire_ready_eof_runtime_locked(&mut map, &csid_stdout);
                (transition, should_remove_pid_file)
            } else {
                (BridgeEofCrashTransition::default(), false)
            }
        };
        if should_remove_pid_file {
            #[cfg(unix)]
            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                remove_pgid(&data_dir, &csid_stdout);
            }
        }
        if transition.context_restore_failed_on_init {
            persist_context_carry_failed_after_init_error(
                &app_stdout,
                &session_store_clone,
                &csid_stdout,
                true,
                true,
            );
        }
        if let Some(message) = transition.sdk_error_message.as_deref() {
            let _ = app_stdout.emit(
                "agent-sdk-message",
                serde_json::json!({
                    "type": "error",
                    "message": message,
                    "chat_session_id": &csid_stdout,
                }),
            );
        }
        let was_initializing = transition.was_initializing;
        let effect = transition.turn_complete;
        if effect.turn_completed {
            complete_streaming_turn_post_lock(
                &app_stdout,
                &session_store_clone,
                &handles_stdout,
                &csid_stdout,
                effect,
                TurnCompletePostOptions {
                    consume_pending: true,
                },
            )
            .await;
        } else if was_initializing {
            notify_status_transition(
                &app_stdout,
                &session_store_clone,
                &csid_stdout,
                TurnPhase::Idle,
                Some(crate::usecase::agent_session::session::SessionState::Error),
            );
        }
    });

    // Spawn stderr reader (process-lifetime)
    let csid_stderr = chat_session_id.to_string();
    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                log::warn!("bridge stderr [{}]: {}", csid_stderr, line);
            }
        }
    });

    // The auxiliary streaming-flush timer is spawned per-turn from
    // `spawn_streaming_timer`, not at process spawn — process-lifetime ticks
    // would hold `AgentProcessMap` lock every 33ms even while idle.

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TurnWatchdogDecision {
    Continue,
    Timeout(TurnLivenessTimeout),
    BreakClearFlag,
    BreakKeepFlag,
}

pub(super) fn turn_watchdog_decision(
    proc: &AgentProcess,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    now: Instant,
) -> TurnWatchdogDecision {
    if proc.generation_id != captured_gen_id || proc.turn_seq != captured_turn_seq {
        return TurnWatchdogDecision::BreakKeepFlag;
    }
    if proc.turn_phase == TurnPhase::Idle || proc.state == BridgeState::Crashed {
        return TurnWatchdogDecision::BreakClearFlag;
    }
    match evaluate_turn_liveness(
        proc.turn_phase,
        proc.last_progress_at,
        proc.turn_phase_since,
        now,
    ) {
        Some(timeout) => TurnWatchdogDecision::Timeout(timeout),
        None => TurnWatchdogDecision::Continue,
    }
}

pub(super) fn try_mark_turn_watchdog_active(proc: &mut AgentProcess) -> bool {
    if proc.turn_watchdog_active {
        return false;
    }
    proc.turn_watchdog_active = true;
    true
}

pub(super) struct TimeoutFinalizeOutcome {
    pub(crate) completed: bool,
    pub(crate) continue_watchdog: bool,
    pub(crate) captured_pgid: Option<u32>,
}

pub(super) struct TimeoutFinalizeTransition {
    pub(crate) effect: Option<TurnCompleteTransition>,
    pub(crate) continue_watchdog: bool,
    pub(crate) captured_pgid: Option<u32>,
}

pub(super) fn run_timeout_finalize_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    now: Instant,
    emit_stream: F,
) -> TimeoutFinalizeTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let timeout = match turn_watchdog_decision(proc, captured_gen_id, captured_turn_seq, now) {
        TurnWatchdogDecision::Timeout(timeout) => timeout,
        TurnWatchdogDecision::Continue => {
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: true,
                captured_pgid: None,
            };
        }
        TurnWatchdogDecision::BreakClearFlag => {
            proc.turn_watchdog_active = false;
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: false,
                captured_pgid: None,
            };
        }
        TurnWatchdogDecision::BreakKeepFlag => {
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: false,
                captured_pgid: None,
            };
        }
    };

    #[cfg(unix)]
    let captured_pgid = proc.pgid;
    #[cfg(not(unix))]
    let captured_pgid = None;

    let effect = finalize_turn_as_timeout_locked(proc, chat_session_id, timeout, emit_stream);
    TimeoutFinalizeTransition {
        effect: Some(effect),
        continue_watchdog: false,
        captured_pgid,
    }
}

pub(super) async fn finalize_timed_out_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
) -> TimeoutFinalizeOutcome {
    let transition;
    {
        let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
        let mut map = handles.lock().await;
        let Some(proc) = map.get_mut(chat_session_id) else {
            return TimeoutFinalizeOutcome {
                completed: false,
                continue_watchdog: false,
                captured_pgid: None,
            };
        };
        transition = run_timeout_finalize_transition_locked(
            proc,
            chat_session_id,
            captured_gen_id,
            captured_turn_seq,
            Instant::now(),
            |mid, parts| emit_streaming_parts(app, chat_session_id, mid, parts.to_vec()),
        );
    }

    let Some(effect) = transition.effect else {
        return TimeoutFinalizeOutcome {
            completed: false,
            continue_watchdog: transition.continue_watchdog,
            captured_pgid: transition.captured_pgid,
        };
    };

    if !effect.turn_completed {
        return TimeoutFinalizeOutcome {
            completed: false,
            continue_watchdog: false,
            captured_pgid: transition.captured_pgid,
        };
    }

    complete_streaming_turn_post_lock(
        app,
        session_store,
        handles,
        chat_session_id,
        effect,
        TurnCompletePostOptions {
            consume_pending: true,
        },
    )
    .await;

    TimeoutFinalizeOutcome {
        completed: true,
        continue_watchdog: false,
        captured_pgid: transition.captured_pgid,
    }
}

pub(super) async fn recover_timed_out_bridge<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    captured_pgid: Option<u32>,
) {
    let interrupt_sent = match write_bridge_command_for_captured_turn(
        handles,
        chat_session_id,
        captured_gen_id,
        captured_turn_seq,
        serde_json::json!({ "type": "interrupt" }),
    )
    .await
    {
        Ok(sent) => sent,
        Err(e) => {
            log::warn!("Failed to interrupt timed-out bridge {chat_session_id}: {e}");
            false
        }
    };

    if interrupt_sent {
        tokio::time::sleep(Duration::from_secs(STALE_RECOVERY_GRACE_SECS)).await;
    }

    let (remove_current_pid_file, sweep_captured_pgid) = {
        let mut map = handles.lock().await;
        mark_timed_out_bridge_for_recovery_locked(
            &mut map,
            chat_session_id,
            captured_gen_id,
            captured_turn_seq,
            captured_pgid,
        )
    };

    #[cfg(unix)]
    {
        if let (true, Some(pg)) = (sweep_captured_pgid, captured_pgid) {
            sweep_process_group(pg).await;
        }
        if remove_current_pid_file {
            if let Ok(data_dir) = resolve_data_dir(app) {
                remove_pgid(&data_dir, chat_session_id);
            }
        }
    }

    #[cfg(not(unix))]
    {
        if remove_current_pid_file {
            let _ = app;
        }
    }
}

pub(super) fn mark_timed_out_bridge_for_recovery_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    captured_pgid: Option<u32>,
) -> (bool, bool) {
    if let Some(proc) = map.get_mut(chat_session_id) {
        if proc.generation_id == captured_gen_id
            && proc.turn_seq == captured_turn_seq
            && proc.state != BridgeState::Ready
        {
            proc.state = BridgeState::Crashed;
            proc.turn_phase = TurnPhase::Idle;
            proc.turn_watchdog_active = false;
            proc.last_progress_at = None;
            proc.mark_turn_phase_since_now();
            return (true, true);
        }
    }

    #[cfg(unix)]
    {
        let current_owns_captured_pgid = captured_pgid.is_some_and(|pg| {
            map.get(chat_session_id)
                .and_then(|proc| proc.pgid)
                .is_some_and(|current_pg| current_pg == pg)
        });
        (
            false,
            captured_pgid.is_some() && !current_owns_captured_pgid,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = captured_pgid;
        (false, false)
    }
}

pub(super) fn spawn_turn_watchdog<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    proc: &mut AgentProcess,
) {
    if !try_mark_turn_watchdog_active(proc) {
        return;
    }
    let app_watchdog = app.clone();
    let handles_watchdog = Arc::clone(handles);
    let session_store_watchdog = Arc::clone(session_store);
    let csid_watchdog = chat_session_id.to_string();
    let captured_gen_id = proc.generation_id;
    let captured_turn_seq = proc.turn_seq;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(WATCHDOG_TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let decision = {
                let mut map = handles_watchdog.lock().await;
                let Some(proc) = map.get_mut(&csid_watchdog) else {
                    break;
                };
                match turn_watchdog_decision(
                    proc,
                    captured_gen_id,
                    captured_turn_seq,
                    Instant::now(),
                ) {
                    TurnWatchdogDecision::BreakClearFlag => {
                        proc.turn_watchdog_active = false;
                        TurnWatchdogDecision::BreakClearFlag
                    }
                    other => other,
                }
            };

            match decision {
                TurnWatchdogDecision::Continue => {}
                TurnWatchdogDecision::BreakKeepFlag | TurnWatchdogDecision::BreakClearFlag => break,
                TurnWatchdogDecision::Timeout(_) => {
                    let outcome = finalize_timed_out_turn(
                        &app_watchdog,
                        &handles_watchdog,
                        &session_store_watchdog,
                        &csid_watchdog,
                        captured_gen_id,
                        captured_turn_seq,
                    )
                    .await;
                    if outcome.completed {
                        recover_timed_out_bridge(
                            &app_watchdog,
                            &handles_watchdog,
                            &csid_watchdog,
                            captured_gen_id,
                            captured_turn_seq,
                            outcome.captured_pgid,
                        )
                        .await;
                        break;
                    }
                    if outcome.continue_watchdog {
                        continue;
                    }
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod cleanup_gate_tests {
    use super::{wait_for_startup_orphan_cleanup, CleanupGate};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn cleanup_gate_waits_until_open_and_releases_all_waiters() {
        let gate = Arc::new(CleanupGate::new(false));
        let waiter_a = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                gate.wait_until_open().await;
            })
        };
        let waiter_b = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                gate.wait_until_open().await;
            })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter_a.is_finished());
        assert!(!waiter_b.is_finished());

        gate.open();

        tokio::time::timeout(Duration::from_secs(1), waiter_a)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), waiter_b)
            .await
            .unwrap()
            .unwrap();
        assert!(gate.is_open());
    }

    #[tokio::test]
    async fn cleanup_gate_open_state_returns_immediately() {
        let gate = CleanupGate::new(true);

        tokio::time::timeout(Duration::from_millis(20), gate.wait_until_open())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cleanup_gate_open_without_waiters_is_durable() {
        let gate = CleanupGate::new(false);

        gate.open();

        assert!(gate.is_open());
        tokio::time::timeout(Duration::from_millis(20), gate.wait_until_open())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closed_managed_cleanup_gate_blocks_spawn_closure_until_open() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

        let gate = Arc::new(CleanupGate::new(false));
        let app = tauri::test::mock_builder()
            .manage(Arc::clone(&gate))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let spawn_called = Arc::new(AtomicBool::new(false));
        let spawn_called_for_task = Arc::clone(&spawn_called);
        let app_handle = app.handle().clone();

        let task = tokio::spawn(async move {
            wait_for_startup_orphan_cleanup(&app_handle).await;
            spawn_called_for_task.store(true, AtomicOrdering::SeqCst);
            "spawned"
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!task.is_finished());
        assert!(!spawn_called.load(AtomicOrdering::SeqCst));

        gate.open();

        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, "spawned");
        assert!(spawn_called.load(AtomicOrdering::SeqCst));
    }

    #[tokio::test]
    async fn unregistered_cleanup_gate_returns_immediately() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let app_handle = app.handle().clone();

        tokio::time::timeout(
            Duration::from_millis(20),
            wait_for_startup_orphan_cleanup(&app_handle),
        )
        .await
        .expect("unregistered cleanup gate must not block spawn");
    }
}

#[cfg(test)]
mod tests {
    use super::super::process_registry::TurnPhase;
    use super::{evaluate_turn_liveness, TurnLivenessTimeout, STALE_TIMEOUT_SECS};
    use std::time::{Duration, Instant};

    #[test]
    fn recovery_liveness_marks_streaming_stale_after_timeout() {
        let now = Instant::now();
        let stale_at = now - Duration::from_secs(STALE_TIMEOUT_SECS + 1);

        assert_eq!(
            evaluate_turn_liveness(TurnPhase::Streaming, Some(stale_at), now, now),
            Some(TurnLivenessTimeout::Stale)
        );
    }

    #[test]
    fn claude_bridge_watchdog_env_uses_existing_native_levers() {
        let env = super::claude_bridge_watchdog_env_overrides();
        let keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();

        assert_eq!(
            keys,
            vec![
                "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
                "CLAUDE_ENABLE_STREAM_WATCHDOG",
                "CLAUDE_ENABLE_BYTE_WATCHDOG",
                "CLAUDE_CODE_MAX_RETRIES",
                "API_TIMEOUT_MS",
            ]
        );
        assert!(!keys.contains(&"CLAUDE_CODE_STREAM_CLOSE_TIMEOUT"));
        assert_eq!(
            env.iter()
                .find_map(|(key, value)| (*key == "CLAUDE_ENABLE_STREAM_WATCHDOG")
                    .then_some(value.as_str())),
            Some("1")
        );
        assert_eq!(
            env.iter()
                .find_map(|(key, value)| (*key == "CLAUDE_STREAM_IDLE_TIMEOUT_MS")
                    .then_some(value.as_str())),
            Some("180000")
        );
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::process_registry::*;
    use super::super::recovery::*;

    use super::super::session_lifecycle::*;

    use super::super::shared::test_support::*;
    use super::super::turn_event_log::*;

    use crate::usecase::agent_session::event_log::WorkflowTurnCompleteInput;

    use crate::usecase::agent_session::session::MessagePart;

    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::sync::Mutex;

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_at_most_once_for_concurrent_sends() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let session_id = "step-session".to_string();

        let first = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        });
        let second = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        });

        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key("step-session"));
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_removes_partial_runtime_when_spawn_fails() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "step-session-spawn-fail".to_string();

        let result = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Err("spawn failed".to_string())
            }
        })
        .await;

        assert_eq!(result.unwrap_err(), "spawn failed");
        assert!(!handles.lock().await.contains_key("step-session-spawn-fail"));
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_when_ready_idle_child_exited_before_eof() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-idle-exited-before-eof".to_string();
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ready = make_test_agent_process();
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready
            .pending_messages
            .push_back(test_pending_message("queued-after-result", "continue"));
        ready.child.start_kill().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        handles.lock().await.insert(session_id.clone(), ready);

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut proc = handles
            .lock()
            .await
            .remove("ready-idle-exited-before-eof")
            .unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        assert_eq!(
            proc.pending_messages.pop_front().unwrap().id,
            "queued-after-result"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_after_ready_eof_eviction() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-eof-evicted".to_string();
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ready = make_test_agent_process();
        let _ = ready.child.kill().await;
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready.generation_id = 42;
        handles.lock().await.insert(session_id.clone(), ready);

        {
            let mut map = handles.lock().await;
            let should_evict = {
                let proc = map.get_mut(&session_id).unwrap();
                let generation_matches = proc.generation_id == 42;
                let transition = run_bridge_eof_crash_transition_locked(
                    generation_matches,
                    proc,
                    &session_id,
                    |_mid, _parts| (true, true),
                );
                let should_evict = transition.should_evict;
                assert!(should_evict);
                should_evict
            };
            if should_evict {
                let removed = retire_ready_eof_runtime_locked(&mut map, &session_id);
                assert!(removed);
            }
            assert!(!map.contains_key(&session_id));
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key("ready-eof-evicted"));
        let mut spawned = handles.lock().await.remove("ready-eof-evicted").unwrap();
        let _ = spawned.child.kill().await;
    }

    #[tokio::test]
    async fn ready_eof_with_pending_queue_preserves_pending_when_respawning() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-eof-pending".to_string();
        let mut ready = make_test_agent_process();
        let _ = ready.child.kill().await;
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready.generation_id = 7;
        ready
            .pending_messages
            .push_back(test_pending_message("queued-2", "second pending"));
        ready
            .pending_messages
            .push_back(test_pending_message("queued-3", "third pending"));
        handles.lock().await.insert(session_id.clone(), ready);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session_id).unwrap();
            let generation_matches = proc.generation_id == 7;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session_id,
                |_mid, _parts| (true, true),
            );
            assert!(transition.should_evict);
            let removed = retire_ready_eof_runtime_locked(&mut map, &session_id);
            assert!(!removed);

            let proc = map.get(&session_id).unwrap();
            assert_eq!(proc.state, BridgeState::Crashed);
            assert_eq!(proc.turn_phase, TurnPhase::Idle);
            assert_eq!(proc.pending_messages.len(), 2);
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles.lock().await.remove("ready-eof-pending").unwrap();
        let pending_ids: Vec<&str> = proc
            .pending_messages
            .iter()
            .map(|pending| pending.id.as_str())
            .collect();
        assert_eq!(pending_ids, vec!["queued-2", "queued-3"]);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_preserves_pending_when_replacing_crashed_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "crashed-with-pending".to_string();
        let mut crashed = make_test_agent_process();
        let _ = crashed.child.kill().await;
        crashed.state = BridgeState::Crashed;
        crashed.pending_messages.push_back(PendingMessage {
            id: "queued-before-crash".to_string(),
            content: "continue after reinject".to_string(),
            created_at: 1.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert(session_id.clone(), crashed);

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles.lock().await.remove("crashed-with-pending").unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        let pending = proc.pending_messages.pop_front().unwrap();
        assert_eq!(pending.id, "queued-before-crash");
        assert_eq!(pending.content, "continue after reinject");
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timed_out_recovery_preserves_remaining_pending_messages_for_replacement() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "timeout-with-multiple-pending".to_string();
        let mut timed_out = make_test_agent_process();
        let _ = timed_out.child.kill().await;
        timed_out.generation_id = 42;
        timed_out.turn_seq = 7;
        timed_out.state = BridgeState::Crashed;
        timed_out.turn_phase = TurnPhase::Idle;
        timed_out
            .pending_messages
            .push_back(pending_message_for_test(
                "queued-after-timeout-1",
                "first remaining",
                1.0,
            ));
        timed_out
            .pending_messages
            .push_back(pending_message_for_test(
                "queued-after-timeout-2",
                "second remaining",
                2.0,
            ));
        handles.lock().await.insert(session_id.clone(), timed_out);

        {
            let mut map = handles.lock().await;
            let (remove_pid_file, _sweep_pgid) =
                mark_timed_out_bridge_for_recovery_locked(&mut map, &session_id, 42, 7, None);
            assert!(remove_pid_file);
            assert!(
                map.contains_key(&session_id),
                "timeout recovery must retain crashed runtime so pending queue can be preserved"
            );
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles
            .lock()
            .await
            .remove("timeout-with-multiple-pending")
            .unwrap();
        let pending_ids = proc
            .pending_messages
            .iter()
            .map(|pending| pending.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            pending_ids,
            vec!["queued-after-timeout-1", "queued-after-timeout-2"]
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_fresh_runtime_after_timeout_crash() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "timed-out-runtime".to_string();
        let mut timed_out = make_test_agent_process();
        let _ = timed_out.child.kill().await;
        timed_out.state = BridgeState::Crashed;
        timed_out.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert(session_id.clone(), timed_out);
        let spawn_count = Arc::new(AtomicUsize::new(0));

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let mut proc = handles.lock().await.remove("timed-out-runtime").unwrap();
        assert_eq!(proc.state, BridgeState::Ready);
        let _ = proc.child.kill().await;
    }

    #[test]
    fn liveness_marks_streaming_stale_after_last_progress_timeout() {
        let now = Instant::now();
        let timeout = evaluate_turn_liveness(
            TurnPhase::Streaming,
            Some(now - Duration::from_secs(STALE_TIMEOUT_SECS + 1)),
            now - Duration::from_secs(STALE_TIMEOUT_SECS + 1),
            now,
        );

        assert_eq!(timeout, Some(TurnLivenessTimeout::Stale));
    }

    #[test]
    fn liveness_keeps_streaming_alive_when_progress_is_recent() {
        let now = Instant::now();
        let timeout = evaluate_turn_liveness(
            TurnPhase::Streaming,
            Some(now - Duration::from_secs(STALE_TIMEOUT_SECS - 1)),
            now - Duration::from_secs(STALE_TIMEOUT_SECS + 10),
            now,
        );

        assert_eq!(timeout, None);
    }

    #[test]
    fn liveness_keeps_waiting_permission_alive_after_long_wait() {
        let now = Instant::now();
        let since = now - Duration::from_secs(3600);

        assert_eq!(
            evaluate_turn_liveness(TurnPhase::WaitingPermission, None, since, now),
            None
        );
    }

    #[test]
    fn liveness_keeps_idle_alive() {
        let now = Instant::now();

        assert_eq!(
            evaluate_turn_liveness(TurnPhase::Idle, None, now - Duration::from_secs(3600), now),
            None
        );
    }

    #[tokio::test]
    async fn touch_liveness_resets_streaming_stale_clock() {
        let mut proc = make_streaming_test_process();
        let stale_base = Instant::now() - Duration::from_secs(STALE_TIMEOUT_SECS + 1);
        proc.last_progress_at = Some(stale_base);
        proc.turn_phase_since = stale_base;

        assert_eq!(
            turn_watchdog_decision(&proc, proc.generation_id, proc.turn_seq, Instant::now()),
            TurnWatchdogDecision::Timeout(TurnLivenessTimeout::Stale)
        );

        proc.touch_liveness();
        assert_eq!(
            turn_watchdog_decision(&proc, proc.generation_id, proc.turn_seq, Instant::now()),
            TurnWatchdogDecision::Continue
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timeout_finalize_rechecks_liveness_and_continues_when_progress_arrives_after_decision()
    {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        proc.turn_watchdog_active = true;
        let captured_gen_id = proc.generation_id;
        let captured_turn_seq = proc.turn_seq;
        let stale_base = Instant::now() - Duration::from_secs(STALE_TIMEOUT_SECS + 1);
        proc.last_progress_at = Some(stale_base);
        proc.turn_phase_since = stale_base;

        let decision_now = Instant::now();
        assert_eq!(
            turn_watchdog_decision(&proc, captured_gen_id, captured_turn_seq, decision_now),
            TurnWatchdogDecision::Timeout(TurnLivenessTimeout::Stale)
        );

        proc.last_progress_at = Some(decision_now);
        let mut events = Vec::new();
        let transition = run_timeout_finalize_transition_locked(
            &mut proc,
            "csid",
            captured_gen_id,
            captured_turn_seq,
            decision_now,
            recording_emit(&mut events),
        );

        assert!(transition.effect.is_none());
        assert!(transition.continue_watchdog);
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert!(proc.turn_watchdog_active);
        assert!(proc.streaming_parts.is_empty());
        assert!(events.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn finalize_timeout_adds_error_part_and_completes_as_failure() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_test_turn_event_log(&mut proc);
        let partial = MessagePart::Text {
            content: "partial response".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(partial.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&partial));
        let mut events = Vec::new();

        let effect = finalize_turn_as_timeout_locked(
            &mut proc,
            "csid",
            TurnLivenessTimeout::Stale,
            recording_emit(&mut events),
        );

        assert!(effect.turn_completed);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert!(effect.final_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content == "partial response"
        )));
        assert!(effect.final_parts.iter().any(|part| matches!(
            part,
            MessagePart::Error { content, .. } if content == STALE_ERROR_MESSAGE
        )));
        assert_eq!(proc.pending_stream_part_count, 0);
        assert!(!proc.turn_watchdog_active);
        assert_eq!(events.len(), 1);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn late_turn_complete_after_timeout_does_not_restore_ready_state() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        let _ = finalize_turn_as_timeout_locked(
            &mut proc,
            "csid",
            TurnLivenessTimeout::Stale,
            |_mid, _parts| (true, true),
        );

        let effect =
            run_turn_complete_transition_locked(&mut proc, "csid", 0, |_mid, _parts| (true, true));

        assert!(!effect.turn_completed);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timed_out_recovery_interrupt_is_scoped_to_captured_turn() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.generation_id = 2;
        proc.turn_seq = 7;
        handles.lock().await.insert("csid".to_string(), proc);

        let sent = write_bridge_command_for_captured_turn(
            &handles,
            "csid",
            1,
            6,
            serde_json::json!({ "type": "interrupt" }),
        )
        .await
        .unwrap();

        assert!(!sent, "must not interrupt a later bridge turn");
        let mut proc = handles.lock().await.remove("csid").unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn eof_crash_workflow_exit_code_is_projected_from_interrupted_event() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        begin_turn_event_log(&mut proc, "human-1", test_prompt_input("prompt"), "m1", 1.0);

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(
            transition.turn_complete.workflow_turn_complete,
            Some(WorkflowTurnCompleteInput {
                turn_id: 1,
                exit_code: -1,
                final_text_parts: Vec::new(),
                token_usage: None,
                interrupted: true,
            })
        );
    }

    #[tokio::test]
    async fn bridge_eof_crash_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ストリーミング → クラッシュ):
        //   Bridge process EOF クラッシュ経路では、未配信 delta + 合成 error
        //   part が同一 cumulative payload として state 通知 (Idle) より前に
        //   フロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        begin_test_turn_event_log(&mut proc);
        let pending_text = MessagePart::Text {
            content: "tail-before-eof".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));

        let mut events = Vec::new();
        drive_bridge_eof_crash_path(&mut proc, "csid", &mut events);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                // cumulative: pending Text + EOF transition が積んだ Error。
                assert_eq!(*parts_count, 2);
                assert!(
                    tail_text
                        .as_deref()
                        .unwrap_or("")
                        .contains("Bridge process exited unexpectedly"),
                    "tail must be the synthetic EOF error part, got {tail_text:?}"
                );
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(-1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.pending_stream_part_count, 0);
    }

    #[tokio::test]
    async fn bridge_eof_crash_adds_error_part_for_streaming_message() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("message-1".to_string());
        begin_test_turn_event_log(&mut proc);
        proc.streaming_parts.push(MessagePart::Text {
            content: "partial".to_string(),
            parent_tool_use_id: None,
        });

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(transition.turn_complete.turn_completed);
        assert_eq!(
            transition.turn_complete.final_msg_id.as_deref(),
            Some("message-1")
        );
        assert_eq!(transition.turn_complete.final_parts.len(), 2);
        assert!(transition
            .sdk_error_message
            .as_deref()
            .unwrap()
            .contains("mock"));
        assert!(!transition.should_evict);
        assert!(matches!(
            &transition.turn_complete.final_parts[1],
            MessagePart::Error { content, .. }
                if content.contains("Bridge process exited unexpectedly")
        ));
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_crash_marks_initializing_without_streaming_part() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(transition.was_initializing);
        assert!(!transition.should_evict);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_some());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_ready_idle_requests_eviction_without_error() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        // Ready/Idle EOF leaves the state untouched but flags the runtime for eviction.
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(!transition.was_initializing);
        assert!(transition.should_evict);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_none());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_generation_mismatch_does_not_evict_or_mutate() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(false, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(!transition.should_evict);
        assert!(!transition.was_initializing);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_none());
        let _ = proc.child.kill().await;
    }

    #[cfg(unix)]
    mod process_group_tests {
        use super::super::super::model_selection::set_active_process_model;
        use super::super::super::process_registry::{
            AgentProcess, AgentProcessMap, BridgeState, TurnPhase,
        };
        use super::super::super::session_lifecycle::{
            force_kill_all_sessions, set_session_backend_internal,
        };
        use super::super::super::shared::test_support::MockModelBackend;
        use super::super::super::shared::{CLAUDE_BACKEND_ID, CODEX_BACKEND_ID};
        use super::super::{
            cleanup_orphan_processes, get_process_start_time, pids_dir, remove_pgid, save_pgid,
            wait_for_startup_orphan_cleanup, CleanupGate, OrphanCleanupReport, PidFileV1,
        };
        use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, ModelInfo};
        use crate::usecase::agent_session::event_log::TurnEventLog;
        use crate::usecase::agent_session::session::create_session_internal;
        use std::collections::{HashMap, VecDeque};
        use std::os::unix::process::CommandExt as _;
        use std::path::Path;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        use tokio::sync::Mutex;

        /// PID belonging to a process guaranteed not to exist. PIDs on Linux
        /// and macOS are capped well below this value, so `kill(pid, 0)` will
        /// always return ESRCH.
        const DEAD_OWNER_PID: u32 = 999_999_999;

        /// Helper: serialize a PidFileV1 payload to the given path.
        fn write_pid_file_v1(path: &Path, payload: &PidFileV1) {
            std::fs::write(path, serde_json::to_string(payload).unwrap()).unwrap();
        }

        #[test]
        fn cleanup_orphan_process_logs_do_not_format_pid_file_paths_or_process_ids() {
            let source = include_str!("recovery.rs");
            let (_, after_start) = source
                .split_once("pub fn cleanup_orphan_processes")
                .expect("cleanup function source should be present");
            let (cleanup_source, _) = after_start
                .split_once("\npub(super) struct BridgeEofCrashTransition")
                .expect("cleanup function end marker should be present");

            assert!(!cleanup_source.contains("path.display()"));
            assert!(!cleanup_source.contains("pid={}"));
            assert!(!cleanup_source.contains("owner pid"));
            assert!(!cleanup_source.contains("{pgid}"));
            assert!(!cleanup_source.contains("Invalid PGID {}"));
        }

        #[test]
        fn save_and_remove_pgid() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();

            save_pgid(app_data_dir, "session-1", 12345).unwrap();

            let pid_file = pids_dir(app_data_dir).join("session-1.pid");
            assert!(pid_file.exists());
            let contents = std::fs::read_to_string(&pid_file).unwrap();
            let parsed: PidFileV1 = serde_json::from_str(&contents).unwrap();
            assert_eq!(parsed.version, 1);
            assert_eq!(parsed.pgid, 12345);
            assert_eq!(parsed.owner_app_pid, std::process::id());

            remove_pgid(app_data_dir, "session-1");
            assert!(!pid_file.exists());
        }

        #[test]
        fn save_pgid_writes_owner_app_pid_and_start_time() {
            // issue #1024: PID files must identify their owning Releash
            // instance so cleanup can distinguish self-orphans from files
            // belonging to a different live instance.
            let tmp = tempfile::tempdir().unwrap();
            save_pgid(tmp.path(), "owner-test", 42_424).unwrap();
            let contents =
                std::fs::read_to_string(pids_dir(tmp.path()).join("owner-test.pid")).unwrap();
            let parsed: PidFileV1 = serde_json::from_str(&contents).unwrap();
            assert_eq!(parsed.owner_app_pid, std::process::id());
            assert!(
                parsed.owner_start_time > 0,
                "owner_start_time should be populated on supported platforms"
            );
            // The recorded start_time must match what we can read back now.
            let live_start = get_process_start_time(std::process::id()).unwrap();
            assert_eq!(parsed.owner_start_time, live_start);
        }

        #[test]
        fn save_pgid_rejects_path_traversal() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();

            assert!(save_pgid(app_data_dir, "../escape", 12345).is_err());
            assert!(save_pgid(app_data_dir, "a/b", 12345).is_err());
            assert!(save_pgid(app_data_dir, "", 12345).is_err());
            assert!(save_pgid(app_data_dir, "valid-session-id", 12345).is_ok());
        }

        #[test]
        fn cleanup_orphan_processes_removes_stale_pid_files() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // Owner pid is guaranteed dead → cleanup proceeds and removes the
            // file even though the pgid itself doesn't refer to a live group.
            let pid_file = dir.join("stale-session.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 999_999_999,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );
            assert!(pid_file.exists());

            let report = cleanup_orphan_processes(app_data_dir);

            // PID file should be removed
            assert!(!pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_orphan_processes_handles_empty_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let report = cleanup_orphan_processes(app_data_dir);
            assert_eq!(report, OrphanCleanupReport::default());
        }

        #[test]
        fn cleanup_orphan_processes_handles_no_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path().join("nonexistent");

            let report = cleanup_orphan_processes(&app_data_dir);
            assert_eq!(report, OrphanCleanupReport::default());
        }

        #[test]
        fn cleanup_orphan_processes_ignores_non_pid_files() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let other_file = dir.join("notes.txt");
            std::fs::write(&other_file, "not a pid").unwrap();

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(other_file.exists());
            assert_eq!(report, OrphanCleanupReport::default());
        }

        #[test]
        fn cleanup_orphan_processes_counts_read_failure_for_pid_directory() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::create_dir(dir.join("unreadable.pid")).unwrap();

            let report = cleanup_orphan_processes(app_data_dir);

            assert_eq!(report.scanned, 1);
            assert_eq!(report.processed, 0);
            assert_eq!(report.skipped, 0);
            assert!(
                report.failures > 0,
                "read_to_string failure for a .pid directory must be counted"
            );
        }

        /// Spawn a process in a new process group via setsid(), verify it
        /// becomes a process group leader (pgid == pid), then verify that
        /// killpg terminates the entire group.
        #[test]
        fn setsid_creates_new_process_group_leader() {
            use std::process::Command;

            let child = unsafe {
                Command::new("sleep")
                    .arg("999")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };

            let pid = child.id() as libc::pid_t;

            // After setsid(), the child's PGID should equal its PID
            let pgid = unsafe { libc::getpgid(pid) };
            assert_eq!(
                pgid, pid,
                "setsid child should be its own process group leader"
            );

            // killpg should successfully terminate the group
            let ret = unsafe { libc::killpg(pid, libc::SIGKILL) };
            assert_eq!(ret, 0, "killpg should succeed");

            // Reap the child
            let mut child = child;
            let _ = child.wait();

            // Verify process is gone
            let alive = unsafe { libc::kill(pid, 0) };
            assert_ne!(alive, 0, "process should be terminated");
        }

        /// Verify killpg kills grandchild processes within the same group.
        #[test]
        fn killpg_kills_grandchild_processes() {
            use std::process::Command;

            // Spawn a shell that itself spawns a grandchild (sleep).
            // Both shell and sleep will be in the new process group.
            let child = unsafe {
                Command::new("sh")
                    .args(["-c", "sleep 999 & wait"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };

            let pgid = child.id() as libc::pid_t;

            // Give the grandchild time to spawn
            std::thread::sleep(std::time::Duration::from_millis(200));

            // Kill the entire process group
            let ret = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            assert_eq!(ret, 0, "killpg should succeed");

            // Reap the child
            let mut child = child;
            let _ = child.wait();

            // Verify no processes remain in this group
            std::thread::sleep(std::time::Duration::from_millis(100));
            let group_alive = unsafe { libc::killpg(pgid, 0) };
            assert_ne!(
                group_alive, 0,
                "no processes should remain in the killed group"
            );
        }

        /// Spawn `sleep 999` in its own process group (setsid). Returns the
        /// `Child` and the pgid (== child PID after setsid).
        fn spawn_setsid_sleep() -> (std::process::Child, libc::pid_t) {
            use std::process::Command;
            let child = unsafe {
                Command::new("sleep")
                    .arg("999")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };
            let pgid = child.id() as libc::pid_t;
            (child, pgid)
        }

        #[tokio::test]
        async fn cleanup_gate_blocks_new_child_until_cleanup_finishes_and_self_owned_child_survives(
        ) {
            use tokio::sync::oneshot;
            use tokio::sync::oneshot::error::TryRecvError;

            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path().to_path_buf();
            let dir = pids_dir(&app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();
            write_pid_file_v1(
                &dir.join("stale-before-spawn.pid"),
                &PidFileV1 {
                    version: 1,
                    pgid: 999_999_999,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            let gate = Arc::new(CleanupGate::new(false));
            let app = tauri::test::mock_builder()
                .manage(Arc::clone(&gate))
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let app_handle = app.handle().clone();
            let (waiting_tx, waiting_rx) = oneshot::channel();
            let (spawned_tx, mut spawned_rx) = oneshot::channel();
            let app_data_dir_for_spawn = app_data_dir.clone();

            let task = tokio::spawn(async move {
                let _ = waiting_tx.send(());
                wait_for_startup_orphan_cleanup(&app_handle).await;
                let (child, pgid) = spawn_setsid_sleep();
                save_pgid(&app_data_dir_for_spawn, "new-child", pgid as u32).unwrap();
                let _ = spawned_tx.send((child, pgid));
            });

            waiting_rx.await.unwrap();
            assert!(matches!(spawned_rx.try_recv(), Err(TryRecvError::Empty)));
            assert!(!pids_dir(&app_data_dir).join("new-child.pid").exists());

            let cleanup_before_spawn = cleanup_orphan_processes(&app_data_dir);
            assert_eq!(cleanup_before_spawn.scanned, 1);
            assert_eq!(cleanup_before_spawn.processed, 1);
            assert_eq!(cleanup_before_spawn.failures, 0);

            gate.open();
            let (mut child, pgid) = tokio::time::timeout(Duration::from_secs(1), spawned_rx)
                .await
                .unwrap()
                .unwrap();
            task.await.unwrap();

            let cleanup_after_spawn = cleanup_orphan_processes(&app_data_dir);
            assert_eq!(
                cleanup_after_spawn,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 0,
                    skipped: 1,
                    failures: 0,
                }
            );
            assert_eq!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "self-owned child spawned after gate open must survive cleanup"
            );

            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
            let _ = child.wait();
        }

        #[test]
        fn cleanup_orphan_processes_kills_alive_process_group() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let (mut child, pgid) = spawn_setsid_sleep();

            // Owner is a dead PID, so cleanup treats this as a self-orphan and
            // must terminate the bridge group + delete the file.
            let pid_file = dir.join("alive-session.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            assert_eq!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "process group should be alive before cleanup"
            );

            let report = cleanup_orphan_processes(app_data_dir);

            // Reap the child to clear zombie state from process table.
            // cleanup_orphan_processes sends SIGTERM/SIGKILL via killpg, but without
            // wait() the child becomes a zombie and killpg(pgid, 0) still returns 0.
            let _ = child.wait();

            let still_alive = unsafe { libc::killpg(pgid, 0) };
            assert_ne!(
                still_alive, 0,
                "process group should be terminated after cleanup"
            );
            assert!(!pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_skips_pid_file_owned_by_live_other_instance() {
            // issue #1024: A PID file whose owner_app_pid points at a live
            // process with matching start_time belongs to a different,
            // currently-running Releash instance. Cleanup must not touch it.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // Stand-in for "another Releash instance": use the current test
            // process itself as the owner — it is alive and its start_time
            // matches what get_process_start_time returns.
            let owner_pid = std::process::id();
            let owner_start_time = get_process_start_time(owner_pid).unwrap();

            // Stand-in for the bridge process group owned by that instance.
            let (mut bridge, pgid) = spawn_setsid_sleep();

            let pid_file = dir.join("foreign.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: owner_pid,
                    owner_start_time,
                },
            );

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(
                pid_file.exists(),
                "PID file owned by a live instance must be left in place"
            );
            assert_eq!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "bridge process group of a live instance must not be killed"
            );
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 0,
                    skipped: 1,
                    failures: 0
                }
            );

            // Tear down the helper process so it doesn't outlive the test run.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
            let _ = bridge.wait();
        }

        #[test]
        fn cleanup_kills_when_owner_pid_was_reused() {
            // owner_app_pid is alive (we point it at ourselves) but the
            // recorded owner_start_time disagrees with reality → PID has been
            // reused. The file is stale and must be cleaned up.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let (mut child, pgid) = spawn_setsid_sleep();

            let pid_file = dir.join("reused.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: std::process::id(),
                    owner_start_time: 0, // definitely != real start_time
                },
            );

            let report = cleanup_orphan_processes(app_data_dir);
            let _ = child.wait();

            assert!(!pid_file.exists());
            assert_ne!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "stale-owner bridge group should be terminated"
            );
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_skips_legacy_numeric_pid_files() {
            // Files written by older builds had no owner info, so cleanup
            // cannot prove the owner is dead. They must be left in place
            // (conservative) — they get either overwritten on next save or
            // cleaned up manually by the developer.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let pid_file = dir.join("legacy.pid");
            std::fs::write(&pid_file, "999999999").unwrap();

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 0,
                    skipped: 1,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_orphan_processes_ignores_invalid_pgid_zero() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // pgid=0 would target the caller's own group — must be rejected.
            let pid_file = dir.join("bad-zero.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 0,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_orphan_processes_ignores_invalid_pgid_one() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // pgid=1 (init) — must be rejected.
            let pid_file = dir.join("bad-one.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 1,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        #[test]
        fn cleanup_orphan_processes_ignores_negative_pgid() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let pid_file = dir.join("bad-negative.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: -1,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            let report = cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
            assert_eq!(
                report,
                OrphanCleanupReport {
                    scanned: 1,
                    processed: 1,
                    skipped: 0,
                    failures: 0
                }
            );
        }

        fn make_dummy_agent_process(
            child: tokio::process::Child,
            stdin: tokio::process::ChildStdin,
            pgid: Option<u32>,
        ) -> AgentProcess {
            AgentProcess {
                stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
                backend_id: CLAUDE_BACKEND_ID.to_string(),
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: None,
                context_carry_on_ready: None,
                child,
                generation_id: 1,
                pgid,
                streaming_message_id: None,
                active_turn_token: None,
                turn_latency: None,
                post_turn_message_token: None,
                streaming_parts: Vec::new(),
                turn_event_log: TurnEventLog::default(),
                last_message_id: None,
                post_turn_base_untrusted_message_id: None,
                task_id_map: HashMap::new(),
                pending_messages: VecDeque::new(),
                current_permission_mode: "ask".to_string(),
                available_models: Vec::new(),
                selected_model: None,
                last_result_token_usage: None,
                latest_token_usage: None,
                pending_stream_part_count: 0,
                pending_stream_bytes: 0,
                last_stream_emit_at: None,
                streaming_timer_active: false,
                last_progress_at: None,
                turn_phase_since: Instant::now(),
                turn_seq: 0,
                turn_watchdog_active: false,
            }
        }

        #[tokio::test]
        async fn set_active_process_model_updates_selected_model() {
            let mut cmd = tokio::process::Command::new("cat");
            cmd.stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = cmd.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let pgid = child.id();

            let handles = Arc::new(Mutex::new(HashMap::new()));
            {
                let mut map = handles.lock().await;
                let mut proc = make_dummy_agent_process(child, stdin, pgid);
                proc.available_models = vec![ModelInfo::new(CODEX_BACKEND_ID, "gpt-5.4")];
                proc.selected_model = Some("old-model".to_string());
                map.insert("session-1".to_string(), proc);
            }

            set_active_process_model(&handles, "session-1", "gpt-5.4".to_string())
                .await
                .unwrap();

            {
                let map = handles.lock().await;
                let proc = map.get("session-1").unwrap();
                assert_eq!(proc.selected_model, Some("gpt-5.4".to_string()));
                // available_models は process キャッシュとして変更されないこと（owner は config）。
                assert_eq!(proc.available_models[0].model_id, "gpt-5.4");
            }

            let mut map = handles.lock().await;
            force_kill_all_sessions(&mut map).await;
        }

        #[tokio::test]
        async fn set_active_process_model_skips_selected_model_when_process_is_replaced_after_io() {
            let mut old_cmd = tokio::process::Command::new("cat");
            old_cmd
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let mut old_child = old_cmd.spawn().unwrap();
            let old_stdin = old_child.stdin.take().unwrap();

            let mut new_cmd = tokio::process::Command::new("cat");
            new_cmd
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let mut new_child = new_cmd.spawn().unwrap();
            let new_stdin = new_child.stdin.take().unwrap();

            let handles = Arc::new(Mutex::new(HashMap::new()));
            let mut old_proc = make_dummy_agent_process(old_child, old_stdin, None);
            old_proc.generation_id = 1;
            let old_writer = Arc::clone(&old_proc.stdin);
            let old_writer_guard = old_writer.lock().await;

            let mut map_guard = handles.lock().await;
            map_guard.insert("session-1".to_string(), old_proc);
            let update_task = {
                let handles = Arc::clone(&handles);
                tokio::spawn(async move {
                    set_active_process_model(&handles, "session-1", "new-model".to_string()).await
                })
            };
            tokio::task::yield_now().await;
            drop(map_guard);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;

            let mut replacement = make_dummy_agent_process(new_child, new_stdin, None);
            replacement.generation_id = 2;
            replacement.selected_model = Some("replacement-model".to_string());
            let mut old_proc = handles
                .lock()
                .await
                .insert("session-1".to_string(), replacement)
                .expect("old process should be replaced");

            drop(old_writer_guard);
            update_task.await.unwrap().unwrap();

            {
                let map = handles.lock().await;
                let proc = map.get("session-1").unwrap();
                assert_eq!(proc.selected_model.as_deref(), Some("replacement-model"));
            }

            let _ = old_proc.child.kill().await;
            let mut map = handles.lock().await;
            force_kill_all_sessions(&mut map).await;
        }

        #[tokio::test]
        async fn set_active_process_model_inactive_session_is_ok() {
            let handles = Arc::new(Mutex::new(HashMap::new()));

            // 該当 session が無くてもエラーにせず Ok(()) を返す（active 不在は no-op）。
            set_active_process_model(&handles, "missing", "gpt-5.4".to_string())
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn set_session_backend_removes_stale_unstarted_process() {
            let temp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(crate::test_support::build_session_store());
            let session = create_session_internal(
                &session_store,
                temp.path(),
                "/repo",
                Some(CLAUDE_BACKEND_ID.to_string()),
            )
            .unwrap();

            let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
            cfg.agents.codex.models = vec!["b-model".to_string()];
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
                cfg,
                tmp.path().to_path_buf(),
            ));

            let mut registry = AgentBackendRegistry::new();
            registry.register(Arc::new(MockModelBackend {
                backend_id: CLAUDE_BACKEND_ID.to_string(),
                models: Vec::new(),
            }));
            registry.register(Arc::new(MockModelBackend {
                backend_id: CODEX_BACKEND_ID.to_string(),
                models: Vec::new(),
            }));
            registry.set_config(config);
            let registry = Arc::new(registry);

            let mut cmd = tokio::process::Command::new("sleep");
            cmd.arg("999")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = cmd.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let pid = child.id();
            save_pgid(temp.path(), &session.id, pid.unwrap()).unwrap();

            let handles = Arc::new(Mutex::new(HashMap::new()));
            {
                let mut map = handles.lock().await;
                map.insert(
                    session.id.clone(),
                    make_dummy_agent_process(child, stdin, pid),
                );
            }

            let response = set_session_backend_internal(
                &session_store,
                &registry,
                &handles,
                temp.path(),
                &session.id,
                CODEX_BACKEND_ID.to_string(),
            )
            .await
            .unwrap();

            assert_eq!(
                response.session.backend_id,
                Some(CODEX_BACKEND_ID.to_string())
            );
            assert_eq!(response.available_models[0].model_id, "b-model");
            assert!(handles.lock().await.get(&session.id).is_none());
            assert!(!pids_dir(temp.path())
                .join(format!("{}.pid", session.id))
                .exists());
        }

        /// Spawn processes with setsid into AgentProcessMap, then verify
        /// force_kill_all_sessions actually terminates them.
        #[tokio::test]
        async fn force_kill_all_sessions_clears_map_and_kills_processes() {
            let mut map: AgentProcessMap = HashMap::new();
            let mut pids: Vec<u32> = Vec::new();

            for id in ["sess-a", "sess-b"] {
                let mut cmd = tokio::process::Command::new("sleep");
                cmd.arg("999")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let mut child = cmd.spawn().unwrap();
                let stdin = child.stdin.take().unwrap();
                let pid = child.id();
                if let Some(p) = pid {
                    pids.push(p);
                }
                map.insert(id.to_string(), make_dummy_agent_process(child, stdin, pid));
            }

            assert_eq!(map.len(), 2);

            let returned_ids = force_kill_all_sessions(&mut map).await;

            assert!(map.is_empty());
            assert_eq!(returned_ids.len(), 2);
            assert!(returned_ids.contains(&"sess-a".to_string()));
            assert!(returned_ids.contains(&"sess-b".to_string()));

            // Give processes time to be reaped
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // Verify all processes are actually dead
            for pid in &pids {
                let alive = unsafe { libc::kill(*pid as libc::pid_t, 0) };
                assert_ne!(
                    alive, 0,
                    "process {pid} should be terminated after force_kill_all_sessions"
                );
            }
        }

        #[tokio::test]
        async fn force_kill_all_sessions_handles_empty_map() {
            let mut map: AgentProcessMap = HashMap::new();

            let returned_ids = force_kill_all_sessions(&mut map).await;

            assert!(map.is_empty());
            assert!(returned_ids.is_empty());
        }
    }
}
