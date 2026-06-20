use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::mcp::error::McpError;
use crate::domain::mcp::gateway::{AgentConfigGateway, AgentConfigGenerateResult};
use crate::domain::mcp::services::{mcp_url, ALL_AGENTS};
use crate::domain::mcp::value_objects::{AgentKind, McpConfigParams};

pub struct AgentConfigGatewayImpl {
    base_dir: Option<PathBuf>,
}

impl AgentConfigGatewayImpl {
    pub fn new() -> Self {
        Self { base_dir: None }
    }

    #[cfg(test)]
    fn with_base_dir(base_dir: PathBuf) -> Self {
        Self {
            base_dir: Some(base_dir),
        }
    }

    fn base_dir(&self) -> Result<PathBuf, McpError> {
        self.base_dir
            .clone()
            .or_else(dirs::home_dir)
            .ok_or_else(|| McpError::Gateway("ホームディレクトリの取得に失敗".to_string()))
    }
}

impl Default for AgentConfigGatewayImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentConfigGateway for AgentConfigGatewayImpl {
    fn configured_agents(&self) -> Result<Vec<String>, McpError> {
        Ok(get_configured_agents_at(&self.base_dir()?))
    }

    fn remove(&self, agent: AgentKind) -> Result<bool, McpError> {
        remove_releash_entry_at(agent, &self.base_dir()?).map_err(McpError::Gateway)
    }

    fn generate(
        &self,
        agent: AgentKind,
        params: &McpConfigParams,
    ) -> Result<AgentConfigGenerateResult, McpError> {
        generate_config_at(agent, params, &self.base_dir()?).map_err(McpError::Gateway)
    }

    fn preview(&self, agent: AgentKind, params: &McpConfigParams) -> Result<String, McpError> {
        preview_config(agent, params).map_err(McpError::Gateway)
    }
}

pub fn generate_config_at(
    agent: AgentKind,
    params: &McpConfigParams,
    base_dir: &Path,
) -> Result<AgentConfigGenerateResult, String> {
    let rel = agent.global_path();
    let file_path = base_dir.join(rel);

    let content = match agent {
        AgentKind::Claude => generate_claude_config(&file_path, params)?,
        AgentKind::Codex => generate_codex_config(&file_path, params)?,
        AgentKind::Gemini => generate_gemini_config(&file_path, params)?,
        AgentKind::Cursor => generate_cursor_config(&file_path, params)?,
    };

    Ok(AgentConfigGenerateResult {
        agent: agent.as_str().to_string(),
        file_path: file_path.to_string_lossy().to_string(),
        content,
    })
}

// ── JSON helpers ──

fn read_existing_json(path: &Path) -> Result<serde_json::Value, String> {
    if path.exists() {
        let s = fs::read_to_string(path)
            .map_err(|e| format!("設定ファイル読み込み失敗 {}: {e}", path.display()))?;
        serde_json::from_str(&s).map_err(|e| format!("JSONパース失敗 {}: {e}", path.display()))
    } else {
        Ok(serde_json::json!({}))
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("パーミッション設定失敗: {e}"))?;
    }
    Ok(())
}

// ── Claude Code ──

fn generate_claude_config(path: &Path, params: &McpConfigParams) -> Result<String, String> {
    let mut doc = read_existing_json(path)?;

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "type": "http",
        "url": mcp_url(params.port),
        "headers": {
            "Authorization": format!("Bearer {}", params.token)
        }
    });

    servers
        .as_object_mut()
        .ok_or("mcpServers is not an object")?
        .insert("releash".to_string(), entry);

    let content =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
    write_atomic(path, &content)?;
    Ok(content)
}

// ── Codex CLI ──

fn generate_codex_config(path: &Path, params: &McpConfigParams) -> Result<String, String> {
    let mut doc: toml::Value = if path.exists() {
        let s = fs::read_to_string(path)
            .map_err(|e| format!("設定ファイル読み込み失敗 {}: {e}", path.display()))?;
        toml::from_str(&s).map_err(|e| format!("TOMLパース失敗 {}: {e}", path.display()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = doc.as_table_mut().ok_or("TOML root is not a table")?;

    let mcp_servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));

    let mut entry = toml::map::Map::new();
    entry.insert("url".to_string(), toml::Value::String(mcp_url(params.port)));
    entry.insert(
        "bearer_token_env_var".to_string(),
        toml::Value::String("RELEASH_MCP_TOKEN".to_string()),
    );

    mcp_servers
        .as_table_mut()
        .ok_or("mcp_servers is not a table")?
        .insert("releash".to_string(), toml::Value::Table(entry));

    let content = toml::to_string_pretty(&doc).map_err(|e| format!("TOMLシリアライズ失敗: {e}"))?;
    write_atomic(path, &content)?;
    Ok(content)
}

// ── Gemini CLI ──

fn generate_gemini_config(path: &Path, params: &McpConfigParams) -> Result<String, String> {
    let mut doc = read_existing_json(path)?;

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "httpUrl": mcp_url(params.port),
        "headers": {
            "Authorization": format!("Bearer {}", params.token)
        }
    });

    servers
        .as_object_mut()
        .ok_or("mcpServers is not an object")?
        .insert("releash".to_string(), entry);

    let content =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
    write_atomic(path, &content)?;
    Ok(content)
}

