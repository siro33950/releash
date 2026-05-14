import type { AgentState } from "./protocol";

export type PermissionMode =
	| "acceptEdits"
	| "default"
	| "plan"
	| "bypassPermissions";

export interface ModelInfo {
	value: string;
	displayName: string;
}

export interface PermissionRequest {
	request_id: string;
	tool_name: string;
	input: Record<string, unknown>;
	tool_use_id: string;
	title?: string;
	display_name?: string;
	description?: string;
	decision_reason?: string;
}

export type MessageRole = "human" | "agent" | "system";

export type SessionState = "active" | "idle" | "done" | "error" | "closed";

export type TurnPhase = "idle" | "streaming" | "waiting_permission";

export type MessagePart =
	| { type: "thinking"; content: string; parentToolUseId?: string }
	| { type: "text"; content: string; parentToolUseId?: string }
	| { type: "error"; content: string; parentToolUseId?: string }
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
			parentToolUseId?: string;
	  }
	| {
			type: "tool_result";
			content: string;
			isError: boolean;
			toolUseId?: string;
			parentToolUseId?: string;
	  }
	| {
			type: "permission";
			request: PermissionRequest;
			status: "pending" | "allowed" | "denied";
			answers?: Record<string, string>;
			parentToolUseId?: string;
	  }
	| {
			type: "task_status";
			taskToolUseId: string;
			status: "started" | "completed" | "failed" | "stopped" | "progress";
			description?: string;
			summary?: string;
	  }
	| {
			type: "system_notification";
			notificationType:
				| "compaction"
				| "hook"
				| "files_persisted"
				| "local_command_output";
			status: "in_progress" | "completed" | "error";
			label: string;
			detail?: string;
			hookId?: string;
	  }
	| {
			type: "image";
			data: string;
			mediaType: string;
	  };

export type ActivityEntry =
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
	  }
	| {
			type: "tool_result";
			content: string;
			isError: boolean;
			toolUseId?: string;
	  }
	| {
			type: "permission_result";
			toolName: string;
			status: string;
			summary: string;
	  };

export interface ChatMessage {
	id: string;
	role: MessageRole;
	parts: MessagePart[];
	timestamp: number;
	mentions?: MentionReference[];
}

/** Rust backend ChatMessage format (for DB persistence) */
export interface LegacyChatMessage {
	id: string;
	role: MessageRole;
	content: string;
	thinking?: string;
	activities?: ActivityEntry[];
	timestamp: number;
	mentions?: MentionReference[];
}

export interface ChatSession {
	id: string;
	worktreePath: string;
	messages: ChatMessage[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	permissionMode: PermissionMode;
	backendId?: string | null;
	workflowStepSession?: boolean;
}

export function getTextContent(parts: MessagePart[]): string {
	return parts
		.filter((p): p is { type: "text"; content: string } => p.type === "text")
		.map((p) => p.content)
		.join("");
}

export interface SessionSummary {
	id: string;
	worktreePath: string;
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	firstMessage: string;
	messageCount: number;
	agentSessionId?: string | null;
	permissionMode: PermissionMode;
	backendId?: string | null;
	workflowStepSession?: boolean;
}

export interface BackendInfo {
	id: string;
	name: string;
	available: boolean;
}

/**
 * Rust の `agent_status::SessionStatus` に対応するステータス。
 * ChatSession 単位で Rust が算出する派生ステータスをそのまま消費する。
 */
export interface SessionStatus {
	chat_session_id: string;
	worktree_id: string;
	worktree_path: string;
	pty_id: string | null;
	agent_state: AgentState;
	turn_phase: TurnPhase;
	session_state: SessionState;
	pending_permission: boolean;
	last_activity_at: number;
}

/**
 * Rust の `agent_status::WorkspaceStatus` に対応する集約ステータス。
 * 1 worktree 配下の全 SessionStatus を集約した結果。
 */
export interface WorkspaceStatus {
	worktree_id: string;
	worktree_path: string;
	aggregated_state: AgentState;
	running_count: number;
	waiting_count: number;
	error_count: number;
	session_count: number;
	last_activity_at: number;
}

export type ImagePart = Extract<MessagePart, { type: "image" }>;

export interface ImageAttachment {
	data: string;
	mediaType: string;
}

export interface MentionReference {
	filePath: string;
	startLine?: number;
	endLine?: number;
}
