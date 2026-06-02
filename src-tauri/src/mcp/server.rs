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
    Io(std::io::Error),
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

impl From<std::io::Error> for WorktreeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<WorktreeError> for McpError {
    fn from(e: WorktreeError) -> Self {
        match e {
            WorktreeError::JoinError(e) => McpError::internal_error(e.to_string(), None),
            WorktreeError::Git(e) => McpError::internal_error(e.to_string(), None),
            WorktreeError::Serialize(e) => McpError::internal_error(e.to_string(), None),
            WorktreeError::Io(e) => McpError::internal_error(e.to_string(), None),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorktreesListParams {}

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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadFileParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// File path relative to repository root
    pub file_path: String,
    /// Git ref to read the file at (e.g. "HEAD", "main"). Reads working copy if omitted.
    pub git_ref: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ReleashMcpServer {
    state: Arc<McpSharedState>,
    #[allow(dead_code)]
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

    fn resolve_worktree(&self, requested: &str) -> Result<String, McpError> {
        let repo_paths = self.state.repo_paths.read();
        if repo_paths.is_empty() {
            return Err(McpError::invalid_params("No repositories configured", None));
        }

        let usecase = &self.state.repository_usecase;
        for repo_path in repo_paths.iter() {
            let worktrees = usecase.list_worktrees(repo_path).map_err(|e| {
                McpError::internal_error(format!("Failed to list worktrees: {e}"), None)
            })?;
            if worktrees.iter().any(|w| w.path == requested) {
                return Ok(requested.to_string());
            }
        }

        Err(McpError::invalid_params(
            format!(
                "Worktree not found: {requested}. Use worktrees_list to get available worktrees."
            ),
            None,
        ))
    }

    #[tool(
        description = "List all git worktrees across configured repositories. Returns path, branch, dirty_count etc. Use the path value as the 'worktree' parameter for other tools."
    )]
    async fn worktrees_list(
        &self,
        Parameters(_params): Parameters<WorktreesListParams>,
    ) -> Result<CallToolResult, McpError> {
        let repo_paths = self.state.repo_paths.read().clone();
        if repo_paths.is_empty() {
            return Err(McpError::invalid_params("No repositories configured", None));
        }

        let usecase = Arc::clone(&self.state.repository_usecase);
        let entries = tokio::task::spawn_blocking(move || {
            let mut all = Vec::new();
            for repo_path in &repo_paths {
                if let Ok(entries) = usecase.list_worktrees(repo_path) {
                    all.extend(entries);
                }
            }
            all
        })
        .await
        .map_err(WorktreeError::from)?;

        let json = serde_json::to_string_pretty(&entries).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Create a new git worktree and branch")]
    async fn create_workspace(
        &self,
        Parameters(params): Parameters<CreateWorkspaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let repo = {
            let configured = self.state.repo_paths.read();
            if configured.iter().any(|p| p == &params.repo_path) {
                params.repo_path.clone()
            } else {
                return Err(McpError::invalid_params(
                    "repo_path is not in configured repositories",
                    None,
                ));
            }
        };

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

        let usecase = Arc::clone(&self.state.repository_usecase);
        let entry = tokio::task::spawn_blocking(move || {
            usecase.create_worktree(&repo, &wt_path, &branch, true, base.as_deref())
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&entry).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // -----------------------------------------------------------------------
    // File read tool
    // -----------------------------------------------------------------------

    #[tool(
        description = "Read a file from a worktree. Optionally specify a git_ref (e.g. 'HEAD', 'main') to read the file at that revision instead of the working copy."
    )]
    async fn read_file(
        &self,
        Parameters(params): Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;
        let file_path = params.file_path.clone();
        let git_ref = params.git_ref.clone();

        let (content, line_count) = tokio::task::spawn_blocking(move || {
            crate::ws_server::validation::validate_relative_path(&file_path, &worktree_path)
                .map_err(crate::git::error::GitError::Custom)?;
            let full_path = std::path::Path::new(&worktree_path).join(&file_path);
            let full_path_str = full_path.to_string_lossy().to_string();

            let content = if let Some(ref git_ref) = git_ref {
                crate::git::diff::get_file_at_ref(full_path_str, git_ref.clone())?
            } else {
                std::fs::read_to_string(&full_path).map_err(crate::git::error::GitError::Io)?
            };

            let line_count = content.lines().count();

            Ok::<_, crate::git::error::GitError>((content, line_count))
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(WorktreeError::from)?;

        let response = serde_json::json!({
            "file_path": params.file_path,
            "line_count": line_count,
            "content": content,
        });

        let json = serde_json::to_string_pretty(&response).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for ReleashMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        info.protocol_version = ProtocolVersion::V_2024_11_05;
        info.server_info = Implementation::new("releash-mcp", env!("CARGO_PKG_VERSION"));
        info.instructions =
            Some("Releash MCP Server: Git worktree management for AI coding agents.".to_string());
        info
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

        let state = McpSharedState {
            repo_paths: Arc::new(parking_lot::RwLock::new(repo_paths)),
            pty_manager,
            app_config,
            broadcaster,
            app_data_dir: None,
            repository_usecase: Arc::new(
                crate::adaptor::controller::wiring::build_repository_usecase(),
            ),
        };
        ReleashMcpServer::new(state)
    }

    #[test]
    fn resolve_worktree_validates_against_real_worktrees() {
        use crate::git::test_helpers::{create_initial_commit, create_test_repo};

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let server = make_server(vec![repo_path.clone()]);
        let result = server.resolve_worktree(&repo_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), repo_path);
    }

    #[test]
    fn resolve_worktree_rejects_unknown_path() {
        use crate::git::test_helpers::{create_initial_commit, create_test_repo};

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let server = make_server(vec![repo_path]);
        let result = server.resolve_worktree("/tmp/nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_worktree_returns_error_when_no_repos_configured() {
        let server = make_server(vec![]);
        let result = server.resolve_worktree("/tmp/any");
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
