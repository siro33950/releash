import type {
	ChatMessage,
	ChatSession,
	MessagePart,
	PermissionMode,
	PermissionRequest,
	SessionState,
	SessionSummary,
} from "@/types/session";

export interface AgentChatState {
	sessions: SessionSummary[];
	sessionOrder: string[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	streamingSessionIds: string[];
	sessionFinalStates: Record<string, "done" | "error">;
	error: string | null;
	permissionMode: PermissionMode;
	userPermissionMode: PermissionMode;
	pendingPermissions: Record<string, PermissionRequest>;
}

export type AgentChatAction =
	| { type: "SET_SESSIONS"; sessions: SessionSummary[] }
	| { type: "SET_CLOSED_SESSIONS"; sessions: SessionSummary[] }
	| { type: "SET_ACTIVE_SESSION"; session: ChatSession | null }
	| { type: "ADD_MESSAGE"; message: ChatMessage }
	| {
			type: "APPEND_STREAMING";
			messageId: string;
			chunk: string;
			parentToolUseId?: string;
	  }
	| {
			type: "APPEND_THINKING";
			messageId: string;
			chunk: string;
			parentToolUseId?: string;
	  }
	| { type: "START_STREAMING"; sessionId: string }
	| { type: "STOP_STREAMING"; sessionId: string }
	| { type: "SET_ERROR"; error: string | null }
	| { type: "UPDATE_SESSION_STATE"; state: SessionState }
	| { type: "SET_AGENT_SESSION_ID"; agentSessionId: string | null }
	| {
			type: "APPEND_TOOL_USE";
			messageId: string;
			tool: string;
			input: Record<string, unknown>;
			id: string;
			parentToolUseId?: string;
	  }
	| {
			type: "APPEND_TOOL_RESULT";
			messageId: string;
			content: string;
			isError: boolean;
			toolUseId?: string;
			parentToolUseId?: string;
	  }
	| {
			type: "APPEND_TASK_STATUS";
			messageId: string;
			taskToolUseId: string;
			status: "started" | "completed" | "failed" | "stopped" | "progress";
			description?: string;
			summary?: string;
	  }
	| { type: "SET_PERMISSION_MODE"; mode: PermissionMode }
	| { type: "SET_USER_PERMISSION_MODE"; mode: PermissionMode }
	| { type: "RESTORE_USER_PERMISSION_MODE" }
	| {
			type: "SET_PENDING_PERMISSION";
			sessionId: string;
			request: PermissionRequest | null;
	  }
	| { type: "REORDER_SESSIONS"; sessionOrder: string[] }
	| {
			type: "SET_SESSION_FINAL_STATE";
			sessionId: string;
			state: "done" | "error";
	  }
	| {
			type: "ADD_PERMISSION_PART";
			messageId: string;
			request: PermissionRequest;
	  }
	| {
			type: "RESOLVE_PERMISSION_PART";
			messageId: string;
			requestId: string;
			status: "allowed" | "denied";
			answers?: Record<string, string>;
	  };

function updateMessageInSession(
	state: AgentChatState,
	messageId: string,
	updater: (msg: ChatMessage) => ChatMessage,
): AgentChatState {
	if (!state.activeSession) return state;
	const msgs = state.activeSession.messages;
	const lastIdx = msgs.length - 1;
	// Streaming target is almost always the last message — check it first
	const idx =
		lastIdx >= 0 && msgs[lastIdx].id === messageId
			? lastIdx
			: msgs.findIndex((m) => m.id === messageId);
	if (idx === -1) return state;
	const messages = msgs.slice();
	messages[idx] = updater(msgs[idx]);
	return { ...state, activeSession: { ...state.activeSession, messages } };
}

export function appendToParts(
	parts: MessagePart[],
	partType: "thinking" | "text",
	chunk: string,
	parentToolUseId?: string,
): MessagePart[] {
	const last = parts[parts.length - 1];
	if (last && last.type === partType) {
		const lastPid =
			"parentToolUseId" in last ? last.parentToolUseId : undefined;
		if (lastPid === parentToolUseId) {
			const updated = { ...last, content: last.content + chunk };
			return [...parts.slice(0, -1), updated];
		}
	}
	return [
		...parts,
		{ type: partType, content: chunk, parentToolUseId } as MessagePart,
	];
}

export function reducer(
	state: AgentChatState,
	action: AgentChatAction,
): AgentChatState {
	switch (action.type) {
		case "SET_SESSIONS": {
			const newIds = new Set(action.sessions.map((s) => s.id));
			const kept = state.sessionOrder.filter((id) => newIds.has(id));
			const added = action.sessions
				.filter((s) => !kept.includes(s.id))
				.map((s) => s.id);
			return {
				...state,
				sessions: action.sessions,
				sessionOrder: [...kept, ...added],
			};
		}
		case "SET_CLOSED_SESSIONS":
			return { ...state, closedSessions: action.sessions };
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
				parts: appendToParts(
					m.parts,
					"text",
					action.chunk,
					action.parentToolUseId,
				),
			}));
		case "APPEND_THINKING":
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: appendToParts(
					m.parts,
					"thinking",
					action.chunk,
					action.parentToolUseId,
				),
			}));
		case "START_STREAMING": {
			const { [action.sessionId]: _, ...restFinal } = state.sessionFinalStates;
			return {
				...state,
				streamingSessionIds: state.streamingSessionIds.includes(
					action.sessionId,
				)
					? state.streamingSessionIds
					: [...state.streamingSessionIds, action.sessionId],
				sessionFinalStates: restFinal,
			};
		}
		case "STOP_STREAMING":
			return {
				...state,
				streamingSessionIds: state.streamingSessionIds.filter(
					(id) => id !== action.sessionId,
				),
			};
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
			const part: MessagePart = {
				type: "tool_use",
				tool: action.tool,
				input: action.input,
				id: action.id,
				...(action.parentToolUseId && {
					parentToolUseId: action.parentToolUseId,
				}),
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: [...m.parts, part],
			}));
		}
		case "APPEND_TOOL_RESULT": {
			const part: MessagePart = {
				type: "tool_result",
				content: action.content,
				isError: action.isError,
				...(action.toolUseId && { toolUseId: action.toolUseId }),
				...(action.parentToolUseId && {
					parentToolUseId: action.parentToolUseId,
				}),
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: [...m.parts, part],
			}));
		}
		case "APPEND_TASK_STATUS": {
			const part: MessagePart = {
				type: "task_status",
				taskToolUseId: action.taskToolUseId,
				status: action.status,
				...(action.description && { description: action.description }),
				...(action.summary && { summary: action.summary }),
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: [...m.parts, part],
			}));
		}
		case "ADD_PERMISSION_PART": {
			const part: MessagePart = {
				type: "permission",
				request: action.request,
				status: "pending",
			};
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: [...m.parts, part],
			}));
		}
		case "RESOLVE_PERMISSION_PART":
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: m.parts.map((p) =>
					p.type === "permission" && p.request.request_id === action.requestId
						? { ...p, status: action.status, answers: action.answers }
						: p,
				),
			}));
		case "SET_PERMISSION_MODE":
			return { ...state, permissionMode: action.mode };
		case "SET_USER_PERMISSION_MODE":
			return {
				...state,
				userPermissionMode: action.mode,
				permissionMode: action.mode,
			};
		case "RESTORE_USER_PERMISSION_MODE": {
			const restored =
				state.userPermissionMode === "plan"
					? "default"
					: state.userPermissionMode;
			return { ...state, permissionMode: restored };
		}
		case "SET_PENDING_PERMISSION": {
			if (action.request === null) {
				const { [action.sessionId]: _, ...rest } = state.pendingPermissions;
				return { ...state, pendingPermissions: rest };
			}
			return {
				...state,
				pendingPermissions: {
					...state.pendingPermissions,
					[action.sessionId]: action.request,
				},
			};
		}
		case "REORDER_SESSIONS":
			return { ...state, sessionOrder: action.sessionOrder };
		case "SET_SESSION_FINAL_STATE":
			return {
				...state,
				sessionFinalStates: {
					...state.sessionFinalStates,
					[action.sessionId]: action.state,
				},
			};
	}
}

export const INITIAL_STATE: AgentChatState = {
	sessions: [],
	sessionOrder: [],
	closedSessions: [],
	activeSession: null,
	streamingSessionIds: [],
	sessionFinalStates: {},
	error: null,
	permissionMode: "acceptEdits",
	userPermissionMode: "acceptEdits",
	pendingPermissions: {},
};
