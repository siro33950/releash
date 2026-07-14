use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::domain::agent_session::gateway::SessionSpec;
use crate::infrastructure::process::child_env::AgentChildEnv;
use crate::infrastructure::process::child_process::{configure_process_group, staged_shutdown};
use crate::infrastructure::process::child_stderr::drain_child_stderr;
use crate::infrastructure::process::pid_registry::{
    save_pgid, wait_for_cleanup_gate, PidRegistration,
};

use super::wire::{claude_wire_mode, ClaudeWireMode};

pub(crate) const MAX_CLAUDE_STDOUT_LINE_BYTES: usize = 8 * 1024 * 1024;
const CLAUDE_SCRUBBED_ENV: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

#[derive(Debug)]
pub(crate) enum ClaudeStdoutItem {
    Json(Value),
    OversizeDropped { bytes: usize },
}

#[derive(Debug)]
enum ClaudeStdoutLine {
    Line(Vec<u8>),
    Oversize { bytes: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeProcessConfig {
    pub cli_path: String,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn claude_child_env(spec: &SessionSpec, config: &ClaudeProcessConfig) -> AgentChildEnv {
    AgentChildEnv::for_session(
        &spec.session_id,
        spec.base_branch.as_deref(),
        config
            .env
            .iter()
            .cloned()
            .chain(spec.extra_env.iter().cloned()),
        CLAUDE_SCRUBBED_ENV.iter().copied(),
    )
}

#[derive(Clone)]
pub(crate) struct ClaudeStdioHandle {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pid_registration: Option<PidRegistration>,
}

pub(crate) struct ClaudeStdioProcess {
    handle: ClaudeStdioHandle,
    stdout: BufReader<ChildStdout>,
    _system_prompt_file: Option<tempfile::NamedTempFile>,
}

impl ClaudeStdioHandle {
    pub(crate) async fn write_json(&self, value: &Value) -> Result<(), String> {
        #[cfg(test)]
        if std::env::var_os("RELEASH_TEST_FAIL_CLAUDE_STDIN_WRITE").is_some() {
            return Err("injected claude stdin write failure".to_string());
        }
        let mut line = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            return Err("claude stdin is closed".to_string());
        };
        stdin
            .write_all(&line)
            .await
            .map_err(|error| format!("failed to write claude stdin: {error}"))
    }

    pub(crate) async fn shutdown(&self) {
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        staged_shutdown(&mut child, "claude").await;
        if let Some(registration) = &self.pid_registration {
            registration.remove();
        }
    }
}

impl ClaudeStdioProcess {
    pub(crate) async fn spawn(cli_path: String, spec: &SessionSpec) -> Result<Self, String> {
        verify_claude_cli_version(&cli_path).await?;
        let mut system_prompt_file = None;
        if let Some(system_prompt) = spec
            .system_prompt
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let mut file = tempfile::NamedTempFile::new()
                .map_err(|error| format!("failed to create claude system prompt file: {error}"))?;
            file.write_all(system_prompt.as_bytes())
                .map_err(|error| format!("failed to write claude system prompt file: {error}"))?;
            system_prompt_file = Some(file);
        }

        let config = build_process_config(
            cli_path,
            spec,
            system_prompt_file
                .as_ref()
                .map(tempfile::NamedTempFile::path),
        );
        wait_for_cleanup_gate().await;

        let mut command = Command::new(&config.cli_path);
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        claude_child_env(spec, &config).apply(&mut command);
        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to spawn claude CLI: {error}"))?;
        let pid_registration = child
            .id()
            .and_then(|pid| save_pgid(None, &spec.session_id, "claude", pid));
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "claude stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "claude stdout is unavailable".to_string())?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_child_stderr("claude", stderr));
        }
        Ok(Self {
            handle: ClaudeStdioHandle {
                child: Arc::new(Mutex::new(child)),
                stdin: Arc::new(Mutex::new(Some(stdin))),
                pid_registration,
            },
            stdout: BufReader::new(stdout),
            _system_prompt_file: system_prompt_file,
        })
    }

