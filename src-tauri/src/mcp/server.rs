use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

use tauri::{Emitter, Manager};

use crate::protocol::thread::{Thread, ThreadEntry};

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

const VALID_SEVERITIES: &[&str] = &["info", "warning", "error", "suggestion"];
const VALID_FILTER_MODES: &[&str] = &["diff_context", "file", "nofilter"];

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
// Thread tool parameter types (Phase D-1)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetThreadParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// Thread ID
    pub thread_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListThreadsParams {
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
pub struct AddThreadEntryParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// Thread ID to add entry to
    pub thread_id: String,
    /// Entry content (message text)
    pub content: String,
    /// Whether the entry is from AI
    pub is_ai: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReviewDiffParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// Base branch to diff against (auto-detected from config if omitted)
    pub base_branch: Option<String>,
    /// File paths to include full diff for. If omitted, returns summary only (file list + stats, no hunks).
    pub paths: Option<Vec<String>>,
    /// Maximum diff lines (additions + deletions) per file. Files exceeding this limit return stats only (hunks omitted, truncated=true). Use read_file to review truncated files. Defaults to 500.
    pub max_lines_per_file: Option<u32>,
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
// LSP tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckDiagnosticsParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// File paths relative to repository root. If omitted, checks all changed files (for "file"/"diff_context" modes) or all files (for "nofilter" mode).
    pub file_paths: Option<Vec<String>>,
    /// Filter mode: "diff_context" (only diagnostics within diff hunk ranges), "file" (all diagnostics for changed files), "nofilter" (all diagnostics for all files). Defaults to "file".
    pub filter_mode: Option<String>,
    /// Base branch for diff_context mode (auto-detected if omitted)
    pub base_branch: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetFileSymbolsParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// File path relative to repository root
    pub file_path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExploreSymbolParams {
    /// Worktree path (from worktrees_list)
    pub worktree: String,
    /// File path relative to repository root
    pub file_path: String,
    /// Line number (1-based)
    pub line: u32,
    /// Column number (1-based)
    pub column: u32,
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
                crate::git::worktree::list_worktrees(repo_path.clone()).map_err(|e| {
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
        description = "Post a review comment on a specific file and line. The comment is stored as a thread and broadcast to the UI."
    )]
    async fn post_review_comment(
        &self,
        Parameters(params): Parameters<PostReviewCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        if let Some(ref s) = params.severity {
            if !VALID_SEVERITIES.contains(&s.as_str()) {
                return Err(McpError::invalid_params(
                    format!(
                        "Invalid severity: {s}. Must be one of: info, warning, error, suggestion"
                    ),
                    None,
                ));
            }
        }

        let thread_id = uuid::Uuid::new_v4().to_string();
        let entry_id = uuid::Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        let thread = Thread {
            id: thread_id.clone(),
            file_path: params.file_path.clone(),
            line_number: params.line_number,
            end_line: params.end_line,
            entries: vec![ThreadEntry {
                id: entry_id,
                content: params.content.clone(),
                is_ai: true,
                action: None,
                author_name: Some("AI Review".to_string()),
                author_avatar_url: None,
                pr_comment_id: None,
                created_at: now,
            }],
            resolved: false,
            severity: params.severity.clone(),
            anchor: None,
            created_at: now,
        };

        self.state.thread_store.add_thread(&worktree_path, thread);

        self.persist_and_emit_threads_changed(&worktree_path)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Comment posted: {thread_id}"
        ))]))
    }

    #[tool(
        description = "Deprecated: use list_threads instead. Get review comments, optionally filtered by file_path, severity, or resolved status."
    )]
    async fn get_review_comments(
        &self,
        Parameters(params): Parameters<GetReviewCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let threads = self.state.thread_store.get_filtered(
            &worktree_path,
            params.file_path.as_deref(),
            params.severity.as_deref(),
            params.resolved,
        );

        // Return threads serialized as JSON
        let json = serde_json::to_string_pretty(&threads).map_err(WorktreeError::from)?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Toggle the resolved status of a review comment (thread) by its ID.")]
    async fn resolve_comment(
        &self,
        Parameters(params): Parameters<ResolveCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let resolved = self
            .state
            .thread_store
            .resolve_thread(&worktree_path, &params.comment_id)
            .ok_or_else(|| {
                McpError::invalid_params(format!("Thread not found: {}", params.comment_id), None)
            })?;

        self.persist_and_emit_threads_changed(&worktree_path)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Thread {} {}",
            params.comment_id,
            if resolved { "resolved" } else { "unresolved" }
        ))]))
    }

    // -----------------------------------------------------------------------
    // Thread tools (Phase D-1)
    // -----------------------------------------------------------------------

    #[tool(
        description = "Get a thread by its ID. Returns the full thread with all entries (messages)."
    )]
    async fn get_thread(
        &self,
        Parameters(params): Parameters<GetThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let thread = self
            .state
            .thread_store
            .get_thread(&worktree_path, &params.thread_id)
            .ok_or_else(|| {
                McpError::invalid_params(format!("Thread not found: {}", params.thread_id), None)
            })?;

        let json = serde_json::to_string_pretty(&thread).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "List threads in a worktree, optionally filtered by file_path, severity, or resolved status."
    )]
    async fn list_threads(
        &self,
        Parameters(params): Parameters<ListThreadsParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let threads = self.state.thread_store.get_filtered(
            &worktree_path,
            params.file_path.as_deref(),
            params.severity.as_deref(),
            params.resolved,
        );

        let json = serde_json::to_string_pretty(&threads).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Add an entry (message) to an existing thread. Use is_ai=true for AI-generated responses."
    )]
    async fn add_thread_entry(
        &self,
        Parameters(params): Parameters<AddThreadEntryParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;

        let entry_id = uuid::Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        let entry = ThreadEntry {
            id: entry_id.clone(),
            content: params.content.clone(),
            is_ai: params.is_ai,
            action: None,
            author_name: if params.is_ai {
                Some("AI".to_string())
            } else {
                None
            },
            author_avatar_url: None,
            pr_comment_id: None,
            created_at: now,
        };

        let added = self
            .state
            .thread_store
            .add_entry(&worktree_path, &params.thread_id, entry);

        if !added {
            return Err(McpError::invalid_params(
                format!("Thread not found: {}", params.thread_id),
                None,
            ));
        }

        self.persist_and_emit_threads_changed(&worktree_path)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Entry added: {entry_id}"
        ))]))
    }

    // -----------------------------------------------------------------------
    // Code review data tools
    // -----------------------------------------------------------------------

    #[tool(
        description = "Get the diff of a worktree compared to its base branch. Two modes: (1) Summary mode (paths omitted) — returns file list with per-file stats (additions/deletions), no hunks. (2) Detail mode (paths specified) — returns full diff with hunks for the specified files only. Files exceeding max_lines_per_file (default: 500) return stats only with truncated=true; use read_file to review those files."
    )]
    async fn review_diff(
        &self,
        Parameters(params): Parameters<ReviewDiffParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;
        let base = params.base_branch.clone();
        let paths = params.paths.clone();
        let max_lines = params.max_lines_per_file.unwrap_or(500) as usize;

        let review_diff = tokio::task::spawn_blocking(move || {
            crate::git::review::get_review_diff(
                &worktree_path,
                base.as_deref(),
                paths.as_deref(),
                Some(max_lines),
            )
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(WorktreeError::from)?;

        let json = serde_json::to_string_pretty(&review_diff).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

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

        let (content, language, line_count) = tokio::task::spawn_blocking(move || {
            crate::ws_server::validation::validate_relative_path(&file_path, &worktree_path)
                .map_err(crate::git::error::GitError::Custom)?;
            let full_path = std::path::Path::new(&worktree_path).join(&file_path);
            let full_path_str = full_path.to_string_lossy().to_string();

            let content = if let Some(ref git_ref) = git_ref {
                crate::git::diff::get_file_at_ref(full_path_str, git_ref.clone())?
            } else {
                std::fs::read_to_string(&full_path).map_err(crate::git::error::GitError::Io)?
            };

            let ext = std::path::Path::new(&file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let language = crate::lsp::detect::language_for_extension(ext).map(String::from);
            let line_count = content.lines().count();

            Ok::<_, crate::git::error::GitError>((content, language, line_count))
        })
        .await
        .map_err(WorktreeError::from)?
        .map_err(WorktreeError::from)?;

        let response = serde_json::json!({
            "file_path": params.file_path,
            "language": language,
            "line_count": line_count,
            "content": content,
        });

        let json = serde_json::to_string_pretty(&response).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // -----------------------------------------------------------------------
    // LSP-based tools
    // -----------------------------------------------------------------------

    #[tool(
        description = "Check diagnostics (errors, warnings) for files in a worktree using LSP. Three filter modes: 'diff_context' — only diagnostics within diff hunk ranges (changed lines); 'file' (default) — all diagnostics for changed files; 'nofilter' — all diagnostics for all files in the worktree."
    )]
    async fn check_diagnostics(
        &self,
        Parameters(params): Parameters<CheckDiagnosticsParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;
        let (lsp, app) = self.get_lsp_and_app()?;

        let filter_mode = params.filter_mode.as_deref().unwrap_or("file");
        if !VALID_FILTER_MODES.contains(&filter_mode) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid filter_mode: {filter_mode}. Must be one of: diff_context, file, nofilter"
                ),
                None,
            ));
        }

        let file_paths = if let Some(paths) = params.file_paths {
            paths
        } else if filter_mode == "nofilter" {
            let wt = worktree_path.clone();
            tokio::task::spawn_blocking(move || collect_all_files(&wt))
                .await
                .map_err(WorktreeError::from)?
        } else {
            // "file" or "diff_context": use changed files from git status
            let wt = worktree_path.clone();
            let statuses =
                tokio::task::spawn_blocking(move || crate::git::status::get_git_status(wt))
                    .await
                    .map_err(WorktreeError::from)?
                    .map_err(WorktreeError::from)?;

            statuses
                .into_iter()
                .filter(|s| {
                    s.worktree_status != "ignored"
                        && s.worktree_status != "deleted"
                        && s.index_status != "deleted"
                })
                .map(|s| s.path)
                .collect()
        };

        // For diff_context mode, get hunk ranges to filter diagnostics
        let hunk_ranges = if filter_mode == "diff_context" {
            let wt = worktree_path.clone();
            let base = params.base_branch.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    crate::git::review::get_hunk_ranges(&wt, base.as_deref())
                })
                .await
                .map_err(WorktreeError::from)?
                .map_err(WorktreeError::from)?,
            )
        } else {
            None
        };

        let mut results = Vec::new();
        let mut total_errors = 0usize;
        let mut total_warnings = 0usize;

        for file_path in &file_paths {
            let ext = std::path::Path::new(file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let server_lang = match crate::lsp::detect::language_for_extension(ext) {
                Some(lang) => lang,
                None => continue,
            };
            let language_id = crate::lsp::detect::lsp_language_id(ext).unwrap_or(server_lang);

            let session_id = match lsp.ensure_session(&app, &worktree_path, server_lang).await {
                Ok(id) => id,
                Err(e) => {
                    log::warn!("Failed to ensure LSP session for {server_lang}: {e}");
                    continue;
                }
            };

            let uri = self
                .did_open_file(&lsp, session_id, &worktree_path, file_path, language_id)
                .await?;

            let diagnostics = lsp
                .wait_for_diagnostics(session_id, &uri, 3000)
                .await
                .unwrap_or_default();

            // Filter diagnostics by hunk ranges in diff_context mode
            let diagnostics = if let Some(ref ranges_map) = hunk_ranges {
                if let Some(ranges) = ranges_map.get(file_path) {
                    diagnostics
                        .into_iter()
                        .filter(|diag| {
                            diag.get("range")
                                .and_then(|r| r.get("start"))
                                .and_then(|s| s.get("line"))
                                .and_then(|l| l.as_u64())
                                .map(|line_0based| {
                                    let line_1based = (line_0based as u32) + 1;
                                    crate::git::review::is_line_in_hunk_ranges(line_1based, ranges)
                                })
                                .unwrap_or(false)
                        })
                        .collect()
                } else {
                    // File not in diff — no diagnostics in diff_context mode
                    Vec::new()
                }
            } else {
                diagnostics
            };

            for diag in &diagnostics {
                match diag.get("severity").and_then(|v| v.as_u64()) {
                    Some(1) => total_errors += 1,
                    Some(2) => total_warnings += 1,
                    _ => {}
                }
            }

            if !diagnostics.is_empty() {
                results.push(serde_json::json!({
                    "path": file_path,
                    "diagnostics": diagnostics,
                }));
            }
        }

        let response = serde_json::json!({
            "files": results,
            "filter_mode": filter_mode,
            "summary": {
                "total_files": file_paths.len(),
                "files_with_diagnostics": results.len(),
                "total_errors": total_errors,
                "total_warnings": total_warnings,
            }
        });

        let json = serde_json::to_string_pretty(&response).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get document symbols (functions, classes, variables) from a file using LSP."
    )]
    async fn get_file_symbols(
        &self,
        Parameters(params): Parameters<GetFileSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;
        let (lsp, app) = self.get_lsp_and_app()?;

        let ext = std::path::Path::new(&params.file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let server_lang = crate::lsp::detect::language_for_extension(ext).ok_or_else(|| {
            McpError::invalid_params(format!("Unsupported language for extension: {ext}"), None)
        })?;
        let language_id = crate::lsp::detect::lsp_language_id(ext).unwrap_or(server_lang);

        let session_id = lsp
            .ensure_session(&app, &worktree_path, server_lang)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;

        let uri = self
            .did_open_file(
                &lsp,
                session_id,
                &worktree_path,
                &params.file_path,
                language_id,
            )
            .await?;

        let result = lsp
            .request(
                session_id,
                "textDocument/documentSymbol",
                serde_json::json!({
                    "textDocument": { "uri": uri }
                }),
                &worktree_path,
                10000,
            )
            .await
            .map_err(|e| McpError::internal_error(e, None))?;

        let json = serde_json::to_string_pretty(&result).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Explore a symbol at a specific position. Returns definition location, hover info, and references."
    )]
    async fn explore_symbol(
        &self,
        Parameters(params): Parameters<ExploreSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        let worktree_path = self.resolve_worktree(&params.worktree)?;
        let (lsp, app) = self.get_lsp_and_app()?;

        let ext = std::path::Path::new(&params.file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let server_lang = crate::lsp::detect::language_for_extension(ext).ok_or_else(|| {
            McpError::invalid_params(format!("Unsupported language for extension: {ext}"), None)
        })?;
        let language_id = crate::lsp::detect::lsp_language_id(ext).unwrap_or(server_lang);

        let session_id = lsp
            .ensure_session(&app, &worktree_path, server_lang)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;

        let uri = self
            .did_open_file(
                &lsp,
                session_id,
                &worktree_path,
                &params.file_path,
                language_id,
            )
            .await?;

        let position = serde_json::json!({
            "line": params.line.saturating_sub(1),
            "character": params.column.saturating_sub(1),
        });

        let text_doc = serde_json::json!({ "uri": &uri });

        let (definition, hover, references) = tokio::join!(
            lsp.request(
                session_id,
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": text_doc.clone(),
                    "position": position.clone(),
                }),
                &worktree_path,
                10000,
            ),
            lsp.request(
                session_id,
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": text_doc.clone(),
                    "position": position.clone(),
                }),
                &worktree_path,
                10000,
            ),
            lsp.request(
                session_id,
                "textDocument/references",
                serde_json::json!({
                    "textDocument": text_doc,
                    "position": position,
                    "context": { "includeDeclaration": true },
                }),
                &worktree_path,
                10000,
            ),
        );

        let response = serde_json::json!({
            "definition": definition.unwrap_or(serde_json::Value::Null),
            "hover": hover.unwrap_or(serde_json::Value::Null),
            "references": references.unwrap_or(serde_json::Value::Null),
        });

        let json = serde_json::to_string_pretty(&response).map_err(WorktreeError::from)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn persist_and_emit_threads_changed(&self, worktree_path: &str) -> Result<(), McpError> {
        if let (Some(app), Some(data_dir)) = (
            self.state.app_handle.as_ref(),
            self.state.app_data_dir.as_ref(),
        ) {
            self.state
                .thread_store
                .save(data_dir, worktree_path)
                .map_err(|e| {
                    McpError::internal_error(format!("Failed to save threads: {e}"), None)
                })?;
            app.emit(
                "threads-changed",
                crate::thread_store::ThreadsChangedPayload {
                    worktree_name: worktree_path.to_string(),
                    source: "mcp".to_string(),
                    threads: self.state.thread_store.get_all(worktree_path),
                },
            )
            .map_err(|e| {
                McpError::internal_error(format!("Failed to emit threads-changed: {e}"), None)
            })?;
        }
        Ok(())
    }

    fn get_lsp_and_app(&self) -> Result<(Arc<crate::lsp::LspManager>, tauri::AppHandle), McpError> {
        let app = self
            .state
            .app_handle
            .as_ref()
            .ok_or_else(|| McpError::internal_error("AppHandle not available", None))?;
        let lsp = app.state::<Arc<crate::lsp::LspManager>>().inner().clone();
        Ok((lsp, app.clone()))
    }

    async fn did_open_file(
        &self,
        lsp: &crate::lsp::LspManager,
        session_id: u64,
        worktree_path: &str,
        file_path: &str,
        language: &str,
    ) -> Result<String, McpError> {
        crate::ws_server::validation::validate_relative_path(file_path, worktree_path)
            .map_err(|e| McpError::invalid_params(e, None))?;
        let full_path = std::path::Path::new(worktree_path).join(file_path);
        let content = tokio::fs::read_to_string(&full_path).await.map_err(|e| {
            McpError::internal_error(
                format!("ファイル読み取り失敗: {}: {e}", full_path.display()),
                None,
            )
        })?;

        let uri = url::Url::from_file_path(&full_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| {
                format!("file://{}", full_path.to_string_lossy().replace(' ', "%20"))
            });

        let did_open = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": language,
                    "version": 1,
                    "text": content,
                }
            }
        });

        lsp.send_message(session_id, &did_open.to_string(), worktree_path)
            .await
            .map_err(|e| McpError::internal_error(format!("didOpen送信失敗: {e}"), None))?;

        Ok(uri)
    }
}

