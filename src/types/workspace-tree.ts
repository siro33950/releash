import type {
	AgentSessionItem,
	AgentSessionLaunchAttachment,
} from "./agent-session";
import type { WorkflowExecutionSummary } from "./workflow";

export type CenterSelection =
	| {
			kind: "node";
			worktreePath: string;
			nodeId: string;
	  }
	| {
			kind: "agent_session";
			worktreePath: string;
			agentSessionId: string;
			initialAttachment?: AgentSessionLaunchAttachment;
	  }
	| {
			kind: "agent_session_launching";
			worktreePath: string;
			provider: string;
			launchToken: string;
			error?: string;
	  };

export type WorkspaceNodeStatus =
	| "queued"
	| "running"
	| "paused"
	| "failed"
	| "error"
	| "waiting"
	| "interrupted"
	| "aborted"
	| "completed";

export interface WorkspaceNodeCapabilities {
	canApprove: boolean;
	canRetry: boolean;
	canClose: boolean;
}

export interface WorkspaceWorkflowCapabilities {
	canStop: boolean;
	canResume: boolean;
	canAbort: boolean;
	canArchive: boolean;
}

export interface WorkspaceNode {
	kind: "node";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	errorReason?: string | null;
	contentKind: "session" | "command";
	capabilities: WorkspaceNodeCapabilities;
	updatedAt: number;
}

export interface WorkspaceWorkflow {
	kind: "workflow";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	capabilities: WorkspaceWorkflowCapabilities;
	children: WorkspaceTreeItem[];
	updatedAt: number;
}

export interface WorkspaceFanout {
	kind: "fanout";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	children: WorkspaceTreeItem[];
	updatedAt: number;
}

export type WorkspaceTreeItem =
	| WorkspaceNode
	| WorkspaceWorkflow
	| WorkspaceFanout;

export interface WorkspaceTreeSnapshot {
	nodes: WorkspaceTreeItem[];
	sessions?: AgentSessionItem[];
	preferredNodeId?: string | null;
}

export interface WorkspaceSelectionReconciliation {
	selectionInSnapshot: boolean;
}

export interface WorkspaceTreeSelectionSnapshot {
	snapshot: WorkspaceTreeSnapshot;
	reconciliation: WorkspaceSelectionReconciliation;
}

export interface WorkspaceAgentSessionNodeContent {
	kind: "agentSession";
	sessionId?: string | null;
}

export interface WorkspaceCommandResult {
	exitCode: number;
	duration: number;
	stdout: string;
	stderr: string;
}

export interface WorkspaceCommandNodeContent {
	kind: "command";
	displayCommand?: string | null;
	result?: WorkspaceCommandResult | null;
}

export type WorkspaceNodeContent =
	| WorkspaceAgentSessionNodeContent
	| WorkspaceCommandNodeContent;

export interface WorkspaceNodeDetail {
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	attempt?: number;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: "submit" | "stop";
	hasArtifact: boolean;
	errorReason?: string | null;
	recoveryReason?: string | null;
	capabilities: WorkspaceNodeCapabilities;
	updatedAt: number;
	content: WorkspaceNodeContent;
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
