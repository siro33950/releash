use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_STREAM_BYTES: usize = 64 * 1024;
const NOTICE_PREVIEW_CHARS: usize = 1200;
const BACKGROUND_LOG_READ_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedStream {
    content: String,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellCommandExecution {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentShellCommandResult {
    pub title: String,
    pub detail: String,
    pub prompt: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentShellCompletionResult {
    pub completed: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPreparedShellCommand {
    pub command: String,
    pub display_command: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_output_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedBackgroundCommand {
    command: String,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentShellBackgroundOutput {
    pub output: String,
    pub truncated: bool,
    pub path: String,
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut result: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        result.push_str("...");
    }
    result
}

pub(crate) fn complete_agent_shell_command_inner(
    history: &[String],
    draft: &str,
) -> AgentShellCompletionResult {
    let trimmed = draft.trim_start();
    if !trimmed.starts_with('!') {
        return AgentShellCompletionResult { completed: None };
    }
    let query = trimmed[1..].trim_start().to_lowercase();
    let mut seen = std::collections::HashSet::new();
    for entry in history.iter().rev() {
        let command = entry.trim_start();
        if !command.starts_with('!') {
            continue;
        }
        let shell_command = command[1..].trim_start();
        if shell_command.is_empty() {
            continue;
        }
        if !query.is_empty() && !shell_command.to_lowercase().starts_with(&query) {
            continue;
        }
        if !seen.insert(shell_command.to_string()) {
            continue;
        }
        return AgentShellCompletionResult {
            completed: Some(format!("! {shell_command}")),
        };
    }
    AgentShellCompletionResult { completed: None }
}

#[tauri::command]
pub fn complete_agent_shell_command(
    history: Vec<String>,
    draft: String,
) -> AgentShellCompletionResult {
    complete_agent_shell_command_inner(&history, &draft)
}

fn strip_background_suffix(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if !trimmed.ends_with('&') || trimmed.ends_with("&&") {
        return None;
    }
    let stripped = trimmed.trim_end_matches('&').trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

#[cfg(unix)]
fn shell_output_dir() -> PathBuf {
    std::env::temp_dir().join("releash-agent-shell")
}

#[cfg(not(unix))]
fn shell_output_dir() -> PathBuf {
    std::env::temp_dir().join("releash-agent-shell")
}

fn create_background_output_path() -> Result<PathBuf, String> {
    let dir = shell_output_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create shell output directory: {e}"))?;
    let path = dir.join(format!("{}.log", uuid::Uuid::new_v4()));
    std::fs::File::create(&path).map_err(|e| format!("Failed to create shell output file: {e}"))?;
    Ok(path)
}

#[cfg(unix)]
fn background_shell_command(command: &str) -> Result<PreparedBackgroundCommand, String> {
    let output_path = create_background_output_path()?;
    let quoted_path = shell_quote(output_path.to_string_lossy().as_ref());
    Ok(PreparedBackgroundCommand {
        command: format!("({command}) > {quoted_path} 2>&1 &"),
        output_path: Some(output_path.to_string_lossy().to_string()),
    })
}

#[cfg(not(unix))]
fn background_shell_command(command: &str) -> Result<PreparedBackgroundCommand, String> {
    let output_path = create_background_output_path()?;
    let escaped_path = output_path.to_string_lossy().replace('"', "\\\"");
    Ok(PreparedBackgroundCommand {
        command: format!("{command} > \"{escaped_path}\" 2>&1"),
        output_path: Some(output_path.to_string_lossy().to_string()),
    })
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn prepare_agent_shell_command_inner(
    command: &str,
) -> Result<AgentPreparedShellCommand, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Shell command is empty".to_string());
    }
    if let Some(foreground_command) = strip_background_suffix(command) {
        let prepared = background_shell_command(&foreground_command)?;
        return Ok(AgentPreparedShellCommand {
            command: prepared.command,
            display_command: foreground_command.clone(),
            label: format!("agent-shell-bg:{}", truncate_chars(&foreground_command, 80)),
            timeout_secs: None,
            background: true,
            background_output_path: prepared.output_path,
        });
    }
    Ok(AgentPreparedShellCommand {
        command: command.to_string(),
        display_command: command.to_string(),
        label: format!("agent-shell:{}", truncate_chars(command, 80)),
        timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        background: false,
        background_output_path: None,
    })
}

pub(crate) fn prepare_agent_shell_input_inner(
    content: &str,
) -> Result<Option<AgentPreparedShellCommand>, String> {
    let trimmed = content.trim();
    if !trimmed.starts_with('!') {
        return Ok(None);
    }
    let command = trimmed[1..].trim();
    if command.is_empty() {
        return Err("Shell command is empty".to_string());
    }
    prepare_agent_shell_command_inner(command).map(Some)
}

pub(crate) fn parse_agent_runtime_shell_command(content: &str) -> Result<Option<String>, String> {
    let trimmed = content.trim();
    if !trimmed.starts_with('!') {
        return Ok(None);
    }
    let command = trimmed[1..].trim();
    if command.is_empty() {
        return Err("Shell command is empty".to_string());
    }
    Ok(Some(command.to_string()))
}

#[tauri::command]
pub fn prepare_agent_shell_command(command: String) -> Result<AgentPreparedShellCommand, String> {
    prepare_agent_shell_command_inner(&command)
}

#[tauri::command]
pub fn prepare_agent_shell_input(
    content: String,
) -> Result<Option<AgentPreparedShellCommand>, String> {
    prepare_agent_shell_input_inner(&content)
}

fn fenced(label: &str, content: &str) -> String {
    if content.trim().is_empty() {
        format!("{label}:\n<empty>")
    } else {
        format!("{label}:\n```text\n{content}\n```")
    }
}

fn shell_status_label(execution: &ShellCommandExecution) -> String {
    if execution.timed_out {
        "timeout".to_string()
    } else {
        match execution.exit_code {
            Some(0) => "completed".to_string(),
            Some(code) => format!("exit {code}"),
            None => "unknown".to_string(),
        }
    }
}

pub(crate) fn build_agent_shell_command_prompt(
    execution: &ShellCommandExecution,
) -> AgentShellCommandResult {
    let status = shell_status_label(execution);
    let truncation_note = match (execution.stdout_truncated, execution.stderr_truncated) {
        (true, true) => "\nNote: stdout and stderr were truncated.",
        (true, false) => "\nNote: stdout was truncated.",
        (false, true) => "\nNote: stderr was truncated.",
        (false, false) => "",
    };
    let prompt = format!(
        "The user ran this local shell command from the agent composer. Treat the command output as already-produced workspace context for this session. Do not rerun the command unless the user explicitly asks.\n\nCommand:\n```sh\n{}\n```\n\nStatus: {}{}\n\n{}\n\n{}",
        execution.command,
        status,
        truncation_note,
        fenced("Stdout", &execution.stdout),
        fenced("Stderr", &execution.stderr),
    );
    let combined_output = match (
        execution.stdout.trim().is_empty(),
        execution.stderr.trim().is_empty(),
    ) {
        (false, false) => format!("{}\n{}", execution.stdout, execution.stderr),
        (false, true) => execution.stdout.clone(),
        (true, false) => execution.stderr.clone(),
        (true, true) => "<no output>".to_string(),
    };
    AgentShellCommandResult {
        title: format!("Shell: {status}"),
        detail: truncate_chars(&combined_output, NOTICE_PREVIEW_CHARS),
        prompt,
        exit_code: execution.exit_code,
        timed_out: execution.timed_out,
    }
}

#[tauri::command]
pub fn build_agent_shell_command_context_prompt(
    command: String,
    output: String,
    exit_code: Option<i32>,
    timed_out: Option<bool>,
    truncated: Option<bool>,
) -> Result<AgentShellCommandResult, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Shell command is empty".to_string());
    }
    Ok(build_agent_shell_command_prompt(&ShellCommandExecution {
        command: command.to_string(),
        stdout: output,
        stderr: String::new(),
        stdout_truncated: truncated.unwrap_or(false),
        stderr_truncated: false,
        exit_code,
        timed_out: timed_out.unwrap_or(false),
    }))
}

fn validate_background_output_path(path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    if !candidate.is_absolute() {
        return Err("Background shell output path must be absolute".to_string());
    }
    let output_dir = shell_output_dir()
        .canonicalize()
        .map_err(|e| format!("Failed to resolve shell output directory: {e}"))?;
    let parent = candidate
        .parent()
        .ok_or_else(|| "Invalid background shell output path".to_string())?
        .canonicalize()
        .map_err(|e| format!("Failed to resolve shell output path: {e}"))?;
    if parent != output_dir {
        return Err(
            "Background shell output path is outside Releash shell output directory".to_string(),
        );
    }
    Ok(candidate)
}

#[tauri::command]
pub fn read_agent_shell_background_output(
    output_path: String,
) -> Result<AgentShellBackgroundOutput, String> {
    let path = validate_background_output_path(&output_path)?;
    if !path.exists() {
        return Ok(AgentShellBackgroundOutput {
            output: String::new(),
            truncated: false,
            path: output_path,
        });
    }
    let content =
        std::fs::read(&path).map_err(|e| format!("Failed to read background shell output: {e}"))?;
    let truncated = content.len() > BACKGROUND_LOG_READ_BYTES;
    let start = content.len().saturating_sub(BACKGROUND_LOG_READ_BYTES);
    let output = String::from_utf8_lossy(&content[start..]).to_string();
    Ok(AgentShellBackgroundOutput {
        output,
        truncated,
        path: output_path,
    })
}

async fn read_limited<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<CapturedStream, String> {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|e| format!("Failed to read shell output: {e}"))?;
        if read == 0 {
            break;
        }
        if collected.len() < limit {
            let remaining = limit - collected.len();
            let take = remaining.min(read);
            collected.extend_from_slice(&buffer[..take]);
        }
        if read > 0 && collected.len() >= limit {
            truncated = true;
        }
    }
    Ok(CapturedStream {
        content: String::from_utf8_lossy(&collected).to_string(),
        truncated,
    })
}

#[cfg(unix)]
fn shell_command(command: &str) -> Command {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    let mut cmd = Command::new(shell);
    cmd.arg("-lc").arg(command);
    cmd
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

pub(crate) async fn run_agent_shell_command_inner(
    worktree_path: &Path,
    command: &str,
    timeout_secs: u64,
) -> Result<ShellCommandExecution, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Shell command is empty".to_string());
    }
    if !worktree_path.is_dir() {
        return Err(format!(
            "Worktree path is not a directory: {}",
            worktree_path.display()
        ));
    }

    let mut child = shell_command(command)
        .current_dir(worktree_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn shell command: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture shell stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture shell stderr".to_string())?;
    let stdout_task = tokio::spawn(read_limited(stdout, MAX_STREAM_BYTES));
    let stderr_task = tokio::spawn(read_limited(stderr, MAX_STREAM_BYTES));

    let wait = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;
    let (exit_code, timed_out) = match wait {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("Failed to wait for shell command: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            (None, true)
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|e| format!("Failed to join stdout capture task: {e}"))??;
    let stderr = stderr_task
        .await
        .map_err(|e| format!("Failed to join stderr capture task: {e}"))??;

    Ok(ShellCommandExecution {
        command: command.to_string(),
        stdout: stdout.content,
        stderr: stderr.content,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        exit_code,
        timed_out,
    })
}

#[tauri::command]
pub async fn run_agent_shell_command(
    worktree_path: String,
    command: String,
    timeout_secs: Option<u64>,
) -> Result<AgentShellCommandResult, String> {
    let execution = run_agent_shell_command_inner(
        Path::new(&worktree_path),
        &command,
        timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
    )
    .await?;
    Ok(build_agent_shell_command_prompt(&execution))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_prompt_includes_command_output_and_status() {
        let result = build_agent_shell_command_prompt(&ShellCommandExecution {
            command: "printf hello".to_string(),
            stdout: "hello".to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            exit_code: Some(0),
            timed_out: false,
        });

        assert_eq!(result.title, "Shell: completed");
        assert!(result.prompt.contains("printf hello"));
        assert!(result.prompt.contains("Status: completed"));
        assert!(result.prompt.contains("hello"));
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn shell_prompt_reports_truncation() {
        let result = build_agent_shell_command_prompt(&ShellCommandExecution {
            command: "cmd".to_string(),
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            stdout_truncated: true,
            stderr_truncated: false,
            exit_code: Some(1),
            timed_out: false,
        });

        assert_eq!(result.title, "Shell: exit 1");
        assert!(result.prompt.contains("stdout was truncated"));
        assert!(result.detail.contains("out"));
        assert!(result.detail.contains("err"));
    }

    #[test]
    fn shell_context_prompt_reuses_captured_output_without_rerun() {
        let result = build_agent_shell_command_context_prompt(
            "printf hello".to_string(),
            "hello".to_string(),
            Some(0),
            Some(false),
            Some(false),
        )
        .unwrap();

        assert_eq!(result.title, "Shell: completed");
        assert!(result.prompt.contains("already-produced workspace context"));
        assert!(result.prompt.contains("hello"));
    }

    #[test]
    fn shell_completion_uses_latest_matching_bang_command() {
        let result = complete_agent_shell_command_inner(
            &[
                "! npm test".to_string(),
                "regular prompt".to_string(),
                "! pnpm test --filter agent".to_string(),
            ],
            "! pn",
        );

        assert_eq!(
            result.completed,
            Some("! pnpm test --filter agent".to_string())
        );
    }

    #[test]
    fn shell_completion_ignores_non_shell_drafts() {
        let result = complete_agent_shell_command_inner(&["! pnpm test".to_string()], "pn");

        assert_eq!(result.completed, None);
    }

    #[test]
    fn prepare_shell_command_keeps_foreground_commands() {
        let prepared = prepare_agent_shell_command_inner("printf hello").unwrap();

        assert_eq!(prepared.command, "printf hello");
        assert_eq!(prepared.display_command, "printf hello");
        assert_eq!(prepared.timeout_secs, Some(DEFAULT_TIMEOUT_SECS));
        assert!(!prepared.background);
        assert_eq!(prepared.background_output_path, None);
    }

    #[test]
    fn prepare_shell_input_returns_none_for_regular_prompt() {
        let prepared = prepare_agent_shell_input_inner("explain this change").unwrap();

        assert_eq!(prepared, None);
    }

    #[test]
    fn prepare_shell_input_extracts_bang_command() {
        let prepared = prepare_agent_shell_input_inner(" ! printf hello ").unwrap();

        assert_eq!(prepared.unwrap().command, "printf hello");
    }

    #[test]
    fn runtime_shell_command_preserves_shell_syntax_after_bang() {
        let command = parse_agent_runtime_shell_command(" ! pnpm test > out.log & ")
            .unwrap()
            .unwrap();

        assert_eq!(command, "pnpm test > out.log &");
    }

    #[test]
    fn runtime_shell_command_ignores_regular_prompt() {
        let command = parse_agent_runtime_shell_command("explain this").unwrap();

        assert_eq!(command, None);
    }

    #[test]
    fn prepare_shell_input_rejects_empty_bang_command() {
        let err = prepare_agent_shell_input_inner(" !  ").unwrap_err();

        assert_eq!(err, "Shell command is empty");
    }

    #[test]
    fn prepare_shell_command_marks_trailing_ampersand_as_owned_background() {
        let prepared = prepare_agent_shell_command_inner("pnpm test &").unwrap();

        assert!(prepared.command.contains("pnpm test"));
        assert_eq!(prepared.display_command, "pnpm test");
        assert!(prepared.background);
        assert_eq!(prepared.timeout_secs, None);
        assert!(prepared.background_output_path.is_some());
    }

    #[test]
    fn read_background_output_reads_releash_temp_log() {
        let output_dir = shell_output_dir();
        std::fs::create_dir_all(&output_dir).unwrap();
        let path = output_dir.join(format!("test-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&path, "background output\n").unwrap();

        let result =
            read_agent_shell_background_output(path.to_string_lossy().to_string()).unwrap();

        assert_eq!(result.output, "background output\n");
        assert!(!result.truncated);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_background_output_rejects_paths_outside_releash_temp_log_dir() {
        std::fs::create_dir_all(shell_output_dir()).unwrap();
        let err = read_agent_shell_background_output("/tmp/not-releash-shell.log".to_string())
            .unwrap_err();

        assert!(err.contains("outside Releash shell output directory"));
    }

    #[tokio::test]
    async fn shell_command_rejects_empty_command() {
        let err = run_agent_shell_command_inner(Path::new("."), " ", 1)
            .await
            .unwrap_err();

        assert_eq!(err, "Shell command is empty");
    }
}
