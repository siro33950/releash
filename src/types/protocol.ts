import type { WorkflowStatePayload } from "@/types/workflow";
import type { ContextCarryState, MessagePart } from "./session";

export interface PtyExitMsg {
	pty_id: number;
	exit_code: number | null;
}

export interface WorktreePrEntry {
	path: string;
	pr_number: number;
	pr_url: string;
}

export interface WorktreePrStatusSync {
	entries: WorktreePrEntry[];
}

interface BranchCardMsg {
	name: string;
	is_main_worktree: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
}

export interface BranchListSync {
	branches: BranchCardMsg[];
}

export type AgentState = "running" | "done" | "error" | "waiting";

export interface AgentStateSync {
	worktree_path: string;
	state: AgentState;
	exit_code: number | null;
	timestamp: number;
	session_id: string | null;
	pty_id?: string | null;
}

export interface AgentSupportedCommandMsg {
	name: string;
	description: string;
	argumentHint?: string;
}

export interface AgentSupportedCommandsUpdated {
	chat_session_id: string;
	commands: AgentSupportedCommandMsg[];
}

export interface AgentSessionContextCarryUpdated {
	chat_session_id: string;
	agent_session_id?: string | null;
	context_carry?: ContextCarryState | null;
	updated_at: number;
}

export interface AgentStreamSync {
	session_id: string;
	message_id: string;
	parts: MessagePart[];
}

export interface WorkflowStateSync {
	worktree_path: string;
	workflow_state: WorkflowStatePayload;
}

export type ReviewActorKind = "human" | "agent";
export type ReviewThreadState = "open" | "resolved";
export type ReviewErrorCode =
	| "invalid_input"
	| "not_found"
	| "already_resolved"
	| "permission_denied"
	| "io"
	| "serialize";

export interface ReviewActor {
	kind: ReviewActorKind;
	backendId?: string | null;
	model?: string | null;
	displayName: string;
}

export interface ReviewTarget {
	filePath?: string | null;
	lineNumber?: number | null;
	endLine?: number | null;
}

export interface ReviewComment {
	id: string;
	threadId: string;
	author: ReviewActor;
	content: string;
	createdAt: number;
}

export interface ReviewResolveInfo {
	actor: ReviewActor;
	outcome: string;
	summary: string;
	resolvedAt: number;
}

export interface ReviewThread {
	id: string;
	worktreeName: string;
	author: ReviewActor;
	target: ReviewTarget;
	state: ReviewThreadState;
	comments: ReviewComment[];
	resolve?: ReviewResolveInfo | null;
	createdAt: number;
	updatedAt: number;
	version: number;
	canResolve: boolean;
}

export type AuthorScope = "mine" | "other";

export interface ReviewThreadFilter {
	file?: string | null;
	state?: ReviewThreadState | null;
	author?: AuthorScope | null;
	unread?: boolean | null;
	threadId?: string[];
}

export interface ReviewErrorPayload {
	code: ReviewErrorCode;
	message: string;
}

export type ReviewHistoryEntry =
	| {
			kind: "thread_created";
			id: string;
			threadId: string;
			commentId: string;
			actor: ReviewActor;
			target: ReviewTarget;
			content: string;
			at: number;
	  }
	| {
			kind: "comment_appended";
			id: string;
			threadId: string;
			commentId: string;
			actor: ReviewActor;
			content: string;
			at: number;
	  }
	| {
			kind: "thread_resolved";
			id: string;
			threadId: string;
			actor: ReviewActor;
			outcome: string;
			summary: string;
			at: number;
	  };

export interface ErrorMsg {
	code: string;
	message: string;
}

export type WsMessage =
	| { type: "auth_challenge"; payload: { challenge: string } }
	| { type: "auth_response"; payload: { hmac: string } }
	| { type: "auth_result"; payload: { success: boolean; message?: string } }
	| { type: "pty_output"; payload: { pty_id: number; data: string } }
	| { type: "pty_exit"; payload: PtyExitMsg }
	| { type: "worktree_pr_status_sync"; payload: WorktreePrStatusSync }
	| { type: "branch_list_sync"; payload: BranchListSync }
	| { type: "agent_state_sync"; payload: AgentStateSync }
	| { type: "workflow_state_sync"; payload: WorkflowStateSync }
	| { type: "agent_stream_sync"; payload: AgentStreamSync }
	| { type: "error"; payload: ErrorMsg };
