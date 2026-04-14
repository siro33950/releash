export type PermissionMode =
	| "acceptEdits"
	| "default"
	| "plan"
	| "bypassPermissions";

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
}

/** Rust backend ChatMessage format (for DB persistence) */
export interface LegacyChatMessage {
	id: string;
	role: MessageRole;
	content: string;
	thinking?: string;
	activities?: ActivityEntry[];
	timestamp: number;
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
}
