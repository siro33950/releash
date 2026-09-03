use std::path::{Path, PathBuf};

use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    ProviderSessionTitleEntry, ProviderSessionTitleGateway, ProviderSessionTitleGatewayError,
    ProviderSessionTitleRequest,
};
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) struct LocalAgentSessionHistoryGateway {
    claude_config_dir: PathBuf,
    codex_home: PathBuf,
}

impl LocalAgentSessionHistoryGateway {
    pub(crate) fn new(claude_config_dir: PathBuf, codex_home: PathBuf) -> Self {
        Self {
            claude_config_dir,
            codex_home,
        }
    }
}

#[async_trait::async_trait]
impl AgentSessionHistoryGateway for LocalAgentSessionHistoryGateway {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
        if worktree_path.trim().is_empty() || limit == 0 {
            return Err(AgentSessionHistoryGatewayError::InvalidRequest);
        }
        let worktree_path = worktree_path.to_string();
        let claude_config_dir = self.claude_config_dir.clone();
        let codex_home = self.codex_home.clone();
        tokio::task::spawn_blocking(move || match provider {
            ProviderKind::Claude => claude_metadata(&claude_config_dir, &worktree_path, limit),
            ProviderKind::Codex => codex_metadata(&codex_home, &worktree_path, limit),
        })
        .await
        .map_err(|_| AgentSessionHistoryGatewayError::Unavailable)?
    }

    async fn list_session_titles(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        provider_session_ids: &[String],
    ) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError> {
        if worktree_path.trim().is_empty()
            || provider_session_ids
                .iter()
                .any(|provider_session_id| provider_session_id.trim().is_empty())
        {
            return Err(AgentSessionHistoryGatewayError::InvalidRequest);
        }
        if provider_session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let claude_config_dir = self.claude_config_dir.clone();
        let codex_home = self.codex_home.clone();
        let worktree_path = worktree_path.to_string();
        let provider_session_ids = provider_session_ids.to_vec();
        let fallback_provider_session_ids = provider_session_ids.clone();
        let entries = tokio::task::spawn_blocking(move || match provider {
            ProviderKind::Claude => provider_session_ids
                .into_iter()
                .map(|provider_session_id| {
                    let session_title = claude_session_title(
                        &claude_config_dir,
                        &worktree_path,
                        &provider_session_id,
                        None,
                    )
                    .unwrap_or_else(|error| {
                        log::warn!(
                            "Claude provider history title read failed for {provider_session_id}: {error:?}"
                        );
                        None
                    });
                    let first_user_prompt = claude_first_user_prompt(
                        &claude_config_dir,
                        &worktree_path,
                        &provider_session_id,
                    )
                    .unwrap_or_else(|error| {
                        log::warn!(
                            "Claude provider history first prompt read failed for {provider_session_id}: {error:?}"
                        );
                        None
                    });
                    ProviderSessionTitleEntry {
                        provider_session_id,
                        session_title,
                        first_user_prompt,
                    }
                })
                .collect(),
            ProviderKind::Codex => codex_session_titles(&codex_home, &provider_session_ids)
                .unwrap_or_else(|error| {
                    log::warn!("Codex provider history title read failed: {error:?}");
                    provider_session_ids
                        .into_iter()
                        .map(|provider_session_id| ProviderSessionTitleEntry {
                            provider_session_id,
                            session_title: None,
                            first_user_prompt: None,
                        })
                        .collect()
                }),
        })
        .await
        .unwrap_or_else(|error| {
            log::warn!("provider history title worker failed: {error}");
            fallback_provider_session_ids
                .into_iter()
                .map(|provider_session_id| ProviderSessionTitleEntry {
                    provider_session_id,
                    session_title: None,
                    first_user_prompt: None,
                })
                .collect()
        });
        Ok(entries)
    }
}

#[async_trait::async_trait]
impl ProviderSessionTitleGateway for LocalAgentSessionHistoryGateway {
    async fn read_title(
        &self,
        request: ProviderSessionTitleRequest,
    ) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
        let claude_config_dir = self.claude_config_dir.clone();
        let codex_home = self.codex_home.clone();
        tokio::task::spawn_blocking(move || {
            provider_session_title(&claude_config_dir, &codex_home, request)
        })
        .await
        .map_err(|_| ProviderSessionTitleGatewayError::Unavailable)?
    }
}

const CLAUDE_TITLE_TAIL_BYTES: usize = 64 * 1024;
const CLAUDE_PROMPT_HEAD_BYTES: usize = 64 * 1024;

fn provider_session_title(
    claude_config_dir: &Path,
    codex_home: &Path,
    request: ProviderSessionTitleRequest,
) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
    match request.provider {
        ProviderKind::Claude => claude_session_title(
            claude_config_dir,
            &request.worktree_path,
            &request.provider_session_id,
            request.transcript_ref.as_deref(),
        ),
        ProviderKind::Codex => codex_session_title(codex_home, &request.provider_session_id),
    }
}

