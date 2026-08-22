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
			initialSessionAttachment?: AgentSessionLaunchAttachment;
	  }
	| {
			kind: "agent_session_launching";
			worktreePath: string;
			provider: string;
			launchToken: string;
			error?: string;
	  };

export type WorkspaceNodeStatus =
	| "running"
	| "paused"
	| "failed"
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
	resumeUnavailableReason?: string | null;
}

export interface WorkspaceSessionCapabilities {
	sessionRef: string;
	canArchive: boolean;
	canDelete: boolean;
}

export interface WorkspaceNode {
	kind: "node";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	errorReason?: string | null;
	contentKind: "session" | "command";
	capabilities: WorkspaceNodeCapabilities;
	workflowCapabilities?: WorkspaceWorkflowCapabilities | null;
	sessionCapabilities?: WorkspaceSessionCapabilities | null;
	pastAttempts: WorkspaceNode[];
	pastAttemptsCollapsed: boolean;
	updatedAt: number;
}

export interface WorkspaceSequence {
	kind: "sequence";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	workflowCapabilities?: WorkspaceWorkflowCapabilities | null;
	children: WorkspaceTreeItem[];
	updatedAt: number;
}

export interface WorkspaceFanout {
	kind: "fanout";
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	workflowCapabilities?: WorkspaceWorkflowCapabilities | null;
	children: WorkspaceTreeItem[];
	updatedAt: number;
}

export type WorkspaceTreeItem =
	| WorkspaceNode
	| WorkspaceSequence
	| WorkspaceFanout;

export interface WorkspaceTreeSnapshot {
	nodes: WorkspaceTreeItem[];
	archivedSessions: AgentSessionItem[];
	preferredNodeId?: string | null;
}

export interface WorkspaceSelectionReconciliation {
	selectionInSnapshot: boolean;
}

export interface WorkspaceTreeSelectionSnapshot {
	snapshot: WorkspaceTreeSnapshot;
	reconciliation: WorkspaceSelectionReconciliation;
}

export interface WorkspaceSessionNodeContent {
	kind: "session";
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
	| WorkspaceSessionNodeContent
	| WorkspaceCommandNodeContent;

export interface WorkspaceNodeDetail {
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
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
