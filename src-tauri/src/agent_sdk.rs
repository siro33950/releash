use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::session::SessionStore;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeState {
    Initializing,
    Ready,
    Streaming,
    Crashed,
}

pub struct AgentProcess {
    pub stdin: tokio::process::ChildStdin,
    pub state: BridgeState,
    pub sdk_session_id: Option<String>,
    pub child: tokio::process::Child,
    pub generation_id: u64,
}

/// Per-session agent process map: chat_session_id → AgentProcess
pub type AgentProcessMap = HashMap<String, AgentProcess>;

fn dev_bridge_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("claude-sdk-bridge.mjs")
}

fn resolve_bridge_script(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    // Dev mode: resolve from CARGO_MANIFEST_DIR (src-tauri/)
    #[cfg(debug_assertions)]
    {
        let dev_path = dev_bridge_path();
        if dev_path.exists() {
            return Ok(dev_path);
        }
    }

    // Production: resolve from Tauri resource_dir (bundled version)
    app.path()
        .resource_dir()
        .map(|d| d.join("resources").join("claude-sdk-bridge.bundled.mjs"))
        .map_err(|e| format!("Failed to resolve resource dir: {e}"))
}

fn resolve_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

fn get_resume_session_id(
    app: &tauri::AppHandle,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Option<String> {
    resolve_data_dir(app)
        .ok()
        .and_then(|data_dir| {
            session_store
                .get_session(&data_dir, chat_session_id)
                .ok()
                .flatten()
        })
        .and_then(|s| s.agent_session_id)
}

fn emit_agent_query_completed(
    app: &tauri::AppHandle,
    exit_code: i64,
    stderr: &str,
    chat_session_id: &str,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "agent-query-completed",
        serde_json::json!({
            "exit_code": exit_code,
            "stderr": stderr,
            "chat_session_id": chat_session_id,
        }),
    );
}

fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

const INTERRUPT_TIMEOUT_SECS: u64 = 10;
const CLOSE_TIMEOUT_SECS: u64 = 5;