/// Collect all files in a worktree, respecting .gitignore.
/// Only returns files that have a recognized language extension.
fn collect_all_files(worktree_path: &str) -> Vec<String> {
    let root = std::path::Path::new(worktree_path);
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let mut files = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if crate::lsp::detect::language_for_extension(ext).is_none() {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.to_string_lossy().to_string());
        }
    }
    files
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
        let thread_store = Arc::new(crate::thread_store::ThreadStore::default());

        let state = McpSharedState {
            repo_paths: Arc::new(parking_lot::RwLock::new(repo_paths)),
            pty_manager,
            app_config,
            broadcaster,
            agent_states,
            comment_store,
            thread_store,
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

    #[test]
    fn check_diagnostics_params_deserialize_defaults() {
        let json = r#"{"worktree": "/tmp/test"}"#;
        let params: CheckDiagnosticsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.worktree, "/tmp/test");
        assert!(params.file_paths.is_none());
        assert!(params.filter_mode.is_none());
        assert!(params.base_branch.is_none());
    }

    #[test]
    fn check_diagnostics_params_deserialize_with_filter_mode() {
        let json =
            r#"{"worktree": "/tmp/test", "filter_mode": "diff_context", "base_branch": "main"}"#;
        let params: CheckDiagnosticsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.filter_mode.as_deref(), Some("diff_context"));
        assert_eq!(params.base_branch.as_deref(), Some("main"));
    }

    #[test]
    fn collect_all_files_respects_gitignore() {
        use crate::git::test_helpers::{create_initial_commit, create_test_repo};

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let workdir = repo.workdir().unwrap();
        // Create files with recognized extensions
        std::fs::write(workdir.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(workdir.join("lib.ts"), "export {}").unwrap();
        // Create a file that should be ignored
        std::fs::write(workdir.join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(workdir.join("ignored.rs"), "// ignored").unwrap();
        // Create a file with unrecognized extension
        std::fs::write(workdir.join("data.xyz"), "data").unwrap();

        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let files = collect_all_files(&repo_path);
        assert!(files.contains(&"main.rs".to_string()));
        assert!(files.contains(&"lib.ts".to_string()));
        assert!(!files.contains(&"ignored.rs".to_string()));
        assert!(!files.contains(&"data.xyz".to_string()));
    }
}
