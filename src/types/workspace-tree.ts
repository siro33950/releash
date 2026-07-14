import type { AgentState } from "./protocol";
import type { SessionState, SessionSummary } from "./session";
import type {
	JsonValue,
	NodeExecutionStatus,
	WorkflowRunSummary,
} from "./workflow";

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

type WorkspaceStepType = "command" | "session" | "fanout";

export interface WorkspaceFanoutParent {
	parentNode: string;
	parentAttempt: number;
	itemIndex?: number;
	childIndex: number;
}

export interface WorkspaceWorkflowStepNode {
	kind: "step";
	id: string;
	runId: string;
	worktreePath: string;
	title: string;
	nodeName: string;
	status: WorkspaceStepStatus;
	stepType: WorkspaceStepType;
	nodeExecutionStatus?: NodeExecutionStatus;
	canApprove?: boolean;
	updatedAt: number;
	runIndex?: number | null;
	attempt: number;
	nodeExecutionId?: string;
	sessionId?: string;
	artifact?: JsonValue;
	fanoutParent?: WorkspaceFanoutParent;
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

interface WorkflowStepRepresentative {
	executionId: string;
	stepName: string;
	runIndex?: number | null;
	representative: WorkspaceStepStatus;
}

interface WorkflowRepresentative {
	executionId: string;
	representative: WorkspaceStepStatus;
}

export interface WorktreeStepStatusView {
	worktreePath: string;
	version: number;
	steps: WorkflowStepRepresentative[];
	workflows: WorkflowRepresentative[];
}
