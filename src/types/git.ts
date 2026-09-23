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

export interface BranchInfo {
	name: string;
	is_remote: boolean;
}

export interface WorktreeBranch {
	name: string;
	is_main_worktree: boolean;
	is_deleting: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	has_pr?: boolean;
	pr_number?: number | null;
	pr_url?: string | null;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
}

interface PrInfo {
	number: number;
	url: string;
}

export interface PrStatus {
	open_prs: Record<string, PrInfo>;
	merged_branches: string[];
}

interface PrAuthor {
	login: string;
}

interface Milestone {
	title: string;
}

interface IssueLabel {
	name: string;
	color: string;
}

export interface IssueInfo {
	number: number;
	default_branch_name: string;
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
