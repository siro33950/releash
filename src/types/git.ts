export interface GitFileStatus {
	path: string;
	index_status: "new" | "modified" | "deleted" | "renamed" | "none";
	worktree_status: "new" | "modified" | "deleted" | "ignored" | "none";
}

export interface CommitInfo {
	hash: string;
	short_hash: string;
	message: string;
	author_name: string;
	author_email: string;
	timestamp: number;
}
