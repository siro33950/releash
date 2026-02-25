import type { AgentState } from "./protocol";

export interface WorktreeTab {
	type: "worktree";
	id: string;
	rootPath: string;
	branchName: string;
	repoName?: string;
	agentState?: AgentState;
}
