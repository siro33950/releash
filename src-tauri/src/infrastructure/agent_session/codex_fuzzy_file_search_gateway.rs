use std::collections::HashMap;

use serde_json::Value;

use crate::domain::code::CodeError;
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_fuzzy_file_search_request, CodexAppServerProcess,
};
use crate::usecase::code_query_service::CodexFuzzyFileSearchGateway;

pub(crate) struct TauriCodexFuzzyFileSearchGateway<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriCodexFuzzyFileSearchGateway<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime + 'static> CodexFuzzyFileSearchGateway
    for TauriCodexFuzzyFileSearchGateway<R>
{
    async fn search_files(
        &self,
        worktree_path: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, CodeError> {
        let cli_path = configured_cli_path(&self.app).unwrap_or_else(|| "codex".to_string());
        let mut process = CodexAppServerProcess::spawn(&self.app, &cli_path)
            .await
            .map_err(CodeError::External)?;
        let result = async {
            process
                .initialize(env!("CARGO_PKG_VERSION"))
                .await
                .map_err(CodeError::External)?;
            let id = process.next_request_id();
            process
                .send(&build_fuzzy_file_search_request(id, worktree_path, query))
                .await
                .map_err(CodeError::External)?;
            let response = process
                .read_response_result(id)
                .await
                .map_err(CodeError::External)?;
            Ok(codex_fuzzy_file_paths(&response, worktree_path, limit))
        }
        .await;
        process.shutdown().await;
        result
    }
}

fn normalize_codex_fuzzy_path(root: &str, path: &str) -> Option<String> {
    let root = root.trim_end_matches(['/', '\\']);
    let mut path = path.trim().replace('\\', "/");
    if path.is_empty() {
        return None;
    }
    let normalized_root = root.replace('\\', "/");
    if !normalized_root.is_empty() && path == normalized_root {
        return None;
    }
    if !normalized_root.is_empty() {
        if let Some(stripped) = path.strip_prefix(&(normalized_root.clone() + "/")) {
            path = stripped.to_string();
        }
    }
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

fn codex_fuzzy_file_paths(response: &Value, root: &str, limit: usize) -> Vec<String> {
    let mut seen = HashMap::<String, ()>::new();
    let mut paths = Vec::new();
    let Some(files) = response.get("files").and_then(Value::as_array) else {
        return paths;
    };
    for item in files {
        let match_type = item
            .get("match_type")
            .or_else(|| item.get("matchType"))
            .and_then(Value::as_str);
        if match_type.is_none_or(|value| value != "file" && value != "directory") {
            continue;
        }
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(mut path) = normalize_codex_fuzzy_path(root, path) else {
            continue;
        };
        if match_type.is_some_and(|value| value == "directory") {
            path = format!("{}/", path.trim_end_matches('/'));
        }
        if seen.insert(path.clone(), ()).is_some() {
            continue;
        }
        paths.push(path);
        if paths.len() >= limit {
            break;
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_fuzzy_file_paths_keeps_file_and_directory_matches_in_runtime_order() {
        let result = codex_fuzzy_file_paths(
            &serde_json::json!({
                "files": [
                    { "root": "/repo", "path": "/repo/src/main.rs", "match_type": "file" },
                    { "root": "/repo", "path": "src", "match_type": "directory" },
                    { "root": "/repo", "path": "src/main.rs", "match_type": "file" },
                    { "root": "/repo", "path": "src/lib.rs", "matchType": "file" }
                ]
            }),
            "/repo",
            50,
        );

        assert_eq!(result, vec!["src/main.rs", "src/", "src/lib.rs"]);
    }

    #[test]
    fn codex_fuzzy_file_paths_respects_limit() {
        let result = codex_fuzzy_file_paths(
            &serde_json::json!({
                "files": [
                    { "path": "a.rs", "match_type": "file" },
                    { "path": "b.rs", "match_type": "file" }
                ]
            }),
            "/repo",
            1,
        );

        assert_eq!(result, vec!["a.rs"]);
    }
}
