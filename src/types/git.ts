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

export interface CommitInfo {
	hash: string;
	short_hash: string;
	message: string;
	author_name: string;
	author_email: string;
	timestamp: number;
}
