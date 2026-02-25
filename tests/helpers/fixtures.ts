import type { MockConfig } from "./tauri-mock";

// -------------------------------------------------------
// 型定義（src/types/ と同じ構造。import は避けて自己完結させる）
// -------------------------------------------------------

interface WorktreeBranch {
	name: string;
	is_default: boolean;
	worktree_path: string | null;
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

interface GitFileStatus {
	path: string;
	index_status: "new" | "modified" | "deleted" | "renamed" | "none";
	worktree_status: "new" | "modified" | "deleted" | "ignored" | "none";
}

interface WorktreeEntry {
	name: string;
	path: string;
	branch: string;
	is_main: boolean;
	is_locked: boolean;
	dirty_count: number;
	base_branch: string | null;
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

	// PullRequestPanel
	get_pr_detail: null,

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
};

// -------------------------------------------------------
// Kanban表示用ブランチリスト
// -------------------------------------------------------

export const kanbanBranches: WorktreeBranch[] = [
	{
		name: "feat/todo",
		is_default: false,
		worktree_path: null,
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
// PrDetail フィクスチャ（PullRequestPanel用）
// -------------------------------------------------------

interface PrAuthor {
	login: string;
}

interface PrComment {
	author: PrAuthor;
	body: string;
	created_at: string;
}

interface PrReview {
	author: PrAuthor;
	body: string;
	state: string;
	submitted_at: string;
}

interface PrDetail {
	number: number;
	title: string;
	body: string;
	state: string;
	url: string;
	author: PrAuthor;
	created_at: string;
	head_ref_name: string;
	base_ref_name: string;
	additions: number;
	deletions: number;
	changed_files: number;
	comments: PrComment[];
	reviews: PrReview[];
}

export const prDetailOpen: PrDetail = {
	number: 42,
	title: "feat: add screenshot testing infrastructure",
	body: "## Summary\n\nAdds Playwright-based visual regression testing.\n\n## Changes\n- New config file\n- Helper utilities\n- 84 screenshot tests",
	state: "OPEN",
	url: "https://github.com/test/repo/pull/42",
	author: { login: "testuser" },
	created_at: "2026-01-15T10:00:00Z",
	head_ref_name: "feat/screenshot-tests",
	base_ref_name: "main",
	additions: 1200,
	deletions: 50,
	changed_files: 15,
	comments: [
		{
			author: { login: "reviewer1" },
			body: "Looks good overall! A few minor suggestions.",
			created_at: "2026-01-16T09:00:00Z",
		},
	],
	reviews: [
		{
			author: { login: "reviewer1" },
			body: "Nice work!",
			state: "APPROVED",
			submitted_at: "2026-01-16T10:00:00Z",
		},
	],
};

export const prDetailMerged: PrDetail = {
	...prDetailOpen,
	state: "MERGED",
	title: "fix: resolve flaky test in CI",
};

export const prDetailChangesRequested: PrDetail = {
	...prDetailOpen,
	state: "OPEN",
	title: "refactor: restructure panel components",
	reviews: [
		{
			author: { login: "reviewer2" },
			body: "Please address the following concerns before merging.",
			state: "CHANGES_REQUESTED",
			submitted_at: "2026-01-16T11:00:00Z",
		},
	],
};

// -------------------------------------------------------
// IssueInfo フィクスチャ（IssuePanel用）
// -------------------------------------------------------

interface IssueLabel {
	name: string;
	color: string;
}

interface Milestone {
	title: string;
}

interface IssueInfo {
	number: number;
	title: string;
	state: string;
	url: string;
	author: PrAuthor;
	created_at: string;
	updated_at: string;
	labels: IssueLabel[];
	assignees: PrAuthor[];
	body: string;
	milestone: Milestone | null;
}

export const issueList: IssueInfo[] = [
	{
		number: 101,
		title: "Add dark mode support for mobile view",
		state: "open",
		url: "https://github.com/test/repo/issues/101",
		author: { login: "dev1" },
		created_at: "2026-01-10T08:00:00Z",
		updated_at: "2026-01-12T14:00:00Z",
		labels: [
			{ name: "enhancement", color: "a2eeef" },
			{ name: "ui", color: "d4c5f9" },
		],
		assignees: [{ login: "dev1" }],
		body: "Mobile view should support dark mode theme switching.",
		milestone: { title: "v0.2.0" },
	},
	{
		number: 102,
		title: "Fix commit message validation",
		state: "open",
		url: "https://github.com/test/repo/issues/102",
		author: { login: "dev2" },
		created_at: "2026-01-11T09:00:00Z",
		updated_at: "2026-01-11T09:00:00Z",
		labels: [{ name: "bug", color: "d73a4a" }],
		assignees: [],
		body: "Commit message with special characters causes an error.",
		milestone: null,
	},
	{
		number: 103,
		title: "Improve search performance for large repos",
		state: "open",
		url: "https://github.com/test/repo/issues/103",
		author: { login: "dev3" },
		created_at: "2026-01-12T10:00:00Z",
		updated_at: "2026-01-13T16:00:00Z",
		labels: [{ name: "performance", color: "fbca04" }],
		assignees: [{ login: "dev1" }, { login: "dev3" }],
		body: "Search is slow on repos with 10k+ files.",
		milestone: { title: "v0.2.0" },
	},
];

// -------------------------------------------------------
// Notion フィクスチャ（NotionPanel用）
// -------------------------------------------------------

interface NotionRepoConfig {
	api_token: string;
	database_id: string;
	property_mapping: {
		title: string;
		labels: { name: string; property_type: string }[];
		branch_name: string;
		branch_prefix: string;
	};
}

interface NotionTask {
	id: string;
	title: string;
	url: string;
	labels: Record<string, string[]>;
	branch_name: string;
	created_at: string;
	last_edited_at: string;
}

interface NotionLabelOption {
	property_name: string;
	property_type: string;
	options: string[];
	option_ids: string[];
}

export const notionConfig: NotionRepoConfig = {
	api_token: "ntn_test_token_xxxxx",
	database_id: "abc123-def456",
	property_mapping: {
		title: "Name",
		labels: [
			{ name: "Status", property_type: "select" },
			{ name: "Priority", property_type: "select" },
		],
		branch_name: "Branch",
		branch_prefix: "notion/",
	},
};

export const notionTasks: NotionTask[] = [
	{
		id: "task-1",
		title: "Design new onboarding flow",
		url: "https://notion.so/task-1",
		labels: { Status: ["In Progress"], Priority: ["High"] },
		branch_name: "notion/design-onboarding",
		created_at: "2026-01-10T08:00:00Z",
		last_edited_at: "2026-01-15T12:00:00Z",
	},
	{
		id: "task-2",
		title: "Update API documentation",
		url: "https://notion.so/task-2",
		labels: { Status: ["Todo"], Priority: ["Medium"] },
		branch_name: "notion/update-api-docs",
		created_at: "2026-01-11T09:00:00Z",
		last_edited_at: "2026-01-14T10:00:00Z",
	},
	{
		id: "task-3",
		title: "Fix notification bug on Safari",
		url: "https://notion.so/task-3",
		labels: { Status: ["Todo"], Priority: ["Low"] },
		branch_name: "notion/fix-safari-notification",
		created_at: "2026-01-12T10:00:00Z",
		last_edited_at: "2026-01-13T11:00:00Z",
	},
];

export const notionLabelOptions: NotionLabelOption[] = [
	{
		property_name: "Status",
		property_type: "select",
		options: ["Todo", "In Progress", "Done"],
		option_ids: ["opt-1", "opt-2", "opt-3"],
	},
	{
		property_name: "Priority",
		property_type: "select",
		options: ["High", "Medium", "Low"],
		option_ids: ["opt-4", "opt-5", "opt-6"],
	},
];

// -------------------------------------------------------
// Kanban フルバリエーション（7件、全カード状態を網羅）
// -------------------------------------------------------

export const kanbanBranchesFull: WorktreeBranch[] = [
	// Todo: ローカルブランチ（worktreeなし）
	{
		name: "feat/todo-item",
		is_default: false,
		worktree_path: null,
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
	// In Progress: dirty + ahead/behind
	{
		name: "feat/active-work",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-active-work",
		dirty_count: 5,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 3,
		behind: 1,
		has_upstream: true,
		base_ahead: 0,
	},
	// In Progress: agent running
	{
		name: "feat/agent-running",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-agent-running",
		dirty_count: 0,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 1,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
		agent_state: "running",
		agent_state_timestamp: 9999999999,
	},
	// In Progress: agent done
	{
		name: "feat/agent-done",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-agent-done",
		dirty_count: 2,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 2,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
		agent_state: "done",
		agent_state_timestamp: 9999999999,
	},
	// Review: PR あり
	{
		name: "feat/in-review",
		is_default: false,
		worktree_path: "/test/repo-worktrees/feat-in-review",
		dirty_count: 0,
		is_merged: false,
		has_pr: true,
		pr_number: 88,
		pr_url: "https://github.com/test/repo/pull/88",
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
	},
	// Done: merged
	{
		name: "feat/completed",
		is_default: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: true,
		has_pr: true,
		pr_number: 80,
		pr_url: "https://github.com/test/repo/pull/80",
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
