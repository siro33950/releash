import type { MockConfig } from "./tauri-mock";

// -------------------------------------------------------
// 型定義（src/types/ と同じ構造。import は避けて自己完結させる）
// -------------------------------------------------------

interface WorktreeBranch {
	name: string;
	is_default: boolean;
	worktree_path: string | null;
	management_kind: "working_area" | null;
	dirty_count: number;
	is_merged: boolean;
	has_pr: boolean;
	pr_number: number | null;
	pr_url: string | null;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
	agent_state?: "running" | "done" | "error" | "waiting";
	agent_state_timestamp?: number;
}

interface PrStatus {
	open_prs: Record<string, { number: number; url: string }>;
	merged_branches: string[];
}

// -------------------------------------------------------
// App.tsx 初期化に必要な最小レスポンスセット
// -------------------------------------------------------

const baseIpcHandler: Record<string, unknown> = {
	// App.tsx 初期化
	get_cwd: "/test/repo",
	get_main_repo_path: "/test/repo",
	list_worktrees: [],
	set_menu_items_enabled: null,

	// WorktreeView 初期化
	start_watching: 1,
	stop_watching: null,
	get_current_branch: "feat/test-branch",
	get_git_status: [],

	// RepoKanbanBoard
	list_branches_with_status: [],
	get_cached_pr_status: { open_prs: {}, merged_branches: [] } satisfies PrStatus,
	get_cached_issues: [],
	fetch_issues: [],
	list_workspace_statuses: [],
	get_releash_base: null,
	get_default_branch: "main",

	// Telemetry
	get_crash_reporting_enabled: true,
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
	get_branch_base: null,
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
		terminal_surface: {
			replay: "",
			sequence: 0,
			cols: 80,
			rows: 24,
		},
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
	get_repo_paths: ["/test/repo"],
	add_repo_path: null,
	remove_repo_path: null,

	// Editor
	get_file_at_ref: "",
	get_staged_content: "",
	get_repo_git_dir: "/test/repo/.git",

	// File system plugin (readDir)
	"plugin:fs|read_dir": [],

	// Updater plugin
	"plugin:updater|check": null,
	"plugin:updater|download_and_install": null,

	// Git log
	get_git_log: [],

	// IssuePanel
	list_branches: [],

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
	list_available_agent_session_providers: ["claude", "codex"],
	get_agent_session: null,
	open_agent_session: "attached",
	resume_agent_session: "resumed",
	restore_agent_session: "restored",
	archive_agent_session: "archived",
	delete_agent_session: null,
	confirm_agent_session_archive_delete: null,
	list_agent_session_history: { items: [], nextAfter: null },
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
	start_workflow: null,
	abort_workflow: null,
	approve_workflow_node: null,
	delete_workflow: null,
	open_workflow_in_editor: null,
	list_workflow_executions: [],
	get_workflow_execution: null,
	get_workflow_execution_log: [],
	get_workflow_execution_state: null,
	get_workflow_node_detail: null,
	resolve_active_execution_by_worktree: null,
	resolve_worktree_by_execution: null,

	// Workspace tree
	list_workspace_worktree_nodes: {
		nodes: [],
		sessions: [],
		preferredNodeId: null,
	},
	list_workspace_workflow_history: [],
	get_workspace_node_detail: null,
	close_workspace_node: null,
	approve_workspace_node: null,
	get_workspace_session_node_id: null,
	archive_workspace_workflow_execution: null,
	restore_workspace_workflow_execution: null,
};

// -------------------------------------------------------
// Kanban表示用ブランチリスト
// -------------------------------------------------------

export const kanbanBranches: WorktreeBranch[] = [
	{
		name: "feat/todo",
		is_default: false,
		worktree_path: null,
		management_kind: null,
		dirty_count: 0,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/wip",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-wip",
		management_kind: "working_area",
		dirty_count: 2,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/review",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-review",
		management_kind: "working_area",
		dirty_count: 0,
		is_merged: false,
		has_pr: true,
		pr_number: 42,
		pr_url: "https://github.com/test/repo/pull/42",
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	{
		name: "feat/done",
		is_default: false,
		worktree_path: null,
		management_kind: null,
		dirty_count: 0,
		is_merged: true,
		has_pr: false,
		pr_number: null,
		pr_url: null,
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
	return {
		ipcHandler: {
			...baseIpcHandler,
			...overrides,
		},
	};
}
