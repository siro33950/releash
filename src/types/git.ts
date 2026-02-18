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

export interface BranchCard {
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
	is_remote_only: boolean;
	has_upstream: boolean;
	remote_name: string | null;
	agent_state?: "running" | "done" | "error" | "waiting";
	agent_state_timestamp?: number;
}

export type ProviderStatus =
	| "available"
	| { cli_not_found: { cli: string } }
	| "not_authenticated"
	| "unsupported_platform"
	| "no_remote";

export interface PrInfo {
	number: number;
	url: string;
}

export interface PrStatus {
	open_prs: Record<string, PrInfo>;
	merged_branches: string[];
}

export interface PrAuthor {
	login: string;
}

export interface PrComment {
	author: PrAuthor;
	body: string;
	created_at: string;
}

export interface PrReview {
	author: PrAuthor;
	body: string;
	state: string;
	submitted_at: string;
}

export interface PrDetail {
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

export interface Milestone {
	title: string;
}

export interface IssueLabel {
	name: string;
	color: string;
}

export interface IssueInfo {
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

export interface CommitInfo {
	hash: string;
	short_hash: string;
	message: string;
	author_name: string;
	author_email: string;
	timestamp: number;
}
