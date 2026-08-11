export interface WorktreeTab {
	type: "worktree";
	id: string;
	rootPath: string;
	branchName: string;
	repoName?: string;
}
