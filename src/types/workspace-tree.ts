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
	| "waiting"
	| "aborted"
	| "completed";

export type WorkspaceNodeStatusClassification =
	| "active"
	| "attention"
	| "idle"
	| "unbound";

export interface WorkspaceNodeCapabilities {
	canRename: boolean;
	canApprove: boolean;
	canRetry: boolean;
	canResumeSession: boolean;
}

export interface WorkspaceWorkflowCapabilities {
	canAbort: boolean;
	canArchive: boolean;
}

export interface WorkspaceSessionCapabilities {
	sessionRef: string;
	canArchive: boolean;
	canDelete: boolean;
}

export interface WorkspaceNode {
	processPresence: "live" | "confirmed_absent" | "unknown";
	kind: "node";
	id: string;
	title: string;
	status: WorkspaceNodeStatusClassification;
	errorReason?: string | null;
	contentKind: "session" | "command";
	capabilities: WorkspaceNodeCapabilities;
	workflowCapabilities?: WorkspaceWorkflowCapabilities | null;
	sessionCapabilities?: WorkspaceSessionCapabilities | null;
	children?: WorkspaceTreeItem[];
	pastAttempts: WorkspaceNode[];
	pastAttemptsCollapsed: boolean;
	updatedAt: number;
}

export interface WorkspaceSequence {
	worktree?: { branch: string; path: string } | null;
	kind: "sequence";
	id: string;
	title: string;
	status: WorkspaceNodeStatusClassification;
	workflowCapabilities?: WorkspaceWorkflowCapabilities | null;
	children: WorkspaceTreeItem[];
	updatedAt: number;
}

export interface WorkspaceFanout {
	worktree?: { branch: string; path: string } | null;
	kind: "fanout";
	id: string;
	title: string;
	status: WorkspaceNodeStatusClassification;
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
	processPresence: "live" | "confirmed_absent" | "unknown";
	worktree?: { branch: string; path: string } | null;
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	statusClassification: WorkspaceNodeStatusClassification;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: "submit" | "stop";
	hasArtifact: boolean;
	errorReason?: string | null;
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
