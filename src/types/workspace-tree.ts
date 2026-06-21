import type { AgentState } from "./protocol";
import type { SessionState, SessionSummary } from "./session";
import type { WorkflowRunSummary } from "./workflow";

export type CenterSelection =
	| {
			kind: "agentSession";
			worktreePath: string;
			sessionId: string;
	  }
	| {
			kind: "newAgentSession";
			worktreePath: string;
	  }
	| {
			kind: "workflowRun";
			worktreePath: string;
			runId: string;
			focus?: {
				sessionId?: string;
				stepName?: string;
				runIndex?: number;
			};
	  };

export type CenterSelectionRequest = CenterSelection & {
	requestId: number;
	branchName?: string;
	repoName?: string;
};

export interface WorkspaceSessionNode {
	kind: "session";
	id: string;
	worktreePath: string;
	title: string;
	state: SessionState;
	updatedAt: number;
	workflowStepSession: boolean;
	stepName?: string | null;
	runIndex?: number | null;
	agentState?: AgentState | null;
}

export interface WorkspaceWorkflowNode {
	kind: "workflow";
	runId: string;
	worktreePath: string;
	title: string;
	status: WorkflowRunSummary["status"];
	updatedAt: number;
	children: WorkspaceSessionNode[];
}

export interface WorkspaceWorkflowHistoryItem {
	runId: string;
	worktreePath: string;
	title: string;
	status: WorkflowRunSummary["status"];
	updatedAt: number;
	archivedAt: number;
	archiveReason: "auto_no_sessions" | "manual" | string;
	children: WorkspaceSessionNode[];
}

export type WorkspaceTreeNode = WorkspaceSessionNode | WorkspaceWorkflowNode;

export type WorkspaceSessionHistoryItem = SessionSummary;