    pub(crate) fn handle(&self) -> ClaudeStdioHandle {
        self.handle.clone()
    }

    pub(crate) async fn next_json(&mut self) -> Result<Option<ClaudeStdoutItem>, String> {
        next_stdout_item(&mut self.stdout).await
    }
}

async fn next_stdout_item<R>(stdout: &mut R) -> Result<Option<ClaudeStdoutItem>, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    loop {
        match read_stdout_line_limited(stdout).await? {
            None => return Ok(None),
            Some(ClaudeStdoutLine::Oversize { bytes }) => {
                return Ok(Some(ClaudeStdoutItem::OversizeDropped { bytes }));
            }
            Some(ClaudeStdoutLine::Line(line)) => match serde_json::from_slice::<Value>(&line) {
                Ok(value) => return Ok(Some(ClaudeStdoutItem::Json(value))),
                Err(error) => {
                    log::warn!("skipping non-json claude stdout line: {error}");
                }
            },
        }
    }
}

#[cfg(test)]
mod child_env_tests {
    use super::*;

    #[test]
    fn claude_child_env_scrubs_legacy_claude_env_names() {
        let env = AgentChildEnv::for_session(
            "session-1",
            None,
            Vec::<(String, String)>::new(),
            CLAUDE_SCRUBBED_ENV.iter().copied(),
        );

        assert!(env.scrub_envs().contains(&"CLAUDECODE".to_string()));
        assert!(env
            .scrub_envs()
            .contains(&"CLAUDE_CODE_ENTRYPOINT".to_string()));
    }
}

async fn read_stdout_line_limited<R>(stdout: &mut R) -> Result<Option<ClaudeStdoutLine>, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = stdout
            .fill_buf()
            .await
            .map_err(|error| format!("failed to read claude stdout: {error}"))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(ClaudeStdoutLine::Line(line)))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if line.len().saturating_add(take) > MAX_CLAUDE_STDOUT_LINE_BYTES {
            log::warn!(
                "claude stdout line exceeded {} bytes",
                MAX_CLAUDE_STDOUT_LINE_BYTES
            );
            let mut dropped = line.len().saturating_add(take);
            line.clear();
            let reached_newline = available[..take].last() == Some(&b'\n');
            stdout.consume(take);
            if reached_newline {
                return Ok(Some(ClaudeStdoutLine::Oversize { bytes: dropped - 1 }));
            }
            loop {
                let available = stdout
                    .fill_buf()
                    .await
                    .map_err(|error| format!("failed to read claude stdout: {error}"))?;
                if available.is_empty() {
                    return Ok(Some(ClaudeStdoutLine::Oversize { bytes: dropped }));
                }
                let take = available
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map(|index| index + 1)
                    .unwrap_or(available.len());
                let reached_newline = available[..take].last() == Some(&b'\n');
                dropped = dropped.saturating_add(take);
                stdout.consume(take);
                if reached_newline {
                    return Ok(Some(ClaudeStdoutLine::Oversize { bytes: dropped - 1 }));
                }
            }
        }
        let reached_newline = available[..take].last() == Some(&b'\n');
        line.extend_from_slice(&available[..take]);
        stdout.consume(take);
        if reached_newline {
            return Ok(Some(ClaudeStdoutLine::Line(line)));
        }
    }
}

pub(crate) fn build_process_config(
    cli_path: impl Into<String>,
    spec: &SessionSpec,
    system_prompt_file: Option<&std::path::Path>,
) -> ClaudeProcessConfig {
    ClaudeProcessConfig {
        cli_path: cli_path.into(),
        cwd: PathBuf::from(&spec.cwd),
        args: build_args(spec, system_prompt_file),
        env: watchdog_env(spec.stale_timeout),
    }
}

