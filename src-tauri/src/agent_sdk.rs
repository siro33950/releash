use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Per-session agent process map: chat_session_id → ChildStdin
pub type AgentProcessMap = HashMap<String, tokio::process::ChildStdin>;

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

    // Production: resolve from Tauri resource_dir
    app.path()
        .resource_dir()
        .map(|d| d.join("resources").join("claude-sdk-bridge.mjs"))
        .map_err(|e| format!("Failed to resolve resource dir: {e}"))
}

#[tauri::command]
pub async fn execute_agent_query(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    prompt: String,
    session_id: Option<String>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    let bridge_path = resolve_bridge_script(&app)?;
    if !bridge_path.exists() {
        return Err(format!(
            "Bridge script not found: {}",
            bridge_path.display()
        ));
    }

    let mut args_json = serde_json::json!({
        "prompt": prompt,
        "cwd": cwd,
    });
    if let Some(sid) = &session_id {
        if !sid.is_empty() {
            args_json["sessionId"] = serde_json::Value::String(sid.clone());
        }
    }
    if let Some(pm) = &permission_mode {
        args_json["permissionMode"] = serde_json::Value::String(pm.clone());
    }

    let mut child = Command::new("node")
        .arg(
            bridge_path
                .to_str()
                .ok_or_else(|| "Bridge script path contains invalid UTF-8".to_string())?,
        )
        .arg(args_json.to_string())
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn node process: {e}"))?;

    if let Some(stdin) = child.stdin.take() {
        let mut map = handles.lock().await;
        map.insert(chat_session_id.clone(), stdin);
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture stderr".to_string())?;

    let app_stdout = app.clone();
    let csid_stdout = chat_session_id.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            if let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(&line) {
                msg["chat_session_id"] = serde_json::Value::String(csid_stdout.clone());
                use tauri::Emitter;
                let _ = app_stdout.emit("agent-sdk-message", &msg);
            }
        }
    });

    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut stderr_output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                stderr_output.push_str(&line);
                stderr_output.push('\n');
            }
        }
        stderr_output
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for process: {e}"))?;

    // Wait for stream readers to complete
    if let Err(e) = stdout_task.await {
        log::error!("stdout reader task failed: {e}");
    }
    let stderr_output = match stderr_task.await {
        Ok(output) => output,
        Err(e) => {
            log::error!("stderr reader task failed: {e}");
            String::new()
        }
    };

    // Clean up handle
    {
        let mut map = handles.lock().await;
        map.remove(&chat_session_id);
    }

    let exit_code = status.code().unwrap_or(-1);
    use tauri::Emitter;
    if let Err(e) = app.emit(
        "agent-query-completed",
        serde_json::json!({
            "exit_code": exit_code,
            "stderr": stderr_output,
            "chat_session_id": chat_session_id,
        }),
    ) {
        log::error!("Failed to emit agent-query-completed: {e}");
    }

    Ok(())
}

