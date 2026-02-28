use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

use tauri::Emitter;

use crate::protocol::CommentItem;

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

// ---------------------------------------------------------------------------
// Review tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PostReviewCommentParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// File path relative to repository root
    pub file_path: String,
    /// Line number
    pub line_number: u32,
    /// End line (for range comments)
    pub end_line: Option<u32>,
    /// Comment content
    pub content: String,
    /// Severity: "info", "warning", "error", "suggestion"
    pub severity: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetReviewCommentsParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// Filter by file path
    pub file_path: Option<String>,
    /// Filter by severity
    pub severity: Option<String>,
    /// Filter by resolved status
    pub resolved: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResolveCommentParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// Comment ID to resolve/unresolve
    pub comment_id: String,
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

    fn resolve_worktree(&self, requested: &str) -> Result<String, McpError> {
        let repo_paths = self.state.repo_paths.read();
        if repo_paths.is_empty() {
            return Err(McpError::invalid_params("No repositories configured", None));
        }

        for repo_path in repo_paths.iter() {
            let worktrees =
                crate::git::worktree::list_worktrees(repo_path.clone()).unwrap_or_default();
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

        let entries = tokio::task::spawn_blocking(move || {
            let mut all = Vec::new();
            for repo_path in &repo_paths {
                if let Ok(entries) = crate::git::worktree::list_worktrees(repo_path.clone()) {
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
        // create_workspace still uses repo_path since it needs a base repository
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

        let entry = tokio::task::spawn_blocking(move || {
            crate::git::worktree::create_worktree(repo, wt_path, branch, true, base)
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(WorktreeError::from)?;

        let json = serde_json::to_string_pretty(&entry).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // -----------------------------------------------------------------------
    // Review tools
    // -----------------------------------------------------------------------

    #[tool(
        description = "Post a review comment on a specific file and line. The comment is stored and broadcast to the UI."
    )]
    async fn post_review_comment(
        &self,
        Parameters(params): Parameters<PostReviewCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let comment_id = uuid::Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let comment = CommentItem {
            id: comment_id.clone(),
            file_path: params.file_path.clone(),
            line_number: params.line_number,
            end_line: params.end_line,
            content: params.content.clone(),
            status: "unsent".to_string(),
            created_at: now,
            parent_id: None,
            severity: params.severity.clone(),
            resolved: false,
            target: "review".to_string(),
        };

        self.state
            .comment_store
            .add(&worktree_path, comment.clone());

        // Persist and notify desktop UI
        if let (Some(app), Some(data_dir)) = (
            self.state.app_handle.as_ref(),
            self.state.app_data_dir.as_ref(),
        ) {
            let _ = self.state.comment_store.save(data_dir, &worktree_path);
            let _ = app.emit(
                "comments-changed",
                crate::comment_store::CommentsChangedPayload {
                    worktree_name: worktree_path.clone(),
                    source: "mcp".to_string(),
                },
            );
        }

        // Broadcast via WebSocket
        self.state
            .broadcaster
            .try_send(crate::protocol::WsMessage::AddComment(
                crate::protocol::AddComment {
                    file_path: params.file_path,
                    line_number: params.line_number,
                    end_line: params.end_line,
                    content: params.content,
                    severity: params.severity,
                    target: "review".to_string(),
                },
            ));

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Comment posted: {comment_id}"
        ))]))
    }

    #[tool(
        description = "Get review comments, optionally filtered by file_path, severity, or resolved status."
    )]
    async fn get_review_comments(
        &self,
        Parameters(params): Parameters<GetReviewCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let comments = self.state.comment_store.get_filtered(
            &worktree_path,
            params.file_path.as_deref(),
            params.severity.as_deref(),
            params.resolved,
        );

        let json = serde_json::to_string_pretty(&comments).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Toggle the resolved status of a review comment by its ID.")]
    async fn resolve_comment(
        &self,
        Parameters(params): Parameters<ResolveCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let resolved = self
            .state
            .comment_store
            .resolve(&worktree_path, &params.comment_id)
            .ok_or_else(|| {
                McpError::invalid_params(format!("Comment not found: {}", params.comment_id), None)
            })?;

        // Persist and notify desktop UI
        if let (Some(app), Some(data_dir)) = (
            self.state.app_handle.as_ref(),
            self.state.app_data_dir.as_ref(),
        ) {
            let _ = self.state.comment_store.save(data_dir, &worktree_path);
            let _ = app.emit(
                "comments-changed",
                crate::comment_store::CommentsChangedPayload {
                    worktree_name: worktree_path.clone(),
                    source: "mcp".to_string(),
                },
            );
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Comment {} {}",
            params.comment_id,
            if resolved { "resolved" } else { "unresolved" }
        ))]))
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
        let comment_store = Arc::new(crate::comment_store::CommentStore::default());

        let state = McpSharedState {
            repo_paths: Arc::new(parking_lot::RwLock::new(repo_paths)),
            pty_manager,
            app_config,
            broadcaster,
            agent_states,
            comment_store,
            app_handle: None,
            app_data_dir: None,
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
        // The main worktree path matches the repo_path
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
