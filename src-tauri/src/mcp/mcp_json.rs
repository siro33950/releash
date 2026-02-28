use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Cursor,
}

impl AgentKind {
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            "cursor" => Ok(Self::Cursor),
            _ => Err(format!("Unknown agent type: {s}")),
        }
    }

    fn global_path(&self) -> &'static str {
        match self {
            Self::Claude => ".claude.json",
            Self::Codex => ".codex/config.toml",
            Self::Gemini => ".gemini/settings.json",
            Self::Cursor => ".cursor/mcp.json",
        }
    }
}

pub struct McpConfigParams {
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub file_path: String,
    pub content: String,
}

pub fn generate_config(
    agent: AgentKind,
    params: &McpConfigParams,
) -> Result<GenerateResult, String> {
    let home = dirs::home_dir().ok_or("ホームディレクトリの取得に失敗")?;
    let rel = agent.global_path();
    let file_path = home.join(rel);

    let content = match agent {
        AgentKind::Claude => generate_claude_config(&file_path, params)?,
        AgentKind::Codex => generate_codex_config(&file_path, params)?,
        AgentKind::Gemini => generate_gemini_config(&file_path, params)?,
        AgentKind::Cursor => generate_cursor_config(&file_path, params)?,
    };

    Ok(GenerateResult {
        file_path: file_path.to_string_lossy().to_string(),
        content,
    })
}

fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

// ── JSON helpers ──

fn read_existing_json(path: &Path) -> serde_json::Value {
    if path.exists() {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;
    Ok(())
}

// ── Claude Code ──

fn generate_claude_config(path: &Path, params: &McpConfigParams) -> Result<String, String> {
    let mut doc = read_existing_json(path);

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "type": "http",
        "url": url_for(params.port),
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
        fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = doc.as_table_mut().ok_or("TOML root is not a table")?;

    let mcp_servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));

    let mut entry = toml::map::Map::new();
    entry.insert("url".to_string(), toml::Value::String(url_for(params.port)));
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
    let mut doc = read_existing_json(path);

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "httpUrl": url_for(params.port),
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
    let mut doc = read_existing_json(path);

    let servers = doc
        .as_object_mut()
        .ok_or("JSON root is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let entry = serde_json::json!({
        "url": url_for(params.port),
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
                        "url": url_for(params.port),
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
                url_for(params.port)
            );
            Ok(content)
        }
        AgentKind::Gemini => {
            let doc = serde_json::json!({
                "mcpServers": {
                    "releash": {
                        "httpUrl": url_for(params.port),
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
                        "url": url_for(params.port),
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

// ── Tauri commands ──

#[tauri::command]
pub async fn generate_agent_mcp_config(
    agent_type: String,
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Result<GenerateResult, String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let config = app_config.get_config()?;
        let agent = AgentKind::from_str(&agent_type)?;
        let params = McpConfigParams {
            port: config.server.mcp_port,
            token: config.server.mcp_token.clone(),
        };
        generate_config(agent, &params)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn preview_agent_mcp_config(
    agent_type: String,
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Result<String, String> {
    let config = state.get_config()?;
    let agent = AgentKind::from_str(&agent_type)?;
    let params = McpConfigParams {
        port: config.server.mcp_port,
        token: config.server.mcp_token.clone(),
    };
    preview_config(agent, &params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> McpConfigParams {
        McpConfigParams {
            port: 19801,
            token: "test-token-abc123".to_string(),
        }
    }

    #[test]
    fn agent_kind_from_str_valid() {
        assert_eq!(AgentKind::from_str("claude").unwrap(), AgentKind::Claude);
        assert_eq!(AgentKind::from_str("codex").unwrap(), AgentKind::Codex);
        assert_eq!(AgentKind::from_str("gemini").unwrap(), AgentKind::Gemini);
        assert_eq!(AgentKind::from_str("cursor").unwrap(), AgentKind::Cursor);
    }

    #[test]
    fn agent_kind_from_str_invalid() {
        assert!(AgentKind::from_str("unknown").is_err());
    }

    #[test]
    fn generate_claude_writes_global_config() {
        let params = test_params();
        let result = generate_config(AgentKind::Claude, &params).unwrap();

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
        let params = test_params();
        let result = generate_config(AgentKind::Codex, &params).unwrap();

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
        let params = test_params();
        let result = generate_config(AgentKind::Gemini, &params).unwrap();

        assert!(result.file_path.ends_with(".gemini/settings.json"));
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["releash"]["httpUrl"],
            "http://127.0.0.1:19801/mcp"
        );
    }

    #[test]
    fn generate_cursor_writes_global_config() {
        let params = test_params();
        let result = generate_config(AgentKind::Cursor, &params).unwrap();

        assert!(result.file_path.ends_with(".cursor/mcp.json"));
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["releash"]["url"],
            "http://127.0.0.1:19801/mcp"
        );
    }

    #[test]
    fn generate_merges_with_existing_json() {
        let home = dirs::home_dir().unwrap();
        let path = home.join(".claude.json");
        let had_existing = path.exists();
        let backup = if had_existing {
            Some(fs::read_to_string(&path).unwrap())
        } else {
            None
        };

        // Write test data with another server
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
        let result = generate_config(AgentKind::Claude, &params).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed["mcpServers"]["other-server"].is_object());
        assert!(parsed["mcpServers"]["releash"].is_object());

        // Restore original state
        match backup {
            Some(content) => fs::write(&path, content).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
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
}
