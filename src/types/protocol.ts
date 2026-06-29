import type { ContextCarryState } from "./session";

export type AgentState = "running" | "done" | "error" | "waiting";

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
