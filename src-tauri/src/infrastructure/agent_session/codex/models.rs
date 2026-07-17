use crate::domain::agent_session::gateway::{
    AgentBackend, AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
};
use crate::domain::agent_session::services::filter_agent_skills_for_query;
use crate::domain::agent_session::value_objects::{
    BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
};
use crate::infrastructure::agent_session::stdout_line_reader::{
    StdoutDiagnostics, StdoutItem, StdoutLineReader,
};
use serde_json::{json, Value};
use tokio::io::AsyncBufRead;

use super::app_server::CodexAppServerProcess;
use super::wire::{
    initialize_request, initialized_notification, request, PendingClientRequests,
    METHOD_FUZZY_FILE_SEARCH, METHOD_INITIALIZE, METHOD_SKILLS_LIST, METHOD_THREAD_ARCHIVE,
    METHOD_THREAD_FORK, METHOD_THREAD_UNARCHIVE,
};

pub(crate) const CODEX_BACKEND_ID: &str = "codex";

const CODEX_FIXED_MODELS: &[(&str, &str)] = &[
    ("gpt-5.6-sol", "GPT-5.6 Sol"),
    ("gpt-5.6-terra", "GPT-5.6 Terra"),
    ("gpt-5.6-luna", "GPT-5.6 Luna"),
];

#[derive(Debug, Clone)]
pub(crate) struct CodexBackend {
    cli_path: String,
}

impl CodexBackend {
    pub(crate) fn new(cli_path: Option<String>) -> Self {
        Self {
            cli_path: cli_path.unwrap_or_else(|| "codex".to_string()),
        }
    }

    pub(crate) fn cli_path(&self) -> &str {
        &self.cli_path
    }
}

#[async_trait::async_trait]
impl AgentBackend for CodexBackend {
    fn id(&self) -> &str {
        CODEX_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Codex"
    }

    fn available_models(&self) -> Vec<ModelDescriptor> {
        CODEX_FIXED_MODELS
            .iter()
            .filter_map(|(id, display_name)| {
                Some(ModelDescriptor {
                    id: ModelId::parse(*id).ok()?,
                    display_name: (*display_name).to_string(),
                })
            })
            .collect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { steering: false }
    }

    async fn open_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
        let runtime =
            super::session::CodexSessionRuntime::open(self.cli_path().to_string(), spec).await?;
        Ok(Box::new(runtime))
    }

    async fn archive_session(
        &self,
        backend_session_id: &str,
        cwd: &str,
    ) -> Result<(), AgentBackendError> {
        self.app_server_request(
            "codex-archive",
            Some(cwd),
            METHOD_THREAD_ARCHIVE,
            json!({ "threadId": backend_session_id }),
        )
        .await?;
        Ok(())
    }

    async fn unarchive_session(
        &self,
        backend_session_id: &str,
        cwd: &str,
    ) -> Result<(), AgentBackendError> {
        self.app_server_request(
            "codex-unarchive",
            Some(cwd),
            METHOD_THREAD_UNARCHIVE,
            json!({ "threadId": backend_session_id }),
        )
        .await?;
        Ok(())
    }

    async fn fork_session(
        &self,
        req: ForkSessionRequest,
    ) -> Result<Option<String>, AgentBackendError> {
        let mut params = json!({
            "threadId": req.backend_session_id,
            "cwd": req.cwd,
            "runtimeWorkspaceRoots": [req.cwd],
        });
        if let Some(model) = req.model {
            params["model"] = Value::String(model);
        }
        let result = self
            .app_server_request("codex-fork", Some(&req.cwd), METHOD_THREAD_FORK, params)
            .await?;
        Ok(thread_id_from_result(&result).map(str::to_string))
    }

    async fn skill_catalog(
        &self,
        cwd: &std::path::Path,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, AgentBackendError> {
        let mut skills = super::skills::scan_codex_agent_skills(cwd, None, None);
        match self
            .app_server_request(
                "codex-skills",
                cwd.to_str(),
                METHOD_SKILLS_LIST,
                json!({
                    "cwd": cwd,
                    "query": query,
                    "limit": limit,
                }),
            )
            .await
        {
            Ok(result) => skills.extend(skill_entries_from_result(&result)),
            Err(error) => log::warn!("codex skills/list failed, using local scan only: {error}"),
        }
        skills.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.name.cmp(&b.name)));
        skills.dedup_by(|a, b| a.scope == b.scope && a.name == b.name);
        Ok(filter_agent_skills_for_query(skills, query, limit))
    }

    async fn fuzzy_file_search(
        &self,
        root: &std::path::Path,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<String>>, AgentBackendError> {
        let result = self
            .app_server_request(
                "codex-fuzzy-file-search",
                root.to_str(),
                METHOD_FUZZY_FILE_SEARCH,
                json!({
                    "root": root,
                    "cwd": root,
                    "query": query,
                    "limit": limit,
                }),
            )
            .await?;
        Ok(Some(file_paths_from_result(&result)))
    }
}

