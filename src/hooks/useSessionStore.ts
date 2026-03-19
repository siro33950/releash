import { invoke } from "@tauri-apps/api/core";
import type {
	ActivityEntry,
	ChatMessage,
	ChatSession,
	MessageRole,
	SessionState,
	SessionSummary,
} from "@/types/session";

export async function listSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_sessions", { worktreePath });
}

export async function getSession(
	sessionId: string,
): Promise<ChatSession | null> {
	return invoke<ChatSession | null>("get_session", { sessionId });
}

export async function createSession(
	worktreePath: string,
): Promise<ChatSession> {
	return invoke<ChatSession>("create_session", { worktreePath });
}

export async function addMessage(
	sessionId: string,
	role: MessageRole,
	content: string,
): Promise<ChatMessage> {
	return invoke<ChatMessage>("add_message", { sessionId, role, content });
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
