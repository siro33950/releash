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

export type MessagePart =
	| { type: "thinking"; content: string }
	| { type: "text"; content: string }
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
	  }
	| { type: "tool_result"; content: string; isError: boolean }
	| {
			type: "permission";
			request: PermissionRequest;
			status: "pending" | "allowed" | "denied";
			answers?: Record<string, string>;
	  };

export type ActivityEntry =
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
	  }
	| { type: "tool_result"; content: string; isError: boolean }
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
}
