use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

use crate::adaptor::gateway::mcp::state::McpSharedState;

// ---------------------------------------------------------------------------
// Module-specific error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum WorktreeError {
    JoinError(tokio::task::JoinError),
    Code(crate::domain::code::CodeError),
    Serialize(serde_json::Error),
    Io(std::io::Error),
}

impl From<tokio::task::JoinError> for WorktreeError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::JoinError(e)
    }
}

impl From<crate::domain::code::CodeError> for WorktreeError {
    fn from(e: crate::domain::code::CodeError) -> Self {
        Self::Code(e)
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
            WorktreeError::Code(e) => McpError::internal_error(e.to_string(), None),
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
        let code_usecase = Arc::clone(&self.state.code_usecase);

        let (content, line_count) = tokio::task::spawn_blocking(move || {
            crate::ws_server::validation::validate_relative_path(&file_path, &worktree_path)
                .map_err(crate::domain::code::CodeError::Rule)?;
            let full_path = std::path::Path::new(&worktree_path).join(&file_path);
            let full_path_str = full_path.to_string_lossy().to_string();

            // 周辺入口（MCP）は gateway 実装やファイル I/O へ直接依存せず、code usecase の
            // 公開 API 経由でファイル内容参照を行う。CodeUsecaseError は内包する CodeError を
            // 取り出し、既存のエラー表現（文字列）を等価に保つ。
            let content = if let Some(ref git_ref) = git_ref {
                code_usecase
                    .get_file_at_ref(&full_path_str, git_ref)
                    .map_err(|crate::usecase::code_error::CodeUsecaseError::Code(c)| c)?
            } else {
                code_usecase
                    .get_file_in_worktree(&full_path_str)
                    .map_err(|crate::usecase::code_error::CodeUsecaseError::Code(c)| c)?
            };

            let line_count = content.lines().count();

            Ok::<_, crate::domain::code::CodeError>((content, line_count))
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
        use crate::adaptor::gateway::mcp::state::McpSharedState;

        let app_config: Arc<dyn crate::domain::app_config::ConfigRepository> =
            Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
                crate::adaptor::gateway::app_config::ReleashConfig::default(),
                PathBuf::from("/tmp/test-config.toml"),
            ));
        let pty_session_runtime_gateway = Arc::new(
            crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway::default(),
        );
        let broadcaster = Arc::new(crate::ws_bridge::WsBroadcaster::default());

        let state = McpSharedState {
            repo_paths: Arc::new(parking_lot::RwLock::new(repo_paths)),
            pty_session_runtime_gateway,
            app_config,
            broadcaster,
            app_data_dir: None,
            repository_usecase: Arc::new(
                crate::adaptor::controller::wiring::build_repository_usecase(),
            ),
            code_usecase: Arc::new(crate::adaptor::controller::wiring::build_code_usecase()),
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

    // ── read_file tool（MCP アクター経路のファイル内容参照が移行前後で等価） ──
    // read_file は CodeUsecase 経由でファイル内容を返す。behavior.md「ファイル内容の
    // 参照結果は移行前後で等価」「リモートクライアントから見た振る舞いも等価」を担保する。

    /// `CallToolResult` の text content から、read_file が返す内部レスポンス JSON を取り出す。
    fn read_file_response(result: &CallToolResult) -> serde_json::Value {
        let v = serde_json::to_value(result).expect("CallToolResult should serialize");
        let text = v["content"][0]["text"]
            .as_str()
            .expect("text content should be present");
        serde_json::from_str(text).expect("inner response should be json")
    }

    #[tokio::test]
    async fn read_file_通常ファイルを作業ツリーから読む() {
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
        std::fs::write(
            std::path::Path::new(&repo_path).join("hello.txt"),
            "working tree\n",
        )
        .unwrap();

        let server = make_server(vec![repo_path.clone()]);
        let params = ReadFileParams {
            worktree: repo_path,
            file_path: "hello.txt".to_string(),
            git_ref: None,
        };
        let result = server.read_file(Parameters(params)).await.unwrap();
        let body = read_file_response(&result);
        assert_eq!(body["content"], serde_json::json!("working tree\n"));
        assert_eq!(body["line_count"], serde_json::json!(1));
        assert_eq!(body["file_path"], serde_json::json!("hello.txt"));
    }

    #[tokio::test]
    async fn read_file_git_ref指定でコミット時点の内容を読む() {
        use crate::git::test_helpers::{add_and_commit, create_initial_commit, create_test_repo};

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(
            &repo,
            "committed.txt",
            "committed content\n",
            "add committed",
        );
        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        // 作業ツリーを変更しても git_ref=HEAD はコミット時点の内容を返す。
        std::fs::write(
            std::path::Path::new(&repo_path).join("committed.txt"),
            "modified working copy\n",
        )
        .unwrap();

        let server = make_server(vec![repo_path.clone()]);
        let params = ReadFileParams {
            worktree: repo_path,
            file_path: "committed.txt".to_string(),
            git_ref: Some("HEAD".to_string()),
        };
        let result = server.read_file(Parameters(params)).await.unwrap();
        let body = read_file_response(&result);
        assert_eq!(body["content"], serde_json::json!("committed content\n"));
    }

    #[tokio::test]
    async fn read_file_不正な相対パスを拒否する() {
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
        let params = ReadFileParams {
            worktree: repo_path,
            file_path: "../outside.txt".to_string(),
            git_ref: None,
        };
        let result = server.read_file(Parameters(params)).await;
        assert!(result.is_err(), "path traversal should be rejected");
    }
}
