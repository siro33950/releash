use std::path::{Path, PathBuf};

use crate::domain::agent_session::{
    ProviderAgentSessionHistoryGateway, ProviderAgentSessionHistoryGatewayError,
    ProviderAgentSessionHistoryMetadata,
};
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) struct LocalProviderAgentSessionHistoryGateway {
    claude_config_dir: PathBuf,
    codex_home: PathBuf,
}

impl LocalProviderAgentSessionHistoryGateway {
    pub(crate) fn new(claude_config_dir: PathBuf, codex_home: PathBuf) -> Self {
        Self {
            claude_config_dir,
            codex_home,
        }
    }
}

#[async_trait::async_trait]
impl ProviderAgentSessionHistoryGateway for LocalProviderAgentSessionHistoryGateway {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<ProviderAgentSessionHistoryMetadata>, ProviderAgentSessionHistoryGatewayError>
    {
        if worktree_path.trim().is_empty() || limit == 0 {
            return Err(ProviderAgentSessionHistoryGatewayError::InvalidRequest);
        }
        let worktree_path = worktree_path.to_string();
        let claude_config_dir = self.claude_config_dir.clone();
        let codex_home = self.codex_home.clone();
        tokio::task::spawn_blocking(move || match provider {
            ProviderKind::Claude => claude_metadata(&claude_config_dir, &worktree_path, limit),
            ProviderKind::Codex => codex_metadata(&codex_home, &worktree_path, limit),
        })
        .await
        .map_err(|_| ProviderAgentSessionHistoryGatewayError::Unavailable)?
    }
}

fn claude_metadata(
    config_dir: &Path,
    worktree_path: &str,
    limit: usize,
) -> Result<Vec<ProviderAgentSessionHistoryMetadata>, ProviderAgentSessionHistoryGatewayError> {
    let project_directory = config_dir
        .join("projects")
        .join(claude_project_directory(worktree_path));
    let files =
        crate::infrastructure::provider_history::recent_jsonl_files(&project_directory, limit)
            .map_err(|_| ProviderAgentSessionHistoryGatewayError::Unavailable)?;
    Ok(files
        .into_iter()
        .filter_map(|file| {
            let provider_session_id = file.path.file_stem()?.to_str()?.trim();
            if provider_session_id.is_empty() {
                return None;
            }
            Some(ProviderAgentSessionHistoryMetadata {
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
) -> Result<Vec<ProviderAgentSessionHistoryMetadata>, ProviderAgentSessionHistoryGatewayError> {
    let rows = crate::infrastructure::provider_history::query_codex_history(
        &codex_home.join("state_5.sqlite"),
        worktree_path,
        limit,
    )
    .map_err(|_| ProviderAgentSessionHistoryGatewayError::Unavailable)?;
    Ok(rows
        .into_iter()
        .map(|row| ProviderAgentSessionHistoryMetadata {
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
