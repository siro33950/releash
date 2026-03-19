import type {
	ActivityEntry,
	ChatMessage,
	ChatSession,
	PermissionMode,
	PermissionRequest,
	SessionState,
	SessionSummary,
} from "@/types/session";

export interface AgentChatState {
	sessions: SessionSummary[];
	activeSession: ChatSession | null;
	isStreaming: boolean;
	error: string | null;
	permissionMode: PermissionMode;
	userPermissionMode: PermissionMode;
	pendingPermission: PermissionRequest | null;
}

export type AgentChatAction =
	| { type: "SET_SESSIONS"; sessions: SessionSummary[] }
	| { type: "SET_ACTIVE_SESSION"; session: ChatSession | null }
	| { type: "ADD_MESSAGE"; message: ChatMessage }
	| { type: "APPEND_STREAMING"; messageId: string; chunk: string }
	| { type: "APPEND_THINKING"; messageId: string; chunk: string }
	| { type: "SET_STREAMING"; streaming: boolean }
	| { type: "SET_ERROR"; error: string | null }
	| { type: "UPDATE_SESSION_STATE"; state: SessionState }
	| { type: "SET_AGENT_SESSION_ID"; agentSessionId: string | null }
	| {
			type: "APPEND_TOOL_USE";
			messageId: string;
			tool: string;
			input: Record<string, unknown>;
			id: string;
	  }
	| {
			type: "APPEND_TOOL_RESULT";
			messageId: string;
			content: string;
			isError: boolean;
	  }
	| { type: "SET_PERMISSION_MODE"; mode: PermissionMode }
	| { type: "SET_USER_PERMISSION_MODE"; mode: PermissionMode }
	| { type: "RESTORE_USER_PERMISSION_MODE" }
	| { type: "SET_PENDING_PERMISSION"; request: PermissionRequest | null };

function updateMessageInSession(
	state: AgentChatState,
	messageId: string,
	updater: (msg: ChatMessage) => ChatMessage,
): AgentChatState {
	if (!state.activeSession) return state;
	const messages = state.activeSession.messages.map((m) =>
		m.id === messageId ? updater(m) : m,
	);
	return { ...state, activeSession: { ...state.activeSession, messages } };
}

export function reducer(
	state: AgentChatState,
	action: AgentChatAction,
): AgentChatState {
	switch (action.type) {
		case "SET_SESSIONS":
			return { ...state, sessions: action.sessions };
		case "SET_ACTIVE_SESSION":
			return { ...state, activeSession: action.session, error: null };
		case "ADD_MESSAGE": {
			if (!state.activeSession) return state;
			return {
				...state,
				activeSession: {
					...state.activeSession,
					messages: [...state.activeSession.messages, action.message],
				},
			};
		}
		case "APPEND_STREAMING":
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				content: m.content + action.chunk,
			}));
		case "APPEND_THINKING":
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				thinking: (m.thinking ?? "") + action.chunk,
			}));
		case "SET_STREAMING":
			return { ...state, isStreaming: action.streaming };
		case "SET_ERROR":
			return { ...state, error: action.error };
		case "UPDATE_SESSION_STATE": {
			if (!state.activeSession) return state;
			return {
				...state,
				activeSession: { ...state.activeSession, state: action.state },
			};
		}
		case "SET_AGENT_SESSION_ID": {
			if (!state.activeSession) return state;
			return {
				...state,
				activeSession: {
					...state.activeSession,
					agentSessionId: action.agentSessionId,
				},
			};
		}
		case "APPEND_TOOL_USE": {
			const entry: ActivityEntry = {
				type: "tool_use",
				tool: action.tool,
				input: action.input,
				id: action.id,
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				activities: [...(m.activities ?? []), entry],
			}));
		}
		case "APPEND_TOOL_RESULT": {
			const entry: ActivityEntry = {
				type: "tool_result",
				content: action.content,
				isError: action.isError,
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				activities: [...(m.activities ?? []), entry],
			}));
		}
		case "SET_PERMISSION_MODE":
			return { ...state, permissionMode: action.mode };
		case "SET_USER_PERMISSION_MODE":
			return { ...state, userPermissionMode: action.mode, permissionMode: action.mode };
		case "RESTORE_USER_PERMISSION_MODE": {
			const restored = state.userPermissionMode === "plan" ? "default" : state.userPermissionMode;
			return { ...state, permissionMode: restored };
		}
		case "SET_PENDING_PERMISSION":
			return { ...state, pendingPermission: action.request };
	}
}

export const INITIAL_STATE: AgentChatState = {
	sessions: [],
	activeSession: null,
	isStreaming: false,
	error: null,
	permissionMode: "acceptEdits",
	userPermissionMode: "acceptEdits",
	pendingPermission: null,
};
