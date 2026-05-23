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
	/**
	 * spec issues-1023: Workflow panel から「現在表示中の workflow step session」を
	 * activeSession とは独立に観測するための ChatSession slot。tab bar からは見えない
	 * workflow step session の本文・streaming 反映を保持する。free chat 側の
	 * activeSession には影響しない。
	 */
	viewedStepSession: ChatSession | null;
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
	| { type: "SET_VIEWED_STEP_SESSION"; session: ChatSession | null }
	| { type: "ADD_MESSAGE"; sessionId: string; message: ChatMessage }
	| {
			type: "SET_TURN_PHASE";
			sessionId: string;
			turnPhase: TurnPhase;
	  }
	| { type: "SET_ERROR"; error: string | null }
	| {
			type: "UPDATE_SESSION_STATE";
			sessionId: string;
			state: SessionState;
	  }
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

function applyMessageUpdate(
	session: ChatSession,
	messageId: string,
	updater: (msg: ChatMessage) => ChatMessage,
): ChatSession | null {
	const msgs = session.messages;
	const lastIdx = msgs.length - 1;
	// Streaming target is almost always the last message — check it first
	const idx =
		lastIdx >= 0 && msgs[lastIdx].id === messageId
			? lastIdx
			: msgs.findIndex((m) => m.id === messageId);
	if (idx === -1) return null;
	const messages = msgs.slice();
	messages[idx] = updater(msgs[idx]);
	return { ...session, messages };
}

function appendMessage(
	session: ChatSession,
	message: ChatMessage,
): ChatSession {
	return { ...session, messages: [...session.messages, message] };
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
		case "SET_VIEWED_STEP_SESSION":
			return { ...state, viewedStepSession: action.session };
		case "ADD_MESSAGE": {
			let next = state;
			if (state.activeSession?.id === action.sessionId) {
				next = {
					...next,
					activeSession: appendMessage(state.activeSession, action.message),
				};
			}
			if (state.viewedStepSession?.id === action.sessionId) {
				next = {
					...next,
					viewedStepSession: appendMessage(
						state.viewedStepSession,
						action.message,
					),
				};
			}
			return next;
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
			let next = state;
			if (state.activeSession?.id === action.sessionId) {
				next = {
					...next,
					activeSession: { ...state.activeSession, state: action.state },
				};
			}
			if (state.viewedStepSession?.id === action.sessionId) {
				next = {
					...next,
					viewedStepSession: {
						...state.viewedStepSession,
						state: action.state,
					},
				};
			}
			return next;
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
			let next = state;
			if (state.activeSession?.id === action.sessionId) {
				const updated = applyMessageUpdate(
					state.activeSession,
					action.messageId,
					(m) => ({ ...m, parts: action.parts }),
				);
				if (updated) next = { ...next, activeSession: updated };
			}
			if (state.viewedStepSession?.id === action.sessionId) {
				const updated = applyMessageUpdate(
					state.viewedStepSession,
					action.messageId,
					(m) => ({ ...m, parts: action.parts }),
				);
				if (updated) next = { ...next, viewedStepSession: updated };
			}
			return next;
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
				viewedStepSession:
					state.viewedStepSession?.id === action.sessionId
						? null
						: state.viewedStepSession,
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
	viewedStepSession: null,
	turnPhases: {},
	error: null,
	permissionMode: "edit",
	pendingPermissions: {},
	availableModels: [],
	availableModelsByBackend: {},
	sessionModels: {},
	backends: [],
	selectedBackendId: null,
};
