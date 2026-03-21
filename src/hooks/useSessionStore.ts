import { invoke } from "@tauri-apps/api/core";
import type {
	ActivityEntry,
	ChatMessage,
	ChatSession,
	LegacyChatMessage,
	MessagePart,
	MessageRole,
	SessionState,
	SessionSummary,
} from "@/types/session";

interface LegacyChatSession {
	id: string;
	worktreePath: string;
	messages: LegacyChatMessage[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
}

export function legacyToParts(msg: LegacyChatMessage): MessagePart[] {
	const parts: MessagePart[] = [];
	if (msg.thinking) {
		parts.push({ type: "thinking", content: msg.thinking });
	}
	if (msg.activities) {
		for (const a of msg.activities) {
			if (a.type === "tool_use") {
				parts.push({
					type: "tool_use",
					tool: a.tool,
					input: a.input,
					id: a.id,
				});
			} else if (a.type === "tool_result") {
				parts.push({
					type: "tool_result",
					content: a.content,
					isError: a.isError,
				});
			} else if (a.type === "permission_result") {
				parts.push({
					type: "permission",
					request: {
						request_id: "",
						tool_name: a.toolName,
						input: {},
						tool_use_id: "",
					},
					status: a.status === "allowed" ? "allowed" : "denied",
				});
			}
		}
	}
	if (msg.content) {
		parts.push({ type: "text", content: msg.content });
	}
	return parts;
}

export function partsToLegacy(parts: MessagePart[]): {
	content: string;
	thinking: string | undefined;
	activities: ActivityEntry[] | undefined;
} {
	let content = "";
	let thinking = "";
	const activities: ActivityEntry[] = [];
	for (const part of parts) {
		switch (part.type) {
			case "text":
				content += part.content;
				break;
			case "thinking":
				thinking += part.content;
				break;
			case "tool_use":
				activities.push({
					type: "tool_use",
					tool: part.tool,
					input: part.input,
					id: part.id,
				});
				break;
			case "tool_result":
				activities.push({
					type: "tool_result",
					content: part.content,
					isError: part.isError,
				});
				break;
			case "permission":
				if (part.status !== "pending") {
					const summary = part.answers
						? Object.values(part.answers).join(", ")
						: part.status;
					activities.push({
						type: "permission_result",
						toolName: part.request.tool_name,
						status: part.status,
						summary,
					});
				}
				break;
		}
	}
	return {
		content,
		thinking: thinking || undefined,
		activities: activities.length > 0 ? activities : undefined,
	};
}

function convertLegacyMessage(msg: LegacyChatMessage): ChatMessage {
	return {
		id: msg.id,
		role: msg.role,
		parts: legacyToParts(msg),
		timestamp: msg.timestamp,
	};
}

function convertLegacySession(session: LegacyChatSession): ChatSession {
	return {
		...session,
		messages: session.messages.map(convertLegacyMessage),
	};
}

export async function listSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_sessions", { worktreePath });
}

export async function getSession(
	sessionId: string,
): Promise<ChatSession | null> {
	const raw = await invoke<LegacyChatSession | null>("get_session", {
		sessionId,
	});
	return raw ? convertLegacySession(raw) : null;
}

export async function createSession(
	worktreePath: string,
): Promise<ChatSession> {
	const raw = await invoke<LegacyChatSession>("create_session", {
		worktreePath,
	});
	return convertLegacySession(raw);
}

export async function closeSession(sessionId: string): Promise<void> {
	return invoke("close_session", { sessionId });
}

export async function restoreSession(sessionId: string): Promise<void> {
	return invoke("restore_session", { sessionId });
}

export async function listClosedSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_closed_sessions", { worktreePath });
}

export async function addMessage(
	sessionId: string,
	role: MessageRole,
	content: string,
): Promise<ChatMessage> {
	const raw = await invoke<LegacyChatMessage>("add_message", {
		sessionId,
		role,
		content,
	});
	return convertLegacyMessage(raw);
}

export async function updateSessionState(
	sessionId: string,
	newState: SessionState,
): Promise<void> {
	return invoke("update_session_state", { sessionId, newState });
}

export async function updateMessageContent(
	sessionId: string,
	messageId: string,
	content: string,
	thinking?: string,
	activities?: ActivityEntry[],
): Promise<void> {
	return invoke("update_message_content", {
		sessionId,
		messageId,
		content,
		thinking: thinking ?? null,
		activities: activities ?? null,
	});
}

export async function updateSessionAgentInfo(
	sessionId: string,
	agentSessionId: string | null,
): Promise<void> {
	return invoke("update_session_agent_info", {
		sessionId,
		agentSessionId,
	});
}
