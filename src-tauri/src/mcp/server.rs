use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

use super::state::McpSharedState;

// ---------------------------------------------------------------------------
// Module-specific error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum WorktreeError {
    JoinError(tokio::task::JoinError),
    Git(crate::git::error::GitError),
    Serialize(serde_json::Error),
}

impl From<tokio::task::JoinError> for WorktreeError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::JoinError(e)
    }
}

impl From<crate::git::error::GitError> for WorktreeError {
    fn from(e: crate::git::error::GitError) -> Self {
        Self::Git(e)
    }
}

impl From<serde_json::Error> for WorktreeError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialize(e)
    }
}

impl From<WorktreeError> for McpError {
    fn from(e: WorktreeError) -> Self {
        match e {
            WorktreeError::JoinError(e) => McpError::internal_error(e.to_string(), None),
            WorktreeError::Git(e) => McpError::internal_error(e.to_string(), None),
            WorktreeError::Serialize(e) => McpError::internal_error(e.to_string(), None),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorktreesListParams {
    /// Repository path (uses first configured repo if omitted)
    pub repo_path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateWorkspaceParams {
    /// Branch name to create
    pub branch: String,
    /// Base branch (defaults to HEAD)
    pub base_branch: Option<String>,
    /// Repository path
    pub repo_path: String,
    /// Worktree path (auto-generated from branch name if omitted)
    pub worktree_path: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ReleashMcpServer {
    state: Arc<McpSharedState>,
    tool_router: ToolRouter<ReleashMcpServer>,
}

#[tool_router]
impl ReleashMcpServer {
    pub fn new(state: McpSharedState) -> Self {
        Self {
            state: Arc::new(state),
            tool_router: Self::tool_router(),
        }
    }

    fn resolve_allowed_repo_path(&self, requested: Option<String>) -> Result<String, McpError> {
        let configured = self.state.repo_paths.read();
        let candidate = requested
            .or_else(|| configured.first().cloned())
            .ok_or_else(|| McpError::invalid_params("repo_path is required", None))?;

        if configured.iter().any(|p| p == &candidate) {
            Ok(candidate)
        } else {
            Err(McpError::invalid_params(
                "repo_path is not in configured repositories",
                None,
            ))
        }
    }

    #[tool(description = "List all git worktrees for a repository")]
    async fn worktrees_list(
        &self,
        Parameters(params): Parameters<WorktreesListParams>,
    ) -> Result<CallToolResult, McpError> {
        let repo_path = self.resolve_allowed_repo_path(params.repo_path)?;

        let entries =
            tokio::task::spawn_blocking(move || crate::git::worktree::list_worktrees(repo_path))
                .await
                .map_err(WorktreeError::from)?
                .map_err(WorktreeError::from)?;

        let json = serde_json::to_string_pretty(&entries).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Create a new git worktree and branch")]
    async fn create_workspace(
        &self,
        Parameters(params): Parameters<CreateWorkspaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let repo = self.resolve_allowed_repo_path(Some(params.repo_path.clone()))?;

        let worktree_path = if let Some(path) = params.worktree_path {
            path
        } else {
            let repo_path = std::path::Path::new(&repo);
            let parent = repo_path
                .parent()
                .ok_or_else(|| McpError::invalid_params("Cannot determine parent dir", None))?;
            let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
            let sanitized = params.branch.replace('/', "-");
            parent
                .join(format!("{repo_name}-worktrees"))
                .join(&sanitized)
                .to_string_lossy()
                .to_string()
        };

        let wt_path = worktree_path;
        let branch = params.branch.clone();
        let base = params.base_branch.clone();

        let entry = tokio::task::spawn_blocking(move || {
            crate::git::worktree::create_worktree(repo, wt_path, branch, true, base)
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(WorktreeError::from)?;

        let json = serde_json::to_string_pretty(&entry).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for ReleashMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "releash-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            },
            instructions: Some(
                "Releash MCP Server: Git worktree management for AI coding agents.".to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_server(repo_paths: Vec<String>) -> ReleashMcpServer {
        use crate::mcp::state::McpSharedState;

        let app_config = Arc::new(crate::config::AppConfig::new(
            crate::config::ReleashConfig::default(),
            PathBuf::from("/tmp/test-config.toml"),
        ));
        let pty_manager = Arc::new(crate::pty::PtyManager::default());
        let broadcaster = Arc::new(crate::ws_bridge::WsBroadcaster::default());
        let agent_states: crate::hook_listener::AgentStatesMap =
            Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));

        let state = McpSharedState {
            repo_paths: Arc::new(parking_lot::RwLock::new(repo_paths)),
            pty_manager,
            app_config,
            broadcaster,
            agent_states,
        };
        ReleashMcpServer::new(state)
    }

    #[test]
    fn resolve_repo_path_uses_first_configured_when_none() {
        let server = make_server(vec!["/tmp/repo1".to_string(), "/tmp/repo2".to_string()]);
        let result = server.resolve_allowed_repo_path(None).unwrap();
        assert_eq!(result, "/tmp/repo1");
    }

    #[test]
    fn resolve_repo_path_accepts_configured_path() {
        let server = make_server(vec!["/tmp/repo1".to_string(), "/tmp/repo2".to_string()]);
        let result = server
            .resolve_allowed_repo_path(Some("/tmp/repo2".to_string()))
            .unwrap();
        assert_eq!(result, "/tmp/repo2");
    }

    #[test]
    fn resolve_repo_path_rejects_unconfigured_path() {
        let server = make_server(vec!["/tmp/repo1".to_string()]);
        let result = server.resolve_allowed_repo_path(Some("/tmp/evil".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_repo_path_returns_error_when_empty_and_no_configured() {
        let server = make_server(vec![]);
        let result = server.resolve_allowed_repo_path(None);
        assert!(result.is_err());
    }

    #[test]
    fn worktree_error_converts_to_mcp_error() {
        let err = WorktreeError::Serialize(
            serde_json::from_str::<serde_json::Value>("invalid").unwrap_err(),
        );
        let mcp_err: McpError = err.into();
        assert!(!mcp_err.message.is_empty());
    }
}
