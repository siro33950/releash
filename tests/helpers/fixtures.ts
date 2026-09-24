import type { MockConfig } from "./tauri-mock";

// -------------------------------------------------------
// 型定義（src/types/ と同じ構造。import は避けて自己完結させる）
// -------------------------------------------------------

interface WorktreeBranch {
	name: string;
	is_main_worktree: boolean;
	is_deleting: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
}

// -------------------------------------------------------
// App.tsx 初期化に必要な最小レスポンスセット
// -------------------------------------------------------

const baseIpcHandler: Record<string, unknown> = {
	// App.tsx 初期化
	get_application_startup_outcome: { type: "ready" },
	list_provider_hook_health_warnings: [],
	"startup-repository": "/test/repo",
	"worktrees": [],
	set_menu_items_enabled: null,

	// WorktreeView 初期化
	start_watching: 1,
	start_git_dir_watching: 1,
	stop_git_dir_watching: null,
	"workspace-state": null,
	list_review_threads: [],
	get_review_snapshot: {
		version: 1,
		stale: false,
		loading: false,
		base: "head",
		files: [],
		stagedFiles: [],
		changedFiles: [],
		diffStats: [],
		tree: [],
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
	},
	stop_watching: null,
	"current-branch": "feat/test-branch",

	// RepoKanbanBoard
	"workspaceBranches": [],
	"issues": [],
	fetch_issues: null,
    refresh_workspaces: null,
	get_releash_base: null,

	// Telemetry
	update_crash_reporting: null,
	get_performance_telemetry_enabled: true,
	update_performance_telemetry: null,
	report_frontend_error: null,
	report_mounted_xterm_count: null,
	report_usage_event: null,

	// Background / Autostart
	get_app_settings: {
		close_to_tray: false,
		start_minimized: false,
		last_root_path: "/test/repo",
	},
	update_app_settings: null,
	"plugin:autostart|is_enabled": false,
	"plugin:autostart|enable": null,
	"plugin:autostart|disable": null,

	// Branch base
	"branch-base": null,
	set_branch_base: null,

	// Terminal (PTY) — モック上は何もしない
	get_or_spawn_terminal_surface: {
		session_key: "mock-session",
		restored_from_checkpoint: false,
		is_new: true,
		is_exited: false,
		exit_code: null,
	},
	get_terminal_surface: {
		session_key: "mock-session",
		is_exited: false,
		exit_code: null,
	},
	attach_terminal_surface: { __mockTerminalAttachment: true },
	detach_terminal_surface: null,
	write_terminal_surface: null,
	resize_terminal_surface: null,
	kill_terminal_surface: null,

	// External editor
	get_external_editor: "",
	detect_editors: [],
	update_external_editor: null,

	// Repo registry
	"repository-paths": ["/test/repo"],
	add_repo_path: true,
	remove_repo_path: true,

	// File system plugin (readDir)
	"plugin:fs|read_dir": [],

	// Updater plugin
	"plugin:updater|check": null,
	"plugin:updater|download_and_install": null,

	// IssuePanel
	"branches": [],

	// NotionPanel
	get_notion_config: null,
	save_notion_config: null,
	delete_notion_config: null,
	validate_notion_config: {
		status: "configured",
		properties: [],
	},
	query_notion_tasks: { tasks: [], has_more: false, next_cursor: null },
	fetch_notion_label_options: [],

	// Worktree作成
	create_worktree: null,
	remove_worktree: null,
	delete_branch: null,

	// AgentSession TUI
	"providers": ["claude", "codex"],
	"agent-session": null,
	open_agent_session: "attached",
	restore_agent_session: "restored",
	archive_agent_session: "archived",
	delete_agent_session: null,
	"session-history": { items: [], hasMore: false },
	resume_agent_session_history_candidate: "mock-agent-session-1",
	get_provider_availability: {
		providers: [
			{
				provider: "claude",
				displayName: "Claude",
				defaultExecutable: "claude",
				configuredExecutable: null,
				effectiveExecutable: "claude",
				available: true,
				resolvedExecutable: "/usr/local/bin/claude",
				unavailableReason: null,
			},
			{
				provider: "codex",
				displayName: "Codex",
				defaultExecutable: "codex",
				configuredExecutable: null,
				effectiveExecutable: "codex",
				available: true,
				resolvedExecutable: "/usr/local/bin/codex",
				unavailableReason: null,
			},
		],
	},
	// Workspace state
	save_workspace_state: null,

	// Workflow
	list_workflows: [],
	get_workflow_config: { approval_auto_approve: false },
	get_automation_config_dir: "/test/automation",
	diagnose_all_cmd: {
		items: [],
		workflow_summaries: {},
		facet_summaries: {},
		facet_usage: {},
	},
	start_workflow: null,
	abort_workflow: null,
	approve_workflow_node: null,
	delete_workflow: null,
	open_workflow_in_editor: null,

	// Workspace tree
	"workspaceTree": {
		nodes: [],
		archivedSessions: [],
		preferredNodeId: null,
	},
	"workspaceWorkflowHistory": [],
	"node-detail": null,
	close_workspace_node: null,
	approve_workspace_node: null,
	"session-node": null,
	archive_workspace_workflow_execution: null,
	restore_workspace_workflow_execution: null,
};

// -------------------------------------------------------
// Kanban表示用ブランチリスト
// -------------------------------------------------------

export const kanbanBranches: WorktreeBranch[] = [
	{
		name: "feat/todo",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: false,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/wip",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: "/test/repo-worktrees/feat-wip",
		dirty_count: 2,
		is_merged: false,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/review",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: "/test/repo-worktrees/feat-review",
		dirty_count: 0,
		is_merged: false,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/done",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: true,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
];

// -------------------------------------------------------
// ブランチ一覧（CreateWorktreeDialog用）
// -------------------------------------------------------

interface BranchInfo {
	name: string;
	is_remote: boolean;
}

export const branchList: BranchInfo[] = [
	{ name: "main", is_remote: false },
	{ name: "develop", is_remote: false },
	{ name: "feat/existing", is_remote: false },
	{ name: "origin/main", is_remote: true },
	{ name: "origin/develop", is_remote: true },
];

// -------------------------------------------------------
// ヘルパー: MockConfig を組み立てる
// -------------------------------------------------------

export function buildMockConfig(
	overrides: Record<string, unknown> = {},
): MockConfig {
	const values = { ...baseIpcHandler, ...overrides };
    const stateNames = ["repository-paths", "workspaces", "selection", "node-detail", "agent-session", "session-node", "session-history", "providers", "branches", "branch-base", "branch-status", "current-branch", "issues", "worktrees", "repository-root", "startup-repository", "workspace-state"];
    const states: Record<string, unknown> = { "repository-root": "/test/repo", selection: null };
    for (const kind of stateNames) {
        if (kind in values) { states[kind] = values[kind]; delete values[kind]; }
    }
    const workspace = {
        branches: values.workspaceBranches as MockConfig["workspace"]["branches"],
        tree: values.workspaceTree as MockConfig["workspace"]["tree"],
        history: values.workspaceWorkflowHistory as MockConfig["workspace"]["history"],
    };
    delete values.workspaceBranches;
    delete values.workspaceTree;
    delete values.workspaceWorkflowHistory;
    return { responses: values, states, workspace };
}
