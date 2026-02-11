import type { AgentState } from "./protocol";

export interface KanbanTab {
	type: "kanban";
	id: "kanban";
}

export interface WorktreeTab {
	type: "worktree";
	id: string;
	rootPath: string;
	branchName: string;
	agentState?: AgentState;
}

export type WorkspaceTab = KanbanTab | WorktreeTab;
