import type { MockConfig } from "./tauri-mock";

// -------------------------------------------------------
// 型定義（src/types/ と同じ構造。import は避けて自己完結させる）
// -------------------------------------------------------

export interface BranchCard {
	name: string;
	is_default: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	has_pr: boolean;
	pr_number: number | null;
	pr_url: string | null;
	agent_state?: "running" | "done" | "error" | "waiting";
	agent_state_timestamp?: number;
}

export interface GitFileStatus {
	path: string;
	index_status: "new" | "modified" | "deleted" | "renamed" | "none";
	worktree_status: "new" | "modified" | "deleted" | "ignored" | "none";
}

export interface WorktreeEntry {
	name: string;
	path: string;
	branch: string;
	is_main: boolean;
	is_locked: boolean;
	dirty_count: number;
	base_branch: string | null;
}

export interface PrStatus {
	open_prs: Record<string, { number: number; url: string }>;
	merged_branches: string[];
}

// -------------------------------------------------------
// App.tsx 初期化に必要な最小レスポンスセット
// -------------------------------------------------------

export const baseIpcHandler: Record<string, unknown> = {
	// App.tsx 初期化
	get_cwd: "/test/repo",
	get_main_repo_path: "/test/repo",
	list_worktrees: [],
	check_pr_provider_status: "available",
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
	get_agent_states: {},
	get_releash_base: null,
	get_default_branch: "main",

	// SettingsPanel hooks
	generate_hooks_config: "{}",
	get_hooks_status: "not_configured",
	apply_hooks_config: null,

	// Webhook notifications
	get_notify_config: {
		webhook_url: "",
		on_running: false,
		on_done: true,
		on_error: true,
		on_waiting: true,
		desktop_mode: "always",
		inactive_timeout_minutes: 2,
	},
	update_notify_config: null,

	// Telemetry
	get_crash_reporting_enabled: true,
	update_crash_reporting: null,

	// Terminal (PTY) — モック上は何もしない
	get_or_spawn_pty: { pty_id: 1, output: "" },
	write_pty: null,
	resize_pty: null,
	kill_ptys_by_worktree: null,

	// Remote server
	get_server_config: { ip: "0.0.0.0", port: 19700, mode: "local" },
	get_server_info: null,
	get_network_info: [],
	stop_server: null,

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

	// Search
	search_files: { matches: [], total_matches: 0, truncated: false },
};

// -------------------------------------------------------
// Kanban表示用ブランチリスト
// -------------------------------------------------------

export const kanbanBranches: BranchCard[] = [
	{
		name: "feat/todo",
		is_default: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
	},
	{
		name: "feat/wip",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-wip",
		dirty_count: 2,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
	},
	{
		name: "feat/review",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-review",
		dirty_count: 0,
		is_merged: false,
		has_pr: true,
		pr_number: 42,
		pr_url: "https://github.com/test/repo/pull/42",
	},
	{
		name: "feat/done",
		is_default: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: true,
		has_pr: false,
		pr_number: null,
		pr_url: null,
	},
];

// -------------------------------------------------------
// Git変更ファイル
// -------------------------------------------------------

export const unstagedChanges: GitFileStatus[] = [
	{
		path: "src/App.tsx",
		index_status: "none",
		worktree_status: "modified",
	},
	{
		path: "README.md",
		index_status: "none",
		worktree_status: "modified",
	},
];

export const stagedChanges: GitFileStatus[] = [
	{
		path: "src/main.ts",
		index_status: "modified",
		worktree_status: "none",
	},
];

export const mixedChanges: GitFileStatus[] = [
	...unstagedChanges,
	...stagedChanges,
];

// -------------------------------------------------------
// ファイルツリー用データ（plugin:fs|read_dir の戻り値）
// -------------------------------------------------------

export const rootDirEntries = [
	{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
	{ name: "README.md", isDirectory: false, isFile: true, isSymlink: false },
	{
		name: "package.json",
		isDirectory: false,
		isFile: true,
		isSymlink: false,
	},
];

export const srcDirEntries = [
	{ name: "App.tsx", isDirectory: false, isFile: true, isSymlink: false },
	{ name: "main.ts", isDirectory: false, isFile: true, isSymlink: false },
	{
		name: "components",
		isDirectory: true,
		isFile: false,
		isSymlink: false,
	},
];

// -------------------------------------------------------
// 検索結果フィクスチャ
// -------------------------------------------------------

export const searchResults = {
	matches: [
		{
			path: "src/App.tsx",
			line_number: 10,
			line_content: 'import { useState } from "react";',
			match_start: 10,
			match_end: 18,
		},
		{
			path: "src/App.tsx",
			line_number: 25,
			line_content: "  const [state, setState] = useState(false);",
			match_start: 30,
			match_end: 38,
		},
		{
			path: "src/main.ts",
			line_number: 5,
			line_content: "// useState is not used here",
			match_start: 3,
			match_end: 11,
		},
	],
	total_matches: 3,
	truncated: false,
};

// -------------------------------------------------------
// ブランチ一覧（CreateWorktreeDialog用）
// -------------------------------------------------------

export interface BranchInfo {
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
// FS プラグイン追加コマンド（ファイル操作テスト用）
// -------------------------------------------------------

export const fsPluginCommands: Record<string, unknown> = {
	"plugin:fs|write_text_file": null,
	"plugin:fs|mkdir": null,
	"plugin:fs|remove": null,
	"plugin:fs|rename": null,
	"plugin:fs|exists": false,
	"plugin:fs|copy_file": null,
	"plugin:opener|reveal_item_in_dir": null,
};

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