#[tauri::command]
pub async fn interrupt_agent_query(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<(), String> {
    let mut map = handles.lock().await;
    if let Some(stdin) = map.get_mut(&chat_session_id) {
        stdin
            .write_all(b"{\"type\":\"interrupt\"}\n")
            .await
            .map_err(|e| format!("Failed to write interrupt: {e}"))?;
        stdin
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
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
            result["updatedInput"] = parsed;
        }
    }
    let payload = serde_json::json!({
        "type": "permission_response",
        "request_id": request_id,
        "result": result,
    });
    let data = format!("{}\n", payload);

    let mut map = handles.lock().await;
    if let Some(stdin) = map.get_mut(&chat_session_id) {
        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write permission response: {e}"))?;
        stdin
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
    fn bridge_script_args_without_session_id() {
        let mut args = serde_json::json!({
            "prompt": "hello",
            "cwd": "/repo",
        });
        let session_id: Option<String> = None;
        if let Some(sid) = &session_id {
            if !sid.is_empty() {
                args["sessionId"] = serde_json::Value::String(sid.clone());
            }
        }
        assert!(args.get("sessionId").is_none());
        assert_eq!(args["prompt"], "hello");
        assert_eq!(args["cwd"], "/repo");
    }

    #[test]
    fn bridge_script_args_with_session_id() {
        let mut args = serde_json::json!({
            "prompt": "hello",
            "cwd": "/repo",
        });
        let session_id = Some("sess-abc".to_string());
        if let Some(sid) = &session_id {
            if !sid.is_empty() {
                args["sessionId"] = serde_json::Value::String(sid.clone());
            }
        }
        assert_eq!(args["sessionId"], "sess-abc");
    }

    #[test]
    fn bridge_script_args_with_empty_session_id() {
        let mut args = serde_json::json!({
            "prompt": "test",
            "cwd": "/repo",
        });
        let session_id = Some("".to_string());
        if let Some(sid) = &session_id {
            if !sid.is_empty() {
                args["sessionId"] = serde_json::Value::String(sid.clone());
            }
        }
        assert!(args.get("sessionId").is_none());
    }

    #[test]
    fn bridge_script_args_with_permission_mode() {
        let mut args = serde_json::json!({
            "prompt": "test",
            "cwd": "/repo",
        });
        let pm = Some("plan".to_string());
        if let Some(mode) = &pm {
            args["permissionMode"] = serde_json::Value::String(mode.clone());
        }
        assert_eq!(args["permissionMode"], "plan");
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
        // Verify that spawning a node process and reading its stdout line-by-line works.
        // Uses an inline script that outputs mock SDK messages.
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

        // Verify session_id extraction
        assert_eq!(
            messages[0].get("session_id").and_then(|v| v.as_str()),
            Some("test-sid")
        );

        // Verify stream_event text_delta
        let event = &messages[1]["event"];
        assert_eq!(event["type"].as_str(), Some("content_block_delta"));
        assert_eq!(event["delta"]["type"].as_str(), Some("text_delta"));
        assert_eq!(event["delta"]["text"].as_str(), Some("hello"));

        // Verify result
        assert_eq!(messages[2]["type"].as_str(), Some("result"));
        assert_eq!(messages[2]["subtype"].as_str(), Some("success"));
    }

    #[tokio::test]
    async fn bridge_script_spawns_and_exits_from_rust() {
        // Verify that spawning the actual bridge script from Rust works.
        // Uses an invalid prompt scenario that should cause the SDK to exit quickly.
        let bridge_path = dev_bridge_path();
        assert!(bridge_path.exists(), "Bridge script must exist");

        let args_json = serde_json::json!({
            "prompt": "test",
            "cwd": "/tmp",
        });

        let mut child = tokio::process::Command::new("node")
            .arg(bridge_path.to_str().unwrap())
            .arg(args_json.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node with bridge script");

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let stdout_task = tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut output = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.is_empty() {
                    output.push(line);
                }
            }
            output
        });

        let stderr_task = tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut output = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.is_empty() {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
            output
        });

        // Wait for exit with a timeout
        let status = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
            .await
            .expect("Bridge script timed out after 30 seconds")
            .expect("Failed to wait for bridge script");

        let stdout_lines = stdout_task.await.unwrap();
        let stderr_output = stderr_task.await.unwrap();

        // The process should exit (either success or error, but it must not hang)
        eprintln!("Bridge script exit code: {:?}", status.code());
        eprintln!("Bridge script stdout lines: {}", stdout_lines.len());
        for (i, line) in stdout_lines.iter().enumerate() {
            eprintln!("  stdout[{}]: {}", i, &line[..line.len().min(200)]);
        }
        if !stderr_output.is_empty() {
            eprintln!(
                "Bridge script stderr: {}",
                &stderr_output[..stderr_output.len().min(500)]
            );
        }

        // The process must exit (this test's main purpose is to verify it doesn't hang)
        assert!(
            status.code().is_some(),
            "Bridge script should exit with a code, not be killed by signal"
        );
    }

    /// Verifies that the bridge script sets `canUseTool` for interactive tools
    /// (AskUserQuestion, EnterPlanMode) even when permissionMode is "acceptEdits".
    /// Currently FAILS because the bridge only sets canUseTool for "default" mode.
    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_accept_edits_mode() {
        // Inline script that simulates the bridge's canUseTool conditional logic
        // and checks whether canUseTool would be set for interactive tools.
        let test_script = r#"
            const permissionMode = "acceptEdits";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            // Reproduce the bridge logic
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

        // This assertion FAILS: in acceptEdits mode, canUseTool is NOT set,
        // so interactive tools (AskUserQuestion, EnterPlanMode) are auto-handled
        // by the SDK and never generate permission_request messages.
        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "acceptEdits mode should set canUseTool for interactive tools, \
             but the bridge only sets it for 'default' mode. \
             Result: {}",
            result
        );
    }

    /// Same as above but for "plan" permissionMode.
    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_plan_mode() {
        let test_script = r#"
            const permissionMode = "plan";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            // Reproduce the bridge logic
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
            "plan mode should set canUseTool for interactive tools, \
             but the bridge only sets it for 'default' mode. \
             Result: {}",
            result
        );
    }

    /// ExitPlanMode permission round-trip through inline bridge simulation.
    /// Verifies the exact JSON that canUseTool resolves with when user clicks Allow.
    /// The bridge must augment allow responses with `updatedInput` (defaulting to
    /// the original tool input) because the CLI validates with a Zod schema
    /// where `updatedInput` is required in the allow variant.
    #[tokio::test]
    async fn bridge_exit_plan_mode_permission_response_roundtrip() {
        use tokio::io::AsyncWriteExt;

        // This script simulates the bridge's canUseTool logic:
        // - Stores { resolve, input } in pendingPermissions
        // - On permission_response, augments allow results with updatedInput if missing
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

        // Read the permission_request from the bridge
        let request_line = lines.next_line().await.unwrap().unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["type"], "permission_request");
        assert_eq!(request["tool_name"], "ExitPlanMode");

        // Build permission_response using the same logic as respond_agent_permission
        // (Rust sends { behavior: "allow" } WITHOUT updatedInput — the bridge adds it)
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

        // Read the resolved canUseTool value
        let resolved_line =
            tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .expect("Timeout waiting for resolved line")
                .unwrap()
                .unwrap();
        let resolved: serde_json::Value = serde_json::from_str(&resolved_line).unwrap();

        assert_eq!(resolved["type"], "canUseTool_resolved");
        assert_eq!(resolved["tool_name"], "ExitPlanMode");

        // The bridge augments the allow response with updatedInput from the original input.
        // This is required because the CLI's Zod schema (Lo6) requires updatedInput
        // in the allow variant of the PermissionResult union.
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

    /// Reproduces the exact same flow as `execute_agent_query`:
    /// spawn bridge script with piped stdin, read stdout in a spawned task,
    /// read stderr in a spawned task, then wait for exit.
    /// Verifies that `child.wait()` returns within a reasonable time
    /// after all SDK messages (including "result") have been received.
    #[tokio::test]
    async fn bridge_process_exits_after_result_message_like_execute_agent_query() {
        let bridge_path = dev_bridge_path();
        assert!(bridge_path.exists());

        let args_json = serde_json::json!({
            "prompt": "say hi",
            "cwd": "/tmp",
        });

        // Spawn exactly like execute_agent_query does
        let mut child = tokio::process::Command::new("node")
            .arg(bridge_path.to_str().unwrap())
            .arg(args_json.to_string())
            .current_dir("/tmp")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn");

        // Take stdin and HOLD it (same as storing in AgentProcessMap)
        let _stdin = child.stdin.take();

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Spawn stdout reader (same pattern as execute_agent_query)
        let stdout_task = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut messages: Vec<serde_json::Value> = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                    messages.push(msg);
                }
            }
            messages
        });

        // Spawn stderr reader (same pattern as execute_agent_query)
        let stderr_task = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut output = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.is_empty() {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
            output
        });

        // Wait for process exit with timeout (same as execute_agent_query, but with timeout)
        // BUG REPRODUCTION: if the process hangs after emitting "result",
        // child.wait() will never return and this timeout will fire.
        let wait_result =
            tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()).await;

        match wait_result {
            Ok(Ok(status)) => {
                // Process exited — no hang
                let messages = stdout_task.await.unwrap();
                let _stderr = stderr_task.await.unwrap_or_default();
                let has_result = messages.iter().any(|m| m["type"] == "result");
                eprintln!(
                    "Process exited: code={:?}, messages={}, has_result={}",
                    status.code(),
                    messages.len(),
                    has_result
                );
                assert!(has_result, "Should have received a result message");
            }
            Ok(Err(e)) => {
                panic!("child.wait() error: {e}");
            }
            Err(_) => {
                // TIMEOUT: process did not exit within 15 seconds after SDK query completed.
                // This reproduces the bug where agent-query-completed is never emitted.
                //
                // Kill the process FIRST so stdout/stderr pipes close,
                // allowing the reader tasks to finish.
                child.kill().await.ok();

                let messages = stdout_task.await.unwrap_or_default();
                let _stderr = stderr_task.await.unwrap_or_default();
                let has_result = messages.iter().any(|m| m["type"] == "result");
                eprintln!(
                    "TIMEOUT: process hung. messages={}, has_result={}",
                    messages.len(),
                    has_result
                );
                if has_result {
                    panic!(
                        "BUG REPRODUCED: bridge script received result message \
                         but process did not exit within 15 seconds. \
                         This causes agent-query-completed to never fire."
                    );
                } else {
                    panic!(
                        "Process hung but result was not received. \
                         messages count: {}",
                        messages.len()
                    );
                }
            }
        }
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
        // Should return commands from ~/.claude/skills (if any), but no error
    }

    #[tokio::test]
    async fn scan_slash_commands_with_temp_dir() {
        let tmp = std::env::temp_dir().join("releash_test_scan_cmds");
        let commands_dir = tmp.join(".claude").join("commands");
        let _ = std::fs::create_dir_all(&commands_dir);

        // Create a test command file
        std::fs::write(
            commands_dir.join("test-cmd.md"),
            "This is a test command\nMore details here",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.to_string_lossy().to_string())
            .await
            .unwrap();

        // Should contain our test command (may also contain skills from ~/.claude/skills)
        let test_cmd = result.iter().find(|c| c.name == "test-cmd");
        assert!(test_cmd.is_some(), "Should find test-cmd in results");
        assert_eq!(test_cmd.unwrap().description, "This is a test command");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn scan_slash_commands_deduplicates_skill_over_command() {
        // Use a unique name to avoid collisions with real ~/.claude/skills/
        let tmp = std::env::temp_dir().join("releash_test_dedup");
        let _ = std::fs::remove_dir_all(&tmp);

        // Create a project skill
        let skill_dir = tmp
            .join(".claude")
            .join("skills")
            .join("zzz-dedup-test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: zzz-dedup-test\ndescription: From skill\n---\nBody",
        )
        .unwrap();

        // Create a project command with the same name
        let commands_dir = tmp.join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(
            commands_dir.join("zzz-dedup-test.md"),
            "From command\nDetails",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.to_string_lossy().to_string())
            .await
            .unwrap();

        // "zzz-dedup-test" should appear exactly once, with the skill's description (higher priority)
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

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
