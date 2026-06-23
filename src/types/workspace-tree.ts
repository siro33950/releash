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
			kind: "workflowStep";
			worktreePath: string;
			runId: string;
			stepId: string;
			stepName: string;
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

export type WorkspaceStepStatus =
	| "queued"
	| "running"
	| "failed"
	| "error"
	| "waiting"
	| "aborted"
	| "completed";

export type WorkspaceStepType = "agent" | "bash" | "approval" | "parallel";

export interface WorkspaceWorkflowStepNode {
	kind: "step";
	id: string;
	runId: string;
	worktreePath: string;
	title: string;
	status: WorkspaceStepStatus;
	stepType: WorkspaceStepType;
	canReject?: boolean;
	updatedAt: number;
	runIndex?: number | null;
	sessions: WorkspaceSessionNode[];
}

export type WorkspaceWorkflowStepDetail = WorkspaceWorkflowStepNode;

export interface WorkspaceWorkflowNode {
	kind: "workflow";
	runId: string;
	worktreePath: string;
	workflowName: string;
	title: string;
	status: WorkspaceStepStatus;
	canStop: boolean;
	updatedAt: number;
	steps: WorkspaceWorkflowStepNode[];
}

export interface WorkspaceWorkflowHistoryItem {
	runId: string;
	worktreePath: string;
	title: string;
	status: WorkspaceStepStatus | WorkflowRunSummary["status"];
	updatedAt: number;
	archivedAt: number;
	archiveReason: "auto_no_sessions" | "manual" | string;
}

export type WorkspaceTreeNode = WorkspaceSessionNode | WorkspaceWorkflowNode;

export type WorkspaceSessionHistoryItem = SessionSummary;

export interface WorkflowStepStatusChange {
	worktreePath: string;
	executionId: string;
	stepName: string;
	runIndex?: number | null;
	representative?: WorkspaceStepStatus | null;
	workflowRepresentative?: WorkspaceStepStatus | null;
	version: number;
}