// ── Cursor ──

fn generate_cursor_config(path: &Path, params: &McpConfigParams) -> Result<String, String> {
    let mut doc = read_existing_json(path)?;

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "url": mcp_url(params.port),
        "headers": {
            "Authorization": format!("Bearer {}", params.token)
        }
    });

    servers
        .as_object_mut()
        .ok_or("mcpServers is not an object")?
        .insert("releash".to_string(), entry);

    let content =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
    write_atomic(path, &content)?;
    Ok(content)
}

// ── Preview (no write) ──

pub fn preview_config(agent: AgentKind, params: &McpConfigParams) -> Result<String, String> {
    match agent {
        AgentKind::Claude => {
            let doc = serde_json::json!({
                "mcpServers": {
                    "releash": {
                        "type": "http",
                        "url": mcp_url(params.port),
                        "headers": {
                            "Authorization": format!("Bearer {}", params.token)
                        }
                    }
                }
            });
            serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))
        }
        AgentKind::Codex => {
            let content = format!(
                "[mcp_servers.releash]\nurl = \"{}\"\nbearer_token_env_var = \"RELEASH_MCP_TOKEN\"\n",
                mcp_url(params.port)
            );
            Ok(content)
        }
        AgentKind::Gemini => {
            let doc = serde_json::json!({
                "mcpServers": {
                    "releash": {
                        "httpUrl": mcp_url(params.port),
                        "headers": {
                            "Authorization": format!("Bearer {}", params.token)
                        }
                    }
                }
            });
            serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))
        }
        AgentKind::Cursor => {
            let doc = serde_json::json!({
                "mcpServers": {
                    "releash": {
                        "url": mcp_url(params.port),
                        "headers": {
                            "Authorization": format!("Bearer {}", params.token)
                        }
                    }
                }
            });
            serde_json::to_string_pretty(&doc).map_err(|e| format!("JSONシリアライズ失敗: {e}"))
        }
    }
}

// ── Configured agents detection ──

fn has_releash_entry(agent: AgentKind, base_dir: &Path) -> bool {
    let path = base_dir.join(agent.global_path());
    if !path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(&path) else {
        return false;
    };
    match agent {
        AgentKind::Codex => {
            // TOML: mcp_servers.releash
            let Ok(doc) = toml::from_str::<toml::Value>(&content) else {
                return false;
            };
            doc.get("mcp_servers")
                .and_then(|s| s.get("releash"))
                .is_some()
        }
        _ => {
            // JSON: mcpServers.releash
            let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) else {
                return false;
            };
            doc.get("mcpServers")
                .and_then(|s| s.get("releash"))
                .is_some()
        }
    }
}

fn remove_releash_entry_at(agent: AgentKind, base_dir: &Path) -> Result<bool, String> {
    let path = base_dir.join(agent.global_path());
    if !path.exists() {
        return Ok(false);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("設定ファイル読み込み失敗 {}: {e}", path.display()))?;

    match agent {
        AgentKind::Codex => {
            let mut doc: toml::Value = toml::from_str(&content)
                .map_err(|e| format!("TOMLパース失敗 {}: {e}", path.display()))?;
            let removed = doc
                .get_mut("mcp_servers")
                .and_then(|s| s.as_table_mut())
                .map(|t| t.remove("releash").is_some())
                .unwrap_or(false);
            if removed {
                let out = toml::to_string_pretty(&doc)
                    .map_err(|e| format!("TOMLシリアライズ失敗: {e}"))?;
                write_atomic(&path, &out)?;
            }
            Ok(removed)
        }
        _ => {
            let mut doc: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("JSONパース失敗 {}: {e}", path.display()))?;
            let removed = doc
                .get_mut("mcpServers")
                .and_then(|s| s.as_object_mut())
                .map(|o| o.remove("releash").is_some())
                .unwrap_or(false);
            if removed {
                let out = serde_json::to_string_pretty(&doc)
                    .map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
                write_atomic(&path, &out)?;
            }
            Ok(removed)
        }
    }
}