impl CodexBackend {
    async fn app_server_request(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, AgentBackendError> {
        let mut process = CodexAppServerProcess::spawn(self.cli_path(), session_id, cwd, None, &[])
            .await
            .map_err(AgentBackendError::Other)?;
        let handle = process.handle();
        if let Err(error) = handle
            .write_json(&initialize_request(1, env!("CARGO_PKG_VERSION")))
            .await
        {
            process.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }
        if let Err(error) = handle.write_json(&initialized_notification()).await {
            process.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }
        if let Err(error) = handle.write_json(&request(2, method, params)).await {
            process.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }

        let mut pending_requests = PendingClientRequests::default();
        pending_requests.register(1, METHOD_INITIALIZE);
        pending_requests.register(2, method);
        let response_result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            read_one_shot_response(process.stdout_mut(), pending_requests, 2),
        )
        .await;
        process.shutdown().await;
        response_result
            .map_err(|_| AgentBackendError::StartupTimeout {
                retry_count: 0,
                max_retries: 0,
            })?
            .map_err(AgentBackendError::Other)
    }
}

async fn read_one_shot_response<R>(
    stdout: &mut StdoutLineReader<R>,
    mut pending_requests: PendingClientRequests,
    expected_id: u64,
) -> Result<Value, String>
where
    R: AsyncBufRead + Unpin,
{
    let mut diagnostics = StdoutDiagnostics::default();
    loop {
        let Some(item) = stdout.next().await? else {
            return Err("codex app-server exited before one-shot response".to_string());
        };
        let message = match item {
            StdoutItem::Json(message) => message,
            StdoutItem::NonJson { probe } => {
                diagnostics.record_non_json_skip("codex one-shot", &probe);
                continue;
            }
            StdoutItem::Oversize { probe } => {
                let _ = diagnostics.record_oversize_drop("codex one-shot", &probe);
                continue;
            }
        };
        let Some(response) = pending_requests.take_response(&message)? else {
            continue;
        };
        if let Some(error) = message.get("error") {
            return Err(error.to_string());
        }
        if response.id != expected_id {
            continue;
        }
        return Ok(message
            .get("result")
            .cloned()
            .expect("validated JSON-RPC response must contain result or error"));
    }
}

fn thread_id_from_result(result: &Value) -> Option<&str> {
    result
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .or_else(|| result.get("threadId").and_then(Value::as_str))
        .or_else(|| result.get("id").and_then(Value::as_str))
}

fn skill_entries_from_result(result: &Value) -> Vec<SkillEntry> {
    result_array(result, "skills")
        .into_iter()
        .filter_map(|skill| {
            let name = skill
                .get("name")
                .or_else(|| skill.get("id"))
                .and_then(Value::as_str)?;
            Some(SkillEntry {
                name: name.to_string(),
                description: skill
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                scope: normalize_skill_scope(
                    skill
                        .get("scope")
                        .or_else(|| skill.get("source"))
                        .and_then(Value::as_str),
                )
                .to_string(),
            })
        })
        .collect()
}

