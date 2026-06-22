use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_thread_archive_request, build_thread_fork_request, build_thread_unarchive_request,
    CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::{
    close_agent_session_internal, AgentProcessMap,
};
use crate::usecase::agent_session::session::{
    AgentSessionRuntimeCloser, CodexThreadForkRequest, CodexThreadLifecycleGateway,
};

pub(crate) struct CodexThreadLifecycleAppServerGateway {
    app: tauri::AppHandle,
}

impl CodexThreadLifecycleAppServerGateway {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn cli_path(&self) -> String {
        configured_cli_path(&self.app).unwrap_or_else(|| "codex".to_string())
    }
}

#[async_trait::async_trait]
impl CodexThreadLifecycleGateway for CodexThreadLifecycleAppServerGateway {
    async fn archive_thread(&self, thread_id: &str) -> Result<(), String> {
        send_codex_thread_archive_request(&self.cli_path(), thread_id, true).await
    }

    async fn unarchive_thread(&self, thread_id: &str) -> Result<(), String> {
        send_codex_thread_archive_request(&self.cli_path(), thread_id, false).await
    }

    async fn fork_thread(&self, request: CodexThreadForkRequest) -> Result<String, String> {
        fork_codex_thread(&self.cli_path(), request).await
    }
}

async fn send_codex_thread_archive_request(
    cli_path: &str,
    thread_id: &str,
    archive: bool,
) -> Result<(), String> {
    let mut process = CodexAppServerProcess::spawn(cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        let request = if archive {
            build_thread_archive_request(id, thread_id)
        } else {
            build_thread_unarchive_request(id, thread_id)
        };
        process.send(&request).await?;
        process.read_response_result(id).await?;
        Ok(())
    }
    .await;
    process.shutdown().await;
    result
}

async fn fork_codex_thread(
    cli_path: &str,
    request: CodexThreadForkRequest,
) -> Result<String, String> {
    let mut process = CodexAppServerProcess::spawn(cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        let request = build_thread_fork_request(
            id,
            &request.thread_id,
            &request.cwd,
            request.model.as_deref(),
            Some(&request.permission_mode),
            request.plan_mode,
            request.permission_profile_id.as_deref(),
        )?;
        process.send(&request).await?;
        let response = process.read_response_result(id).await?;
        codex_thread_id_from_fork_response(&response)
            .ok_or_else(|| "Codex thread/fork response did not include thread.id".to_string())
    }
    .await;
    process.shutdown().await;
    result
}

fn codex_thread_id_from_fork_response(response: &Value) -> Option<String> {
    response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .or_else(|| response.get("threadId").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) struct TauriAgentSessionRuntimeCloser {
    app: tauri::AppHandle,
    handles: Arc<Mutex<AgentProcessMap>>,
}

impl TauriAgentSessionRuntimeCloser {
    pub(crate) fn new(app: tauri::AppHandle, handles: Arc<Mutex<AgentProcessMap>>) -> Self {
        Self { app, handles }
    }
}

#[async_trait::async_trait]
impl AgentSessionRuntimeCloser for TauriAgentSessionRuntimeCloser {
    async fn close_agent_session(&self, session_id: &str) -> Result<(), String> {
        close_agent_session_internal(&self.app, &self.handles, session_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_thread_id_from_fork_response_reads_thread_id() {
        let response = json!({
            "thread": {
                "id": "thread-forked",
                "sessionId": "tree-1"
            }
        });
        assert_eq!(
            codex_thread_id_from_fork_response(&response),
            Some("thread-forked".to_string())
        );

        assert_eq!(
            codex_thread_id_from_fork_response(&json!({ "threadId": "legacy-id" })),
            Some("legacy-id".to_string())
        );
        assert_eq!(codex_thread_id_from_fork_response(&json!({})), None);
    }
}
