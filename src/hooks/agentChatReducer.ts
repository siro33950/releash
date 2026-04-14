import type {
	ChatMessage,
	ChatSession,
	MessagePart,
	PermissionMode,
	PermissionRequest,
	SessionState,
	SessionSummary,
	TurnPhase,
} from "@/types/session";

export function resolvePermissionMode(mode: PermissionMode): PermissionMode {
	return mode === "plan" ? "default" : mode;
}

export interface AgentChatState {
	sessions: SessionSummary[];
	sessionOrder: string[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	turnPhases: Record<string, TurnPhase>;
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
			type: "SET_TURN_PHASE";
			sessionId: string;
			turnPhase: TurnPhase;
	  }
	| { type: "SET_ERROR"; error: string | null }
	| { type: "UPDATE_SESSION_STATE"; state: SessionState }
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
			type: "SET_STREAMING_MESSAGE";
			sessionId: string;
			messageId: string;
			parts: MessagePart[];
	  };

/**
 * Merge delta parts into existing parts.
 * - text/thinking: merge into the last existing part if same type and parentToolUseId
 * - permission: update existing part by request_id if found, otherwise append
 * - other types: always append
 */
export function mergeDeltaParts(
	existing: MessagePart[],
	delta: MessagePart[],
): MessagePart[] {
	if (delta.length === 0) return existing;
	const result = existing.slice();
	for (const part of delta) {
		if (part.type === "text" || part.type === "thinking") {
			const last = result.length > 0 ? result[result.length - 1] : undefined;
			if (
				last &&
				(last.type === "text" || last.type === "thinking") &&
				last.type === part.type &&
				last.parentToolUseId === part.parentToolUseId
			) {
				result[result.length - 1] = {
					...last,
					content: last.content + part.content,
				};
			} else {
				result.push(part);
			}
		} else if (part.type === "permission") {
			const idx = result.findIndex(
				(p) =>
					p.type === "permission" &&
					p.request.request_id === part.request.request_id,
			);
			if (idx !== -1) {
				result[idx] = part;
			} else {
				result.push(part);
			}
		} else if (part.type === "system_notification") {
			// Update existing notification in-place (compaction/hook completion)
			const idx = result.findIndex((p) => {
				if (p.type !== "system_notification") return false;
				if (p.notificationType !== part.notificationType) return false;
				// Hook: match by hookId
				if (part.notificationType === "hook" && part.hookId) {
					return p.hookId === part.hookId;
				}
				// Compaction: match by notificationType (only one active at a time)
				if (part.notificationType === "compaction") {
					return p.status === "in_progress";
				}
				return false;
			});
			if (idx !== -1) {
				result[idx] = part;
			} else {
				result.push(part);
			}
		} else {
			result.push(part);
		}
	}
	return result;
}

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
		case "SET_TURN_PHASE":
			return {
				...state,
				turnPhases: {
					...state.turnPhases,
					[action.sessionId]: action.turnPhase,
				},
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
		case "SET_PERMISSION_MODE":
			return { ...state, permissionMode: action.mode };
		case "SET_USER_PERMISSION_MODE":
			return {
				...state,
				userPermissionMode: action.mode,
				permissionMode: action.mode,
			};
		case "RESTORE_USER_PERMISSION_MODE": {
			return {
				...state,
				permissionMode: resolvePermissionMode(state.userPermissionMode),
			};
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
		case "SET_STREAMING_MESSAGE": {
			if (!state.activeSession || state.activeSession.id !== action.sessionId)
				return state;
			return updateMessageInSession(state, action.messageId, (m) => ({
				...m,
				parts: mergeDeltaParts(m.parts, action.parts),
			}));
		}
	}
}

export const INITIAL_STATE: AgentChatState = {
	sessions: [],
	sessionOrder: [],
	closedSessions: [],
	activeSession: null,
	turnPhases: {},
	error: null,
	permissionMode: "acceptEdits",
	userPermissionMode: "acceptEdits",
	pendingPermissions: {},
};
