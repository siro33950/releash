use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::infrastructure::agent_session::stdout_line_reader::StdoutLineReader;
use crate::infrastructure::agent_session::wire_record::{WireBackend, WireRecorder};
use crate::infrastructure::process::child_env::AgentChildEnv;
use crate::infrastructure::process::child_process::{configure_process_group, staged_shutdown};
use crate::infrastructure::process::child_stderr::drain_child_stderr;
use crate::infrastructure::process::pid_registry::{
    save_pgid, wait_for_cleanup_gate, PidRegistration,
};

const CLAUDE_SCRUBBED_ENV: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeProcessConfig {
    pub cli_path: String,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub session_id: String,
    pub base_branch: Option<String>,
    pub extra_env: Vec<(String, String)>,
    pub system_prompt: Option<String>,
}

fn claude_child_env(config: &ClaudeProcessConfig) -> AgentChildEnv {
    AgentChildEnv::for_session(
        &config.session_id,
        config.base_branch.as_deref(),
        config
            .env
            .iter()
            .cloned()
            .chain(config.extra_env.iter().cloned()),
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
    stdout: StdoutLineReader<BufReader<ChildStdout>>,
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
    pub(crate) async fn spawn(mut config: ClaudeProcessConfig) -> Result<Self, String> {
        verify_claude_cli_version(&config.cli_path).await?;
        let mut system_prompt_file = None;
        if let Some(system_prompt) = config
            .system_prompt
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let mut file = tempfile::NamedTempFile::new()
                .map_err(|error| format!("failed to create claude system prompt file: {error}"))?;
            file.write_all(system_prompt.as_bytes())
                .map_err(|error| format!("failed to write claude system prompt file: {error}"))?;
            config.args.push("--append-system-prompt-file".to_string());
            config.args.push(file.path().to_string_lossy().to_string());
            system_prompt_file = Some(file);
        }
        wait_for_cleanup_gate().await;

        let mut command = Command::new(&config.cli_path);
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        claude_child_env(&config).apply(&mut command);
        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to spawn claude CLI: {error}"))?;
        let pid_registration = child
            .id()
            .and_then(|pid| save_pgid(None, &config.session_id, "claude", pid));
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
            stdout: StdoutLineReader::with_wire_recorder(
                BufReader::new(stdout),
                WireRecorder::from_env(WireBackend::Claude),
            ),
            _system_prompt_file: system_prompt_file,
        })
    }

    pub(crate) fn handle(&self) -> ClaudeStdioHandle {
        self.handle.clone()
    }

    pub(crate) fn stdout_mut(&mut self) -> &mut StdoutLineReader<BufReader<ChildStdout>> {
        &mut self.stdout
    }

    pub(crate) async fn shutdown_wire_recorder(&mut self) {
        self.stdout.shutdown_wire_recorder().await;
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
    use super::*;

    #[test]
    fn claude_child_env_includes_workflow_execution_ids() {
        let config = ClaudeProcessConfig {
            cli_path: "claude".to_string(),
            cwd: PathBuf::from("/repo"),
            args: Vec::new(),
            env: Vec::new(),
            session_id: "session-1".to_string(),
            base_branch: Some("main".to_string()),
            extra_env: vec![
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    "run-1".to_string(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    "node-1".to_string(),
                ),
            ],
            system_prompt: None,
        };
        let env = claude_child_env(&config);

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
    fn test_first_semver_parses_claude_version() {
        assert_eq!(first_semver("Claude Code 2.1.3"), Some((2, 1, 3)));
        assert_eq!(first_semver("claude 2.0"), Some((2, 0, 0)));
        assert_eq!(first_semver("no version"), None);
    }
}
