import type { AgentState } from "./protocol";
import type { SessionState, SessionSummary } from "./session";
import type {
	Artifact,
	NodeExecutionStatus,
	NodeKind,
	WorkflowExecutionSummary,
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
			kind: "workflowNode";
			worktreePath: string;
			executionId: string;
			nodeExecutionId: string;
			nodeName: string;
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
	workflowNodeSession: boolean;
	nodeExecutionId?: string | null;
	nodeName?: string | null;
	attempt?: number | null;
	agentState?: AgentState | null;
}

export type WorkspaceNodeStatus =
	| "queued"
	| "running"
	| "failed"
	| "error"
	| "waiting"
	| "aborted"
	| "completed";

export interface WorkspaceFanoutParent {
	parentNode: string;
	parentAttempt: number;
	itemIndex?: number;
	childIndex: number;
}

export interface WorkspaceWorkflowNodeExecution {
	kind: "node";
	nodeExecutionId: string;
	executionId: string;
	worktreePath: string;
	title: string;
	nodeName: string;
	status: WorkspaceNodeStatus;
	nodeKind: NodeKind;
	nodeExecutionStatus?: NodeExecutionStatus;
	canApprove?: boolean;
	updatedAt: number;
	attempt: number;
	sessionId?: string;
	artifact?: Artifact;
	fanoutParent?: WorkspaceFanoutParent;
	sessions: WorkspaceSessionNode[];
}

export type WorkspaceWorkflowNodeDetail = WorkspaceWorkflowNodeExecution;

export interface WorkspaceWorkflowExecutionNode {
	kind: "workflow";
	executionId: string;
	worktreePath: string;
	workflowName: string;
	title: string;
	status: WorkspaceNodeStatus;
	canStop: boolean;
	updatedAt: number;
	nodeExecutions: WorkspaceWorkflowNodeExecution[];
}

export interface WorkspaceWorkflowHistoryItem {
	executionId: string;
	worktreePath: string;
	title: string;
	status: WorkspaceNodeStatus | WorkflowExecutionSummary["status"];
	updatedAt: number;
	archivedAt: number;
	archiveReason: "auto_no_sessions" | "manual" | string;
}

export type WorkspaceTreeNode =
	| WorkspaceSessionNode
	| WorkspaceWorkflowExecutionNode;

export type WorkspaceSessionHistoryItem = SessionSummary;

interface NodeExecutionRepresentative {
	nodeExecutionId: string;
	executionId: string;
	nodeName: string;
	attempt: number | null;
	representative: WorkspaceNodeStatus;
}

interface WorkflowExecutionRepresentative {
	executionId: string;
	representative: WorkspaceNodeStatus;
}

export interface WorktreeNodeStatusView {
	worktreePath: string;
	version: number;
	nodeExecutions: NodeExecutionRepresentative[];
	workflowExecutions: WorkflowExecutionRepresentative[];
}