pub(crate) fn build_args(
    spec: &SessionSpec,
    system_prompt_file: Option<&std::path::Path>,
) -> Vec<String> {
    let mut args = vec![
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--permission-prompt-tool".to_string(),
        "stdio".to_string(),
        "--allow-dangerously-skip-permissions".to_string(),
        "--setting-sources".to_string(),
        "user,project".to_string(),
        "--permission-mode".to_string(),
        wire_mode_for_spec(spec).as_str().to_string(),
        "--model".to_string(),
        spec.model.as_str().to_string(),
    ];
    if let Some(resume) = spec.resume.as_deref().filter(|value| !value.is_empty()) {
        args.push("--resume".to_string());
        args.push(resume.to_string());
    }
    if let Some(path) = system_prompt_file {
        args.push("--append-system-prompt-file".to_string());
        args.push(path.to_string_lossy().to_string());
    }
    args
}

pub(crate) fn watchdog_env(stale_timeout: Option<Duration>) -> Vec<(String, String)> {
    let mut env = vec![
        ("CLAUDE_CODE_MAX_RETRIES".to_string(), "10".to_string()),
        ("API_TIMEOUT_MS".to_string(), "600000".to_string()),
    ];
    if let Some(stale_timeout) = stale_timeout {
        env.extend([
            ("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "1".to_string()),
            ("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "1".to_string()),
            (
                "CLAUDE_STREAM_IDLE_TIMEOUT_MS".to_string(),
                stale_timeout.as_millis().to_string(),
            ),
        ]);
    } else {
        env.extend([
            ("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "0".to_string()),
            ("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "0".to_string()),
        ]);
    }
    env
}

pub(crate) fn wire_mode_for_spec(spec: &SessionSpec) -> ClaudeWireMode {
    claude_wire_mode(spec.permission_mode, spec.plan_mode)
}

async fn verify_claude_cli_version(cli_path: &str) -> Result<(), String> {
    let output = Command::new(cli_path)
        .arg("--version")
        .output()
        .await
        .map_err(|error| format!("failed to run claude --version: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "claude --version failed with status {}",
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}\n{stderr}");
    let Some((major, minor, patch)) = first_semver(&text) else {
        return Err(format!(
            "failed to parse claude CLI version from: {}",
            text.trim()
        ));
    };
    if (major, minor, patch) < (2, 0, 0) {
        return Err(format!(
            "claude CLI >= 2.0.0 is required, found {major}.{minor}.{patch}"
        ));
    }
    Ok(())
}

fn first_semver(text: &str) -> Option<(u64, u64, u64)> {
    text.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find_map(|candidate| {
            let mut parts = candidate.split('.');
            let major = parts.next()?.parse().ok()?;
            let minor = parts.next()?.parse().ok()?;
            let patch = parts.next().unwrap_or("0").parse().ok()?;
            Some((major, minor, patch))
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::domain::agent_session::gateway::SessionSpec;
    use crate::domain::agent_session::value_objects::{ModelId, PermissionMode};

    use super::*;

    fn spec() -> SessionSpec {
        SessionSpec {
            session_id: "s1".to_string(),
            cwd: "/repo".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            permission_profile_id: None,
            model: ModelId::parse("claude-sonnet-4-5").unwrap(),
            system_prompt: Some("system".to_string()),
            resume: Some("backend-session".to_string()),
            base_branch: Some("main".to_string()),
            startup_timeout: None,
            startup_max_retries: None,
            stale_timeout: Some(Duration::from_secs(42)),
            extra_env: Vec::new(),
        }
    }

    #[test]
    fn test_build_args_design_必須フラグを含む() {
        let args = build_args(&spec(), Some(std::path::Path::new("/tmp/system.txt")));

        assert!(args.contains(&"--input-format".to_string()));
        assert!(args.contains(&"--include-partial-messages".to_string()));
        assert!(args.contains(&"--permission-prompt-tool".to_string()));
        assert!(args.contains(&"stdio".to_string()));
        assert!(args.contains(&"--allow-dangerously-skip-permissions".to_string()));
        assert!(args.contains(&"--resume".to_string()));
        assert!(args.contains(&"backend-session".to_string()));
        assert!(args.contains(&"--append-system-prompt-file".to_string()));
    }

    #[test]
    fn test_watchdog_env_stale_timeout_ms() {
        let env = watchdog_env(Some(Duration::from_secs(42)));
        assert!(env.contains(&(
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS".to_string(),
            "42000".to_string()
        )));
        assert!(env.contains(&("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "1".to_string())));
        assert!(env.contains(&("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "1".to_string())));
        assert!(env.contains(&("CLAUDE_CODE_MAX_RETRIES".to_string(), "10".to_string())));
    }

    #[test]
    fn claude_child_env_includes_workflow_execution_ids() {
        let mut spec = spec();
        spec.extra_env = vec![
            (
                "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                "run-1".to_string(),
            ),
            (
                "RELEASH_NODE_EXECUTION_ID".to_string(),
                "node-1".to_string(),
            ),
        ];
        let config = build_process_config("claude".to_string(), &spec, None);
        let env = claude_child_env(&spec, &config);

        assert!(env.envs().contains(&(
            "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
            "run-1".to_string()
        )));
        assert!(env.envs().contains(&(
            "RELEASH_NODE_EXECUTION_ID".to_string(),
            "node-1".to_string()
        )));
    }

    #[test]
    fn test_watchdog_env_none_disables_stream_watchdogs() {
        let env = watchdog_env(None);

        assert!(!env
            .iter()
            .any(|(key, _)| key == "CLAUDE_STREAM_IDLE_TIMEOUT_MS"));
        assert!(env.contains(&("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "0".to_string())));
        assert!(env.contains(&("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "0".to_string())));
        assert!(env.contains(&("CLAUDE_CODE_MAX_RETRIES".to_string(), "10".to_string())));
    }

    #[test]
    fn test_first_semver_parses_claude_version() {
        assert_eq!(first_semver("Claude Code 2.1.3"), Some((2, 1, 3)));
        assert_eq!(first_semver("claude 2.0"), Some((2, 0, 0)));
        assert_eq!(first_semver("no version"), None);
    }

    #[tokio::test]
    async fn test_next_stdout_item_8mb未満の巨大行を正常にパースする() {
        let payload = "a".repeat(MAX_CLAUDE_STDOUT_LINE_BYTES - 1024);
        let input = format!("{{\"data\":\"{payload}\"}}\n");
        let mut reader = tokio::io::BufReader::with_capacity(64 * 1024, input.as_bytes());

        let item = next_stdout_item(&mut reader).await.unwrap();

        match item {
            Some(ClaudeStdoutItem::Json(value)) => {
                assert_eq!(value["data"].as_str().unwrap().len(), payload.len());
            }
            other => panic!("expected Json, got {other:?}"),
        }
        assert!(next_stdout_item(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_next_stdout_item_超過行はoversize_droppedを返し後続行の処理を継続する() {
        let payload = "b".repeat(MAX_CLAUDE_STDOUT_LINE_BYTES + 1);
        let input = format!("{{\"data\":\"{payload}\"}}\n{{\"ok\":true}}\n");
        let mut reader = tokio::io::BufReader::with_capacity(64 * 1024, input.as_bytes());

        let first = next_stdout_item(&mut reader).await.unwrap();
        assert!(matches!(
            first,
            Some(ClaudeStdoutItem::OversizeDropped { bytes })
                if bytes > MAX_CLAUDE_STDOUT_LINE_BYTES
        ));

        let second = next_stdout_item(&mut reader).await.unwrap();
        match second {
            Some(ClaudeStdoutItem::Json(value)) => {
                assert_eq!(value["ok"], serde_json::json!(true));
            }
            other => panic!("expected Json, got {other:?}"),
        }
        assert!(next_stdout_item(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_next_stdout_item_改行なしeofの超過行もoversize_droppedとして可視化する() {
        let payload = "c".repeat(MAX_CLAUDE_STDOUT_LINE_BYTES + 1);
        let mut reader = tokio::io::BufReader::with_capacity(64 * 1024, payload.as_bytes());

        let first = next_stdout_item(&mut reader).await.unwrap();
        assert!(matches!(
            first,
            Some(ClaudeStdoutItem::OversizeDropped { bytes })
                if bytes == MAX_CLAUDE_STDOUT_LINE_BYTES + 1
        ));
        assert!(next_stdout_item(&mut reader).await.unwrap().is_none());
    }
}