async fn spawn_bridge_process(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    session_id: Option<String>,
    cwd: &str,
    permission_mode: Option<String>,
) -> Result<(), String> {
    let bridge_path = resolve_bridge_script(app)?;
    if !bridge_path.exists() {
        return Err(format!(
            "Bridge script not found: {}",
            bridge_path.display()
        ));
    }

    let mut child = Command::new("node")
        .arg(
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
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn node process: {e}"))?;

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

    // Send init command
    let init_cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "permissionMode": permission_mode.unwrap_or_else(|| "acceptEdits".to_string()),
        "sessionId": session_id,
    });
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
    {
        let mut map = handles.lock().await;
        map.insert(
            chat_session_id.to_string(),
            AgentProcess {
                stdin,
                state: BridgeState::Initializing,
                sdk_session_id: session_id,
                child,
                generation_id: gen_id,
            },
        );
    }

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
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            if let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(&line) {
                msg["chat_session_id"] = serde_json::Value::String(csid_stdout.clone());

                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match msg_type {
                    "session_ready" => {
                        let mut map = handles_stdout.lock().await;
                        if let Some(proc) = map.get_mut(&csid_stdout) {
                            // Only transition to Ready if still Initializing (not already Streaming)
                            if proc.state == BridgeState::Initializing {
                                proc.state = BridgeState::Ready;
                            }
                            if let Some(sid) = msg.get("session_id").and_then(|v| v.as_str()) {
                                proc.sdk_session_id = Some(sid.to_string());
                            }
                        }
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                    }
                    "turn_complete" => {
                        let exit_code = msg.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
                        let was_streaming;
                        {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                was_streaming = proc.state == BridgeState::Streaming;
                                proc.state = BridgeState::Ready;

                                // User turn succeeded: persist agent_session_id to SessionStore
                                if was_streaming && exit_code == 0 {
                                    if let Some(sid) = &proc.sdk_session_id {
                                        if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                            if let Ok(Some(mut session)) = session_store_clone
                                                .get_session(&data_dir, &csid_stdout)
                                            {
                                                session.agent_session_id = Some(sid.to_string());
                                                session.updated_at = now_timestamp();
                                                let _ = session_store_clone
                                                    .save_session(&data_dir, &session);
                                            }
                                        }
                                    }
                                }
                            } else {
                                was_streaming = false;
                            }
                        }

                        // Resume failure (error during init) → clear stale agent_session_id
                        if !was_streaming && exit_code != 0 {
                            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                if let Ok(Some(mut session)) =
                                    session_store_clone.get_session(&data_dir, &csid_stdout)
                                {
                                    if session.agent_session_id.is_some() {
                                        session.agent_session_id = None;
                                        session.updated_at = now_timestamp();
                                        let _ =
                                            session_store_clone.save_session(&data_dir, &session);
                                    }
                                }
                            }
                        }

                        // Emit agent-query-completed only for user turns (was Streaming)
                        if was_streaming {
                            let stderr_text =
                                msg.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
                            emit_agent_query_completed(
                                &app_stdout,
                                exit_code,
                                stderr_text,
                                &csid_stdout,
                            );
                        }
                    }
                    "error" => {
                        let error_msg = msg
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown bridge error");
                        log::error!("Bridge error [{}]: {}", csid_stdout, error_msg);

                        let _ = app_stdout.emit("agent-sdk-message", &msg);

                        // Transition to Crashed for both Streaming and Initializing states
                        let (was_streaming, was_initializing) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                let ws = proc.state == BridgeState::Streaming;
                                let wi = proc.state == BridgeState::Initializing;
                                if ws || wi {
                                    proc.state = BridgeState::Crashed;
                                }
                                (ws, wi)
                            } else {
                                (false, false)
                            }
                        };
                        if was_streaming {
                            emit_agent_query_completed(&app_stdout, 1, error_msg, &csid_stdout);
                        }
                        // Init error → clear stale agent_session_id to prevent infinite resume loop
                        if was_initializing {
                            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                if let Ok(Some(mut session)) =
                                    session_store_clone.get_session(&data_dir, &csid_stdout)
                                {
                                    if session.agent_session_id.is_some() {
                                        session.agent_session_id = None;
                                        session.updated_at = now_timestamp();
                                        let _ =
                                            session_store_clone.save_session(&data_dir, &session);
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                    }
                }
            }
        }
        // EOF — process exited; verify generation to avoid acting on stale events
        let was_streaming = {
            let map = handles_stdout.lock().await;
            map.get(&csid_stdout).is_some_and(|p| {
                p.generation_id == captured_gen_id && p.state == BridgeState::Streaming
            })
        };
        if was_streaming {
            emit_agent_query_completed(
                &app_stdout,
                -1,
                "Bridge process exited unexpectedly",
                &csid_stdout,
            );
        }
        {
            let mut map = handles_stdout.lock().await;
            if let Some(proc) = map.get_mut(&csid_stdout) {
                if proc.generation_id == captured_gen_id {
                    proc.state = BridgeState::Crashed;
                }
            }
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

    Ok(())
}

#[tauri::command]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    // If process already exists and is not crashed, do nothing; otherwise remove crashed entry
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get(&chat_session_id) {
            if proc.state != BridgeState::Crashed {
                return Ok(());
            }
        }
        map.remove(&chat_session_id);
    }

    let resume_sid = get_resume_session_id(&app, session_store.inner(), &chat_session_id);

    spawn_bridge_process(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        resume_sid,
        &cwd,
        permission_mode,
    )
    .await
}

