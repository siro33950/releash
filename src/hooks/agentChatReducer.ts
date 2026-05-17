import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	MessagePart,
	ModelInfo,
	PermissionMode,
	PermissionRequest,
	SessionState,
	SessionSummary,
	TurnPhase,
} from "@/types/session";

export interface AgentChatState {
	sessions: SessionSummary[];
	sessionOrder: string[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	turnPhases: Record<string, TurnPhase>;
	error: string | null;
	permissionMode: PermissionMode;
	pendingPermissions: Record<string, PermissionRequest>;
	availableModels: ModelInfo[];
	availableModelsByBackend: Record<string, ModelInfo[]>;
	sessionModels: Record<string, string | null>;
	backends: BackendInfo[];
	selectedBackendId: string | null;
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
	  }
	| {
			type: "SET_AVAILABLE_MODELS";
			models: ModelInfo[];
			backendId?: string | null;
	  }
	| { type: "SET_BACKEND_MODELS"; backendId: string; models: ModelInfo[] }
	| {
			type: "SET_SESSION_MODEL";
			sessionId: string;
			modelId: string | null;
	  }
	| { type: "CLEANUP_SESSION"; sessionId: string }
	| { type: "SET_BACKENDS"; backends: BackendInfo[]; defaultId: string | null }
	| { type: "SET_SELECTED_BACKEND"; backendId: string | null };

// Rust now emits the cumulative `streaming_parts` array on every flush.
// `SET_STREAMING_MESSAGE` replaces the message's parts wholesale so that
// re-deliveries (e.g. when one transport channel previously failed) collapse
// to a single state update with no double-application risk.

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

function displayBackendId(state: AgentChatState): string | null {
	return state.activeSession?.backendId ?? state.selectedBackendId;
}

function modelsForDisplayBackend(
	state: Pick<
		AgentChatState,
		| "activeSession"
		| "selectedBackendId"
		| "availableModelsByBackend"
		| "availableModels"
	>,
): ModelInfo[] {
	const backendId = state.activeSession?.backendId ?? state.selectedBackendId;
	if (!backendId) return [];
	return backendId in state.availableModelsByBackend
		? state.availableModelsByBackend[backendId]
		: state.availableModels;
}

function withBackendModels(
	state: AgentChatState,
	backendId: string,
	models: ModelInfo[],
): AgentChatState {
	const availableModelsByBackend = {
		...state.availableModelsByBackend,
		[backendId]: models,
	};
	const nextState = { ...state, availableModelsByBackend };
	return {
		...nextState,
		availableModels: modelsForDisplayBackend(nextState),
	};
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
				parts: action.parts,
			}));
		}
		case "SET_AVAILABLE_MODELS": {
			const backendId = action.backendId ?? displayBackendId(state);
			if (!backendId) return { ...state, availableModels: action.models };
			return withBackendModels(state, backendId, action.models);
		}
		case "SET_BACKEND_MODELS":
			return withBackendModels(state, action.backendId, action.models);
		case "SET_SESSION_MODEL":
			return {
				...state,
				sessionModels: {
					...state.sessionModels,
					[action.sessionId]: action.modelId,
				},
			};
		case "CLEANUP_SESSION": {
			const { [action.sessionId]: _tp, ...restTurnPhases } = state.turnPhases;
			const { [action.sessionId]: _pp, ...restPendingPermissions } =
				state.pendingPermissions;
			const { [action.sessionId]: _sm, ...restSessionModels } =
				state.sessionModels;
			return {
				...state,
				turnPhases: restTurnPhases,
				pendingPermissions: restPendingPermissions,
				sessionModels: restSessionModels,
			};
		}
		case "SET_BACKENDS": {
			const availableModelsByBackend = action.backends.reduce<
				Record<string, ModelInfo[]>
			>(
				(acc, backend) => {
					acc[backend.id] = backend.availableModels;
					return acc;
				},
				{ ...state.availableModelsByBackend },
			);
			const selectedBackendId =
				state.selectedBackendId ??
				action.defaultId ??
				(action.backends.length > 0 ? action.backends[0].id : null);
			const nextDisplayBackendId =
				state.activeSession?.backendId ??
				(state.activeSession ? null : selectedBackendId);
			const nextState = {
				...state,
				backends: action.backends,
				selectedBackendId,
				availableModelsByBackend,
			};
			return {
				...nextState,
				availableModels: nextDisplayBackendId
					? (availableModelsByBackend[nextDisplayBackendId] ?? [])
					: state.activeSession
						? state.availableModels
						: [],
			};
		}
		case "SET_SELECTED_BACKEND": {
			const nextState = {
				...state,
				selectedBackendId: action.backendId,
			};
			return {
				...nextState,
				availableModels: modelsForDisplayBackend(nextState),
			};
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
	pendingPermissions: {},
	availableModels: [],
	availableModelsByBackend: {},
	sessionModels: {},
	backends: [],
	selectedBackendId: null,
};
