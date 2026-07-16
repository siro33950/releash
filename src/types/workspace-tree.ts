import type { SessionSummary } from "./session";
import type { WorkflowExecutionSummary } from "./workflow";

export interface CenterSelection {
	kind: "node";
	worktreePath: string;
	nodeId: string;
}

export interface NewSessionCreationRequest {
	requestId: string;
	worktreePath: string;
	attempt: number;
}

export interface NewSessionCreationStatus {
	pending: boolean;
	error: string | null;
}

export type WorkspaceNodeStatus =
	| "queued"
	| "running"
	| "failed"
	| "error"
	| "waiting"
	| "interrupted"
	| "aborted"
	| "completed";

export interface WorkspaceNodeCapabilities {
	canApprove: boolean;
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
	preferredNodeId?: string | null;
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

export type WorkspaceSessionHistoryItem = SessionSummary;