fn get_configured_agents_at(base_dir: &Path) -> Vec<String> {
    ALL_AGENTS
        .iter()
        .filter(|a| has_releash_entry(**a, base_dir))
        .map(|a| a.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp::services::{normalize_agent_types, validate_generation_credentials};
    use tempfile::TempDir;

    fn test_params() -> McpConfigParams {
        McpConfigParams {
            port: 19801,
            token: "test-token-abc123".to_string(),
        }
    }

    #[test]
    fn agent_kind_from_str_valid() {
        assert_eq!(AgentKind::parse("claude").unwrap(), AgentKind::Claude);
        assert_eq!(AgentKind::parse("codex").unwrap(), AgentKind::Codex);
        assert_eq!(AgentKind::parse("gemini").unwrap(), AgentKind::Gemini);
        assert_eq!(AgentKind::parse("cursor").unwrap(), AgentKind::Cursor);
    }

    #[test]
    fn agent_kind_from_str_invalid() {
        assert!(AgentKind::parse("unknown").is_err());
    }

    #[test]
    fn normalize_agent_types_deduplicates_and_trims() {
        let normalized = normalize_agent_types(vec![
            " Claude ".to_string(),
            "codex".to_string(),
            "claude".to_string(),
            "".to_string(),
        ])
        .unwrap();

        assert_eq!(normalized, vec!["claude", "codex"]);
    }

    #[test]
    fn normalize_agent_types_rejects_unknown_agent() {
        assert!(normalize_agent_types(vec!["unknown".to_string()]).is_err());
    }

    #[test]
    fn generation_credentials_are_not_required_for_deletion_only_changes() {
        assert!(validate_generation_credentials(false, 0, "").is_ok());
    }

    #[test]
    fn generation_credentials_are_required_when_agents_are_desired() {
        assert!(validate_generation_credentials(true, 0, "token").is_err());
        assert!(validate_generation_credentials(true, 19801, " ").is_err());
        assert!(validate_generation_credentials(true, 19801, "token").is_ok());
    }

    #[test]
    fn generate_claude_writes_global_config() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        let result = generate_config_at(AgentKind::Claude, &params, tmp.path()).unwrap();

        assert!(result.file_path.ends_with(".claude.json"));
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["mcpServers"]["releash"]["type"], "http");
        assert_eq!(
            parsed["mcpServers"]["releash"]["url"],
            "http://127.0.0.1:19801/mcp"
        );
        assert_eq!(
            parsed["mcpServers"]["releash"]["headers"]["Authorization"],
            "Bearer test-token-abc123"
        );
    }

    #[test]
    fn generate_codex_writes_global_config() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        let result = generate_config_at(AgentKind::Codex, &params, tmp.path()).unwrap();

        assert!(result.file_path.ends_with(".codex/config.toml"));
        assert!(result
            .content
            .contains("url = \"http://127.0.0.1:19801/mcp\""));
        assert!(result
            .content
            .contains("bearer_token_env_var = \"RELEASH_MCP_TOKEN\""));
    }

    #[test]
    fn generate_gemini_writes_global_config() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        let result = generate_config_at(AgentKind::Gemini, &params, tmp.path()).unwrap();

        assert!(result.file_path.ends_with(".gemini/settings.json"));
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["releash"]["httpUrl"],
            "http://127.0.0.1:19801/mcp"
        );
    }

    #[test]
    fn generate_cursor_writes_global_config() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        let result = generate_config_at(AgentKind::Cursor, &params, tmp.path()).unwrap();

        assert!(result.file_path.ends_with(".cursor/mcp.json"));
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["releash"]["url"],
            "http://127.0.0.1:19801/mcp"
        );
    }

    #[test]
    fn generate_merges_with_existing_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        let existing = serde_json::json!({
            "mcpServers": {
                "other-server": {
                    "type": "http",
                    "url": "http://localhost:3000/mcp"
                }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        let params = test_params();
        let result = generate_config_at(AgentKind::Claude, &params, tmp.path()).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed["mcpServers"]["other-server"].is_object());
        assert!(parsed["mcpServers"]["releash"].is_object());
    }

    #[test]
    fn preview_does_not_write_file() {
        let params = test_params();
        let content = preview_config(AgentKind::Claude, &params).unwrap();
        assert!(content.contains("releash"));
        assert!(content.contains("19801"));
    }

    #[test]
    fn global_path_returns_correct_paths() {
        assert_eq!(AgentKind::Claude.global_path(), ".claude.json");
        assert_eq!(AgentKind::Codex.global_path(), ".codex/config.toml");
        assert_eq!(AgentKind::Gemini.global_path(), ".gemini/settings.json");
        assert_eq!(AgentKind::Cursor.global_path(), ".cursor/mcp.json");
    }

    #[test]
    fn to_str_roundtrips() {
        for kind in &ALL_AGENTS {
            assert_eq!(AgentKind::parse(kind.as_str()).unwrap(), *kind);
        }
    }

    #[test]
    fn configured_agents_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(get_configured_agents_at(tmp.path()).is_empty());
    }

    #[test]
    fn configured_agents_detects_claude() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        generate_config_at(AgentKind::Claude, &params, tmp.path()).unwrap();
        let agents = get_configured_agents_at(tmp.path());
        assert_eq!(agents, vec!["claude"]);
    }

    #[test]
    fn configured_agents_detects_codex() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        generate_config_at(AgentKind::Codex, &params, tmp.path()).unwrap();
        let agents = get_configured_agents_at(tmp.path());
        assert_eq!(agents, vec!["codex"]);
    }

    #[test]
    fn configured_agents_detects_all() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        for kind in &ALL_AGENTS {
            generate_config_at(*kind, &params, tmp.path()).unwrap();
        }
        let agents = get_configured_agents_at(tmp.path());
        assert_eq!(agents, vec!["claude", "codex", "gemini", "cursor"]);
    }

    #[test]
    fn configured_agents_ignores_json_without_releash() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        let doc = serde_json::json!({
            "mcpServers": {
                "other-server": { "type": "http" }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        assert!(get_configured_agents_at(tmp.path()).is_empty());
    }

    #[test]
    fn configured_agents_ignores_broken_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(&path, "not valid json {{{").unwrap();
        assert!(get_configured_agents_at(tmp.path()).is_empty());
    }

    #[test]
    fn remove_releash_entry_json() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        generate_config_at(AgentKind::Claude, &params, tmp.path()).unwrap();
        assert!(has_releash_entry(AgentKind::Claude, tmp.path()));

        let removed = remove_releash_entry_at(AgentKind::Claude, tmp.path()).unwrap();
        assert!(removed);
        assert!(!has_releash_entry(AgentKind::Claude, tmp.path()));
    }

    #[test]
    fn remove_releash_entry_preserves_other_servers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        let doc = serde_json::json!({
            "mcpServers": {
                "other-server": { "type": "http", "url": "http://localhost:3000" },
                "releash": { "type": "http", "url": "http://127.0.0.1:19801/mcp" }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

        let removed = remove_releash_entry_at(AgentKind::Claude, tmp.path()).unwrap();
        assert!(removed);

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["mcpServers"]["other-server"].is_object());
        assert!(parsed["mcpServers"]["releash"].is_null());
    }

    #[test]
    fn remove_releash_entry_codex() {
        let tmp = TempDir::new().unwrap();
        let params = test_params();
        generate_config_at(AgentKind::Codex, &params, tmp.path()).unwrap();
        assert!(has_releash_entry(AgentKind::Codex, tmp.path()));

        let removed = remove_releash_entry_at(AgentKind::Codex, tmp.path()).unwrap();
        assert!(removed);
        assert!(!has_releash_entry(AgentKind::Codex, tmp.path()));
    }

    #[test]
    fn remove_releash_entry_missing_file() {
        let tmp = TempDir::new().unwrap();
        let removed = remove_releash_entry_at(AgentKind::Claude, tmp.path()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_releash_entry_no_releash_key() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        let doc = serde_json::json!({
            "mcpServers": {
                "other-server": { "type": "http" }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

        let removed = remove_releash_entry_at(AgentKind::Claude, tmp.path()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_agent_config_gateway_impl_設定生成と検出と削除ができる() {
        // Given
        let tmp = TempDir::new().unwrap();
        let gateway = AgentConfigGatewayImpl::with_base_dir(tmp.path().to_path_buf());
        let params = test_params();

        // When
        gateway.generate(AgentKind::Claude, &params).unwrap();
        let configured = gateway.configured_agents().unwrap();
        let removed = gateway.remove(AgentKind::Claude).unwrap();

        // Then
        assert_eq!(configured, vec!["claude"]);
        assert!(removed);
        assert!(gateway.configured_agents().unwrap().is_empty());
    }
}