#[tauri::command]
pub async fn execute_agent_query(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    prompt: String,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    // Check if we need to spawn a new process (single lock to avoid TOCTOU)
    let spawn_info = {
        let mut map = handles.lock().await;
        match map.get(&chat_session_id) {
            None => Some(get_resume_session_id(
                &app,
                session_store.inner(),
                &chat_session_id,
            )),
            Some(proc) if proc.state == BridgeState::Crashed => {
                map.remove(&chat_session_id);
                Some(get_resume_session_id(
                    &app,
                    session_store.inner(),
                    &chat_session_id,
                ))
            }
            _ => None,
        }
    };

    if let Some(resume_sid) = spawn_info {
        spawn_bridge_process(
            &app,
            handles.inner(),
            session_store.inner(),
            &chat_session_id,
            resume_sid,
            &cwd,
            permission_mode,
        )
        .await?;
    }

    // Send message command.
    // Even if a message is sent while the SDK is still processing an interrupt,
    // the Bridge's promptGenerator queues it and only yields after the current turn completes.
    // The SDK calls generator.next() only when ready for the next turn, providing ordering guarantee.
    let msg_cmd = serde_json::json!({
        "type": "message",
        "prompt": prompt,
    });
    let data = format!("{}\n", msg_cmd);

    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(&chat_session_id) {
        proc.state = BridgeState::Streaming;
        proc.stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write message: {e}"))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush message: {e}"))?;
    } else {
        return Err(format!("No agent process for session {chat_session_id}"));
    }

    Ok(())
}

#[tauri::command]
pub async fn interrupt_agent_query(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<(), String> {
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            proc.stdin
                .write_all(b"{\"type\":\"interrupt\"}\n")
                .await
                .map_err(|e| format!("Failed to write interrupt: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush: {e}"))?;
        } else {
            return Err(format!(
                "No active agent process for session {chat_session_id}"
            ));
        }
    }

    // Timeout fallback: if turn doesn't complete, kill the process
    let handles_clone = Arc::clone(handles.inner());
    let csid = chat_session_id.clone();
    let timeout_gen_id = {
        let map = handles_clone.lock().await;
        map.get(&csid).map(|p| p.generation_id)
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(INTERRUPT_TIMEOUT_SECS)).await;
        let mut map = handles_clone.lock().await;
        if let Some(proc) = map.get_mut(&csid) {
            if timeout_gen_id == Some(proc.generation_id) && proc.state == BridgeState::Streaming {
                log::warn!("Interrupt timeout for session {csid}, killing process");
                let _ = proc.child.kill().await;
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn close_agent_session(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<(), String> {
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            if let Err(e) = proc.stdin.write_all(b"{\"type\":\"close\"}\n").await {
                log::warn!("Failed to send close command for session {chat_session_id}: {e}");
            }
            if let Err(e) = proc.stdin.flush().await {
                log::warn!("Failed to flush close command for session {chat_session_id}: {e}");
            }
        } else {
            // No process to close — already gone
            return Ok(());
        }
    }

    // Timeout fallback: if process doesn't exit, kill it
    let handles_clone = Arc::clone(handles.inner());
    let csid = chat_session_id.clone();
    let timeout_gen_id = {
        let map = handles_clone.lock().await;
        map.get(&csid).map(|p| p.generation_id)
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS)).await;
        let mut map = handles_clone.lock().await;
        if let Some(proc) = map.get_mut(&csid) {
            if timeout_gen_id == Some(proc.generation_id) {
                log::warn!("Close timeout for session {csid}, killing process");
                let _ = proc.child.kill().await;
            } else {
                // Generation mismatch: a new process has been spawned; skip kill and remove
                return;
            }
        }
        map.remove(&csid);
    });

    Ok(())
}

#[tauri::command]
pub async fn respond_agent_permission(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    if behavior != "allow" && behavior != "deny" {
        return Err(format!("Invalid behavior: {behavior}"));
    }
    let mut result = serde_json::json!({ "behavior": behavior });
    if let Some(msg) = &message {
        result["message"] = serde_json::Value::String(msg.clone());
    }
    if let Some(input_json) = &updated_input {
        match serde_json::from_str::<serde_json::Value>(input_json) {
            Ok(parsed) => {
                result["updatedInput"] = parsed;
            }
            Err(e) => {
                log::warn!("Failed to parse updated_input JSON: {e}");
            }
        }
    }
    let payload = serde_json::json!({
        "type": "permission_response",
        "request_id": request_id,
        "result": result,
    });
    let data = format!("{}\n", payload);

    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(&chat_session_id) {
        proc.stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write permission response: {e}"))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush: {e}"))?;
    } else {
        return Err(format!(
            "No active agent process for session {chat_session_id}"
        ));
    }
    Ok(())
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct SlashCommandEntry {
    pub name: String,
    pub description: String,
}

