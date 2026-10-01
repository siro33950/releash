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
}

// -------------------------------------------------------
// App.tsx 初期化に必要な最小レスポンスセット
// -------------------------------------------------------

const baseIpcHandler: Record<string, unknown> = {
	// App.tsx 初期化
	"startup-outcome": { type: "ready" },
	"provider-hook-health": [],
	"startup-repository": "/test/repo",
	"worktrees": [],
	set_menu_items_enabled: null,

	// WorktreeView 初期化
	start_watching: 1,
	start_git_dir_watching: 1,
	stop_git_dir_watching: null,
	"workspace-state": null,
	"review-threads": [],
	"review-snapshot": {
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
	"current-branch": "feat/test-branch",

	// RepoKanbanBoard
	"workspaceBranches": [],
	"issues": { issues: [] },
	fetch_issues: null,
    refresh_workspaces: null,
	"releash-base": null,

	// Telemetry
	update_crash_reporting: null,
	update_performance_telemetry: null,
	report_frontend_error: null,
	report_mounted_xterm_count: null,
	report_usage_event: null,

	// Background / Autostart
	"desktop-settings": {
		closeToTray: false,
		startMinimized: false,
		crashReporting: true,
		performanceTelemetry: true,
		autoLaunch: false,
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
	},
	terminal_snapshot: {
		session_key: "mock-session",
		is_exited: false,
		exit_code: null,
	},
	start_state_subscription: { __mockTerminalAttachment: true },
	stop_state_subscription: null,
	write_terminal_surface: null,
	resize_terminal_surface: null,
	kill_terminal_surface: null,

	// External editor
	"external-editor": { selected: "", editors: [] },
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
	"notion-config": null,
	save_notion_config: null,
	delete_notion_config: null,
	validate_notion_config: {
		status: "configured",
		properties: [],
	},
	"notion-tasks": { page: { tasks: [], has_more: false } },
	"notion-label-options": { options: [] },

	// Worktree作成
	create_worktree: null,
	remove_worktree: null,

	// AgentSession TUI
	"providers": ["claude", "codex"],
	"agent-session": null,
	open_agent_session: "attached",
	restore_agent_session: "restored",
	archive_agent_session: "archived",
	delete_agent_session: null,
	"session-history": { items: [], hasMore: false },
	resume_agent_session_history_candidate: "mock-agent-session-1",
	"provider-availability": {
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
	workflows: [],
	"workflow-config": { approval_auto_approve: false },
	// Chromiumで走るmockテストはDOM span/CSSのassertを維持するため
	// DOMレンダラを明示する（WebGL既定の実機経路はwdio harnessが担う）。
	"performance-switches": {
		realAppMode: false,
		terminal: {
			disableOutputFlowControl: false,
			disableTerminalJournal: false,
			disableRendererWriteSerialization: false,
			disableWebglRenderer: true,
		},
	},
	diagnostics: {
		items: [],
		workflow_summaries: {},
		facet_summaries: {},
		facet_usage: {},
	},
	get_workflow_config: { approval_auto_approve: false },
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
	},
	{
		name: "feat/wip",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: "/test/repo-worktrees/feat-wip",
		dirty_count: 2,
		is_merged: false,
	},
	{
		name: "feat/review",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: "/test/repo-worktrees/feat-review",
		dirty_count: 0,
		is_merged: false,
	},
	{
		name: "feat/done",
		is_main_worktree: false,
		is_deleting: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: true,
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
    const stateNames = ["repository-paths", "workspaces", "selection", "node-detail", "agent-session", "session-node", "session-history", "providers", "branches", "branch-base", "branch-status", "current-branch", "issues", "worktrees", "repository-root", "startup-repository", "workspace-state", "review-snapshot", "review-file-view", "review-threads", "workflows", "workflow", "workflow-source", "facets", "facet", "diagnostics", "desktop-settings", "notion-config", "notion-tasks", "notion-label-options", "provider-availability", "external-editor", "releash-base", "workflow-config", "performance-switches", "provider-hook-health", "startup-outcome"];
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
