use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;

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

fn codex_child_env(
    session_id: &str,
    base_branch: Option<&str>,
    extra_env: &[(String, String)],
) -> AgentChildEnv {
    AgentChildEnv::for_session(session_id, base_branch, extra_env.iter().cloned(), [])
}

#[derive(Clone)]
pub(crate) struct CodexAppServerHandle {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pid_registration: Option<PidRegistration>,
}

impl CodexAppServerHandle {
    pub(crate) async fn write_json(&self, value: &Value) -> Result<(), String> {
        #[cfg(test)]
        if std::env::var_os("RELEASH_TEST_FAIL_CODEX_APP_SERVER_STDIN_WRITE").is_some() {
            return Err("injected codex app-server stdin write failure".to_string());
        }
        let line = encode_jsonl(value)?;
        let mut stdin = self.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            return Err("codex app-server stdin is closed".to_string());
        };
        stdin
            .write_all(&line)
            .await
            .map_err(|error| format!("failed to write codex app-server stdin: {error}"))
    }

    pub(crate) async fn shutdown(&self) {
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        staged_shutdown(&mut child, "codex app-server").await;
        if let Some(registration) = &self.pid_registration {
            registration.remove();
        }
    }
}

pub(crate) struct CodexAppServerProcess {
    handle: CodexAppServerHandle,
    stdout: StdoutLineReader<BufReader<ChildStdout>>,
}

impl CodexAppServerProcess {
    pub(crate) async fn spawn(
        cli_path: &str,
        session_id: &str,
        cwd: Option<&str>,
        base_branch: Option<&str>,
        extra_env: &[(String, String)],
    ) -> Result<Self, String> {
        wait_for_cleanup_gate().await;

        let mut command = Command::new(cli_path);
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        codex_child_env(session_id, base_branch, extra_env).apply(&mut command);
        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to spawn codex app-server: {error}"))?;
        let pid_registration = child
            .id()
            .and_then(|pid| save_pgid(None, session_id, "codex", pid));
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "codex app-server stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "codex app-server stdout is unavailable".to_string())?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_child_stderr("codex app-server", stderr));
        }
        Ok(Self {
            handle: CodexAppServerHandle {
                child: Arc::new(Mutex::new(child)),
                stdin: Arc::new(Mutex::new(Some(stdin))),
                pid_registration,
            },
            stdout: StdoutLineReader::with_wire_recorder(
                BufReader::new(stdout),
                WireRecorder::from_env(WireBackend::Codex),
            ),
        })
    }

    pub(crate) fn handle(&self) -> CodexAppServerHandle {
        self.handle.clone()
    }

    pub(crate) fn stdout_mut(&mut self) -> &mut StdoutLineReader<BufReader<ChildStdout>> {
        &mut self.stdout
    }

    pub(crate) async fn shutdown(mut self) {
        self.handle.shutdown().await;
        self.stdout.shutdown_wire_recorder().await;
    }
}

pub(crate) fn encode_jsonl(message: &Value) -> Result<Vec<u8>, String> {
    let mut line = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_jsonl_encode() {
        let line = encode_jsonl(&json!({"id": 1, "result": {}})).unwrap();
        assert!(line.ends_with(b"\n"));
        let decoded = serde_json::from_slice::<Value>(&line).unwrap();
        assert_eq!(decoded["id"], 1);
    }

    #[test]
    fn codex_child_env_includes_workflow_execution_ids() {
        let env = codex_child_env(
            "session-1",
            None,
            &[
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    "run-1".to_string(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    "node-1".to_string(),
                ),
            ],
        );

        assert!(env.envs().contains(&(
            "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
            "run-1".to_string()
        )));
        assert!(env.envs().contains(&(
            "RELEASH_NODE_EXECUTION_ID".to_string(),
            "node-1".to_string()
        )));
    }
}
