export function computeWorktreeDir(repoPath: string): string {
	const parent = repoPath.replace(/\/[^/]+\/?$/, "");
	const repoName = repoPath.split("/").filter(Boolean).pop() ?? "repo";
	return `${parent}/${repoName}-worktrees`;
}

export function branchToDir(branch: string): string {
	return branch.replace(/\//g, "-");
}