fn claude_session_title(
    config_dir: &Path,
    worktree_path: &str,
    provider_session_id: &str,
    transcript_ref: Option<&str>,
) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
    let transcript = transcript_ref.map(PathBuf::from).unwrap_or_else(|| {
        config_dir
            .join("projects")
            .join(claude_project_directory(worktree_path))
            .join(format!("{provider_session_id}.jsonl"))
    });
    let tail = crate::infrastructure::provider_history::read_file_tail(
        &transcript,
        CLAUDE_TITLE_TAIL_BYTES,
    )
    .map_err(|_| ProviderSessionTitleGatewayError::Unavailable)?;
    let mut lines = tail.bytes.split(|byte| *byte == b'\n');
    if tail.preceding_byte.is_some_and(|byte| byte != b'\n') {
        let _ = lines.next();
    }
    for line in lines.rev() {
        let line = trim_ascii_whitespace(line);
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("ai-title") {
            continue;
        }
        return value
            .get("aiTitle")
            .and_then(serde_json::Value::as_str)
            .map(|title| Some(title.to_string()))
            .ok_or(ProviderSessionTitleGatewayError::Corrupt);
    }
    Ok(None)
}

fn claude_first_user_prompt(
    config_dir: &Path,
    worktree_path: &str,
    provider_session_id: &str,
) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
    let transcript = config_dir
        .join("projects")
        .join(claude_project_directory(worktree_path))
        .join(format!("{provider_session_id}.jsonl"));
    let head = crate::infrastructure::provider_history::read_file_head(
        &transcript,
        CLAUDE_PROMPT_HEAD_BYTES,
    )
    .map_err(|_| ProviderSessionTitleGatewayError::Unavailable)?;
    let mut lines = head.bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    if head.following_byte.is_some_and(|byte| byte != b'\n')
        && head.bytes.last().is_some_and(|byte| *byte != b'\n')
    {
        let _ = lines.pop();
    }
    for line in lines {
        let line = trim_ascii_whitespace(line);
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("user")
            || value.get("isMeta").and_then(serde_json::Value::as_bool) == Some(true)
        {
            continue;
        }
        return Ok(claude_user_message_text(&value));
    }
    Ok(None)
}

fn claude_user_message_text(value: &serde_json::Value) -> Option<String> {
    let content = value.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let text = content
        .as_array()?
        .iter()
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn codex_session_title(
    codex_home: &Path,
    provider_session_id: &str,
) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
    Ok(
        codex_session_titles(codex_home, &[provider_session_id.to_string()])?
            .into_iter()
            .next()
            .and_then(|entry| entry.session_title),
    )
}

fn codex_session_titles(
    codex_home: &Path,
    provider_session_ids: &[String],
) -> Result<Vec<ProviderSessionTitleEntry>, ProviderSessionTitleGatewayError> {
    let database = codex_home.join("state_5.sqlite");
    if !database.exists() {
        return Err(ProviderSessionTitleGatewayError::Unavailable);
    }
    let threads = crate::infrastructure::provider_history::query_codex_thread_names(
        &database,
        provider_session_ids,
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(_, _) => ProviderSessionTitleGatewayError::Unavailable,
        _ => ProviderSessionTitleGatewayError::Corrupt,
    })?
    .into_iter()
    .map(|row| (row.session_id.clone(), row))
    .collect::<std::collections::HashMap<_, _>>();
    Ok(provider_session_ids
        .iter()
        .map(|provider_session_id| ProviderSessionTitleEntry {
            provider_session_id: provider_session_id.clone(),
            session_title: threads
                .get(provider_session_id)
                .and_then(|row| row.name.clone())
                .filter(|name| !name.is_empty()),
            first_user_prompt: threads
                .get(provider_session_id)
                .and_then(|row| row.first_user_message.clone()),
        })
        .collect())
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn claude_metadata(
    config_dir: &Path,
    worktree_path: &str,
    limit: usize,
) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
    let project_directory = config_dir
        .join("projects")
        .join(claude_project_directory(worktree_path));
    let files =
        crate::infrastructure::provider_history::recent_jsonl_files(&project_directory, limit)
            .map_err(|_| AgentSessionHistoryGatewayError::Unavailable)?;
    Ok(files
        .into_iter()
        .filter_map(|file| {
            let provider_session_id = file.path.file_stem()?.to_str()?.trim();
            if provider_session_id.is_empty() {
                return None;
            }
            Some(AgentSessionHistoryMetadata {
                provider: ProviderKind::Claude,
                provider_session_id: provider_session_id.to_string(),
                worktree_path: worktree_path.to_string(),
                updated_at_ms: file.updated_at_ms,
            })
        })
        .collect())
}

fn codex_metadata(
    codex_home: &Path,
    worktree_path: &str,
    limit: usize,
) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
    let rows = crate::infrastructure::provider_history::query_codex_history(
        &codex_home.join("state_5.sqlite"),
        worktree_path,
        limit,
    )
    .map_err(|_| AgentSessionHistoryGatewayError::Unavailable)?;
    Ok(rows
        .into_iter()
        .map(|row| AgentSessionHistoryMetadata {
            provider: ProviderKind::Codex,
            provider_session_id: row.session_id,
            worktree_path: row.cwd,
            updated_at_ms: row.updated_at_ms,
        })
        .collect())
}

fn claude_project_directory(worktree_path: &str) -> String {
    worktree_path
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}