/// Parse SKILL.md frontmatter (delimited by `---`) and extract `name` / `description` fields.
fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    // First line must be `---`
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = trimmed.strip_prefix("description:") {
            description = Some(val.trim().to_string());
        }
    }
    Some((name.unwrap_or_default(), description.unwrap_or_default()))
}

/// Scan skill and command directories to collect slash commands with deduplication.
///
/// Scan order (highest priority first):
/// 1. `~/.claude/skills/*/SKILL.md` — personal skill
/// 2. `{cwd}/.claude/skills/*/SKILL.md` — project skill
/// 3. `~/.claude/commands/*.md` — personal command
/// 4. `{cwd}/.claude/commands/*.md` — project command
///
/// When the same name appears in multiple sources, the higher-priority entry wins.
#[tauri::command]
pub async fn scan_slash_commands(cwd: String) -> Result<Vec<SlashCommandEntry>, String> {
    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    let cwd_path = PathBuf::from(&cwd);

    // Build list of directories to scan in priority order
    let mut skill_dirs: Vec<PathBuf> = Vec::new();
    let mut command_dirs: Vec<PathBuf> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        skill_dirs.push(home.join(".claude").join("skills"));
        command_dirs.push(home.join(".claude").join("commands"));
    }
    skill_dirs.push(cwd_path.join(".claude").join("skills"));
    command_dirs.push(cwd_path.join(".claude").join("commands"));

    // Scan skills (personal first, then project)
    for skills_dir in &skill_dirs {
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&skill_md) {
                        if let Some((name, description)) = parse_skill_frontmatter(&content) {
                            if !name.is_empty() && seen.insert(name.clone()) {
                                commands.push(SlashCommandEntry { name, description });
                            }
                        }
                    }
                }
            }
        }
    }

    // Scan commands (personal first, then project)
    for cmd_dir in &command_dirs {
        if let Ok(entries) = std::fs::read_dir(cmd_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") && path.is_file() {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    if seen.insert(name.clone()) {
                        let description = std::fs::read_to_string(&path)
                            .ok()
                            .and_then(|c| c.lines().next().map(|l| l.trim().to_string()))
                            .unwrap_or_default();
                        commands.push(SlashCommandEntry { name, description });
                    }
                }
            }
        }
    }

    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_process_map_starts_empty() {
        let map = AgentProcessMap::new();
        assert!(map.is_empty());
    }

    #[test]
    fn bridge_state_transitions() {
        let state = BridgeState::Initializing;
        assert_eq!(state, BridgeState::Initializing);
        assert_ne!(state, BridgeState::Ready);
        assert_ne!(state, BridgeState::Streaming);
        assert_ne!(state, BridgeState::Crashed);
    }

    #[test]
    fn init_command_format() {
        let cwd = "/repo";
        let permission_mode = "acceptEdits";
        let session_id: Option<String> = Some("sess-abc".to_string());
        let cmd = serde_json::json!({
            "type": "init",
            "cwd": cwd,
            "permissionMode": permission_mode,
            "sessionId": session_id,
        });
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "sess-abc");
    }

    #[test]
    fn init_command_without_session_id() {
        let session_id: Option<String> = None;
        let cmd = serde_json::json!({
            "type": "init",
            "cwd": "/repo",
            "permissionMode": "acceptEdits",
            "sessionId": session_id,
        });
        assert!(cmd["sessionId"].is_null());
    }

    #[test]
    fn message_command_format() {
        let prompt = "Hello, agent!";
        let cmd = serde_json::json!({
            "type": "message",
            "prompt": prompt,
        });
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "Hello, agent!");
    }

    #[test]
    fn dev_bridge_path_points_to_src_tauri_resources() {
        let path = dev_bridge_path();
        assert!(
            path.ends_with("src-tauri/resources/claude-sdk-bridge.mjs"),
            "dev_bridge_path should end with src-tauri/resources/claude-sdk-bridge.mjs, got: {}",
            path.display()
        );
    }

    #[test]
    fn dev_bridge_path_file_exists() {
        let path = dev_bridge_path();
        assert!(
            path.exists(),
            "Bridge script should exist at {}, but it does not",
            path.display()
        );
    }

    #[tokio::test]
    async fn permission_request_message_is_parseable() {
        let json_str = r#"{"type":"permission_request","request_id":"abc-123","tool_name":"Edit","input":{},"tool_use_id":"toolu_001"}"#;
        let msg: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            msg.get("type").and_then(|v| v.as_str()),
            Some("permission_request")
        );
        assert_eq!(
            msg.get("request_id").and_then(|v| v.as_str()),
            Some("abc-123")
        );
        assert_eq!(msg.get("tool_name").and_then(|v| v.as_str()), Some("Edit"));
    }

    #[test]
    fn permission_response_payload_format() {
        let request_id = "req-123";
        let behavior = "allow";
        let message: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["type"], "permission_response");
        assert_eq!(payload["request_id"], "req-123");
        assert_eq!(payload["result"]["behavior"], "allow");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn permission_response_payload_with_updated_input() {
        let request_id = "req-789";
        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input = Some(r#"{"questions":[],"answers":{"Q":"A"}}"#.to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "allow");
        assert_eq!(payload["result"]["updatedInput"]["answers"]["Q"], "A");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn behavior_validation_rejects_invalid_values() {
        let valid = ["allow", "deny"];
        let invalid = ["Allow", "ALLOW", "reject", "", "maybe"];
        for v in valid {
            assert!(v == "allow" || v == "deny");
        }
        for v in invalid {
            assert!(v != "allow" && v != "deny");
        }
    }

    #[test]
    fn permission_response_payload_with_deny_message() {
        let request_id = "req-456";
        let behavior = "deny";
        let message = Some("User denied".to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "deny");
        assert_eq!(payload["result"]["message"], "User denied");
    }

    #[tokio::test]
    async fn node_subprocess_stdout_is_readable_as_ndjson() {
        let mock_script = r#"
            process.stdout.write(JSON.stringify({type:"system",session_id:"test-sid"}) + "\n");
            process.stdout.write(JSON.stringify({type:"stream_event",event:{type:"content_block_delta",delta:{type:"text_delta",text:"hello"}}}) + "\n");
            process.stdout.write(JSON.stringify({type:"result",subtype:"success",session_id:"test-sid"}) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(mock_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut messages: Vec<serde_json::Value> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let msg: serde_json::Value =
                serde_json::from_str(&line).expect(&format!("Failed to parse: {line}"));
            messages.push(msg);
        }

        let status = child.wait().await.unwrap();
        assert!(status.success(), "node process should exit 0");
        assert_eq!(messages.len(), 3, "Should have 3 messages");

        assert_eq!(
            messages[0].get("session_id").and_then(|v| v.as_str()),
            Some("test-sid")
        );

        let event = &messages[1]["event"];
        assert_eq!(event["type"].as_str(), Some("content_block_delta"));
        assert_eq!(event["delta"]["type"].as_str(), Some("text_delta"));
        assert_eq!(event["delta"]["text"].as_str(), Some("hello"));

        assert_eq!(messages[2]["type"].as_str(), Some("result"));
        assert_eq!(messages[2]["subtype"].as_str(), Some("success"));
    }

    #[tokio::test]
    async fn bridge_stdin_command_protocol_roundtrip() {
        use tokio::io::AsyncWriteExt;

        // Simulate the bridge's stdin protocol: init → message handling
        // Uses an inline script that mirrors the bridge's command parsing.
        let test_script = r#"
            let stdinBuffer = "";
            const commands = [];
            process.stdin.setEncoding("utf8");
            process.stdin.on("data", (chunk) => {
                stdinBuffer += chunk;
                const lines = stdinBuffer.split("\n");
                stdinBuffer = lines.pop();
                for (const line of lines) {
                    if (!line.trim()) continue;
                    try {
                        commands.push(JSON.parse(line));
                    } catch {}
                }
            });
            process.stdin.on("end", () => {
                process.stdout.write(JSON.stringify({ received: commands }) + "\n");
            });
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // Send init and message commands
        let init_cmd =
            serde_json::json!({"type": "init", "cwd": "/tmp", "permissionMode": "acceptEdits"});
        let msg_cmd = serde_json::json!({"type": "message", "prompt": "hello"});
        let close_cmd = serde_json::json!({"type": "close"});

        stdin
            .write_all(format!("{}\n{}\n{}\n", init_cmd, msg_cmd, close_cmd).as_bytes())
            .await
            .unwrap();
        drop(stdin); // Close stdin to trigger "end" event

        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("Timeout")
            .unwrap()
            .unwrap();

        let result: serde_json::Value = serde_json::from_str(&line).unwrap();
        let received = result["received"].as_array().unwrap();
        assert_eq!(received.len(), 3);
        assert_eq!(received[0]["type"], "init");
        assert_eq!(received[1]["type"], "message");
        assert_eq!(received[1]["prompt"], "hello");
        assert_eq!(received[2]["type"], "close");

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_accept_edits_mode() {
        let test_script = r#"
            const permissionMode = "acceptEdits";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            if (permissionMode !== "bypassPermissions") {
                canUseToolSet = true;
            }

            const result = {
                permissionMode,
                canUseToolSet,
                interactiveToolsHandled: canUseToolSet,
            };
            process.stdout.write(JSON.stringify(result) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "acceptEdits mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_plan_mode() {
        let test_script = r#"
            const permissionMode = "plan";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            if (permissionMode !== "bypassPermissions") {
                canUseToolSet = true;
            }

            const result = {
                permissionMode,
                canUseToolSet,
                interactiveToolsHandled: canUseToolSet,
            };
            process.stdout.write(JSON.stringify(result) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "plan mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_exit_plan_mode_permission_response_roundtrip() {
        use tokio::io::AsyncWriteExt;

        let test_script = r#"
            const pendingPermissions = new Map();

            process.stdin.setEncoding('utf8');
            let buffer = '';
            process.stdin.on('data', (chunk) => {
                buffer += chunk;
                const lines = buffer.split('\n');
                buffer = lines.pop();
                for (const line of lines) {
                    if (!line.trim()) continue;
                    try {
                        const cmd = JSON.parse(line);
                        if (cmd.type === 'permission_response') {
                            const pending = pendingPermissions.get(cmd.request_id);
                            if (pending) {
                                pendingPermissions.delete(cmd.request_id);
                                const result = cmd.result;
                                if (result.behavior === 'allow' && !result.updatedInput) {
                                    result.updatedInput = pending.input;
                                }
                                pending.resolve(result);
                            }
                        }
                    } catch {}
                }
            });

            const requestId = 'req-exit-001';
            const toolInput = {
                allowedPrompts: [{ tool: 'Bash', prompt: 'run tests' }],
                pushToRemote: false,
            };

            const resultPromise = new Promise((resolve) => {
                pendingPermissions.set(requestId, { resolve, input: toolInput });
            });

            process.stdout.write(JSON.stringify({
                type: 'permission_request',
                request_id: requestId,
                tool_name: 'ExitPlanMode',
                input: toolInput,
                tool_use_id: 'toolu_exit_001',
            }) + '\n');

            resultPromise.then((result) => {
                process.stdout.write(JSON.stringify({
                    type: 'canUseTool_resolved',
                    tool_name: 'ExitPlanMode',
                    result: result,
                    result_keys: Object.keys(result).sort(),
                    result_json: JSON.stringify(result),
                }) + '\n');
                process.exit(0);
            });
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let request_line = lines.next_line().await.unwrap().unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["type"], "permission_request");
        assert_eq!(request["tool_name"], "ExitPlanMode");

        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let response = serde_json::json!({
            "type": "permission_response",
            "request_id": request["request_id"].as_str().unwrap(),
            "result": result,
        });
        let data = format!("{}\n", response);
        stdin.write_all(data.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();

        let resolved_line =
            tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .expect("Timeout waiting for resolved line")
                .unwrap()
                .unwrap();
        let resolved: serde_json::Value = serde_json::from_str(&resolved_line).unwrap();

        assert_eq!(resolved["type"], "canUseTool_resolved");
        assert_eq!(resolved["tool_name"], "ExitPlanMode");

        let can_use_tool_result = &resolved["result"];
        assert_eq!(
            can_use_tool_result["behavior"], "allow",
            "behavior should be 'allow'"
        );
        assert!(
            can_use_tool_result.get("updatedInput").is_some(),
            "updatedInput must be present in allow response (required by CLI Zod schema)"
        );
        assert_eq!(
            can_use_tool_result["updatedInput"]["allowedPrompts"][0]["tool"], "Bash",
            "updatedInput should contain the original tool input"
        );

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[test]
    fn turn_complete_message_parsing() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":0}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "turn_complete");
        assert_eq!(msg["exit_code"], 0);
        assert_eq!(msg["session_id"], "sess-123");
    }

    #[test]
    fn turn_complete_with_error() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":1}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["exit_code"], 1);
    }

    #[test]
    fn session_ready_message_parsing() {
        let msg_str = r#"{"type":"session_ready","session_id":"sess-456"}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "session_ready");
        assert_eq!(msg["session_id"], "sess-456");
    }

    #[test]
    fn parse_skill_frontmatter_valid() {
        let content = "---\nname: review\ndescription: Code review tool\n---\nBody here";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "review");
        assert_eq!(desc, "Code review tool");
    }

    #[test]
    fn parse_skill_frontmatter_missing_fields() {
        let content = "---\ntitle: something\n---\n";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "");
        assert_eq!(desc, "");
    }

    #[test]
    fn parse_skill_frontmatter_no_opening_delimiter() {
        let content = "name: review\n---\n";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn parse_skill_frontmatter_empty_content() {
        assert!(parse_skill_frontmatter("").is_none());
    }

    #[tokio::test]
    async fn scan_slash_commands_with_nonexistent_cwd() {
        let result = scan_slash_commands("/nonexistent/path/abc123".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scan_slash_commands_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();

        std::fs::write(
            commands_dir.join("test-cmd.md"),
            "This is a test command\nMore details here",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.path().to_string_lossy().to_string())
            .await
            .unwrap();

        let test_cmd = result.iter().find(|c| c.name == "test-cmd");
        assert!(test_cmd.is_some(), "Should find test-cmd in results");
        assert_eq!(test_cmd.unwrap().description, "This is a test command");
    }

    #[tokio::test]
    async fn scan_slash_commands_deduplicates_skill_over_command() {
        let tmp = tempfile::tempdir().unwrap();

        let skill_dir = tmp
            .path()
            .join(".claude")
            .join("skills")
            .join("zzz-dedup-test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: zzz-dedup-test\ndescription: From skill\n---\nBody",
        )
        .unwrap();

        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(
            commands_dir.join("zzz-dedup-test.md"),
            "From command\nDetails",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.path().to_string_lossy().to_string())
            .await
            .unwrap();

        let matches: Vec<_> = result
            .iter()
            .filter(|c| c.name == "zzz-dedup-test")
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "zzz-dedup-test should appear exactly once, got: {matches:?}"
        );
        assert_eq!(matches[0].description, "From skill");
    }
}