fn normalize_skill_scope(scope: Option<&str>) -> &str {
    match scope {
        Some("user" | "personal") => "personal",
        Some("repo" | "project" | "workspace") => "project",
        _ => "project",
    }
}

fn file_paths_from_result(result: &Value) -> Vec<String> {
    result_array(result, "files")
        .into_iter()
        .filter_map(|entry| {
            entry.as_str().map(str::to_string).or_else(|| {
                entry
                    .get("path")
                    .or_else(|| entry.get("filePath"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .collect()
}

fn result_array<'a>(result: &'a Value, key: &str) -> Vec<&'a Value> {
    result
        .get(key)
        .or_else(|| result.get("matches"))
        .and_then(Value::as_array)
        .or_else(|| result.as_array())
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[test]
    fn test_codex_models_固定順と表示名を返す() {
        let backend = CodexBackend::new(None);
        let models = backend.available_models();

        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        let names = models
            .iter()
            .map(|model| model.display_name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna",]);
        assert_eq!(names, vec!["GPT-5.6 Sol", "GPT-5.6 Terra", "GPT-5.6 Luna",]);
    }

    #[tokio::test]
    async fn test_one_shot_response必須field欠落は失敗として扱う() {
        let input = br#"{"id":2}
"#;
        let mut stdout = StdoutLineReader::new(BufReader::new(&input[..]));
        let mut pending = PendingClientRequests::default();
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_one_shot_response(&mut stdout, pending, 2).await;

        assert!(matches!(
            result,
            Err(message) if message.contains("expected exactly one of result or error")
        ));
    }

    #[tokio::test]
    async fn test_one_shot_responseは_initialize_errorを期待応答より先に返す() {
        let input = br#"{"id":1,"error":{"code":-32603,"message":"initialize failed"}}
{"id":2,"result":{"skills":[]}}
"#;
        let mut stdout = StdoutLineReader::new(BufReader::new(&input[..]));
        let mut pending = PendingClientRequests::default();
        pending.register(1, METHOD_INITIALIZE);
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_one_shot_response(&mut stdout, pending, 2).await;

        assert_eq!(
            result.unwrap_err(),
            json!({ "code": -32603, "message": "initialize failed" }).to_string()
        );
    }

    #[tokio::test]
    async fn test_one_shot_responseは非jsonと上限超過行をskipして正常応答を返す() {
        let payload = "x".repeat(64);
        let input = format!(
            "diagnostic output\n{{\"method\":\"ignored\",\"params\":{{\"data\":\"{payload}\"}}}}\n{{\"id\":2,\"result\":{{\"skills\":[]}}}}\n"
        );
        let mut stdout =
            StdoutLineReader::with_max_line_bytes(BufReader::new(input.as_bytes()), 48);
        let mut pending = PendingClientRequests::default();
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_one_shot_response(&mut stdout, pending, 2).await;

        assert_eq!(result.unwrap(), json!({ "skills": [] }));
    }

    #[test]
    fn test_skill_entries_from_result_scopeを正規化する() {
        let skills = skill_entries_from_result(&json!({
            "skills": [
                {"name": "review", "description": "Review", "scope": "repo"},
                {"name": "daily", "source": "user"}
            ]
        }));

        assert_eq!(
            skills,
            vec![
                SkillEntry {
                    name: "review".to_string(),
                    description: "Review".to_string(),
                    scope: "project".to_string(),
                },
                SkillEntry {
                    name: "daily".to_string(),
                    description: String::new(),
                    scope: "personal".to_string(),
                }
            ]
        );
    }

    #[test]
    fn test_file_paths_from_result_handles_matches() {
        assert_eq!(
            file_paths_from_result(&json!({"matches": [{"path": "src/lib.rs"}, "README.md"]})),
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
    }
}
