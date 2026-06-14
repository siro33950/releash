import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	CodexGoal,
	CodexRuntimeStatus,
	MessagePart,
	ModelInfo,
	PermissionMode,
	PermissionRequest,
	QueuedAgentTurn,
	SessionState,
	SessionSummary,
	SlashCommand,
	TokenUsage,
	TurnPhase,
} from "@/types/session";

export interface AgentChatState {
	sessions: SessionSummary[];
	sessionOrder: string[];
	closedSessions: SessionSummary[];
	/**
	 * ChatSession データの単一の正典。`activeSession` / 旧 `viewedStepSession` を
	 * 並列フィールドで持つ二重管理を廃し、本フィールドに集約する。各 panel は
	 * 自身が選択している sessionId を局所的に持ち、本 store から参照する。
	 */
	sessionsById: Record<string, ChatSession>;
	/** Main panel の active session id。`sessionsById[activeSessionId]` で本体に到達。*/
	activeSessionId: string | null;
	turnPhases: Record<string, TurnPhase>;
	/**
	 * interrupt 要求を出してから turn が idle になるまでの楽観的フラグ。
	 * 停止ボタン押下を即座に UI へ反映し、連打を防ぐ。turnPhase が idle に
	 * 遷移した時点で自動クリアされる。
	 */
	interrupting: Record<string, boolean>;
	error: string | null;
	permissionMode: PermissionMode;
	pendingPermissions: Record<string, PermissionRequest>;
	pendingQueues: Record<string, QueuedAgentTurn[]>;
	latestTokenUsage: Record<string, TokenUsage | null>;
	codexGoals: Record<string, CodexGoal | null>;
	codexRuntimeStatuses: Record<string, CodexRuntimeStatus>;
	runtimeSlashCommands: Record<string, SlashCommand[]>;
	availableModels: ModelInfo[];
	availableModelsByBackend: Record<string, ModelInfo[]>;
	sessionModels: Record<string, string>;
	backends: BackendInfo[];
	selectedBackendId: string | null;
}

export type AgentChatAction =
	| { type: "SET_SESSIONS"; sessions: SessionSummary[] }
	| { type: "SET_CLOSED_SESSIONS"; sessions: SessionSummary[] }
	/** 完全な ChatSession を sessionsById に upsert する。読み込み・送信応答の反映で利用。*/
	| { type: "UPSERT_SESSION"; session: ChatSession }
	/** Main panel の active session id を変更する。*/
	| { type: "SET_ACTIVE_SESSION_ID"; sessionId: string | null }
	| { type: "ADD_MESSAGE"; sessionId: string; message: ChatMessage }
	| {
			type: "SET_TURN_PHASE";
			sessionId: string;
			turnPhase: TurnPhase;
	  }
	| { type: "SET_INTERRUPTING"; sessionId: string; value: boolean }
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
	| {
			type: "SET_PENDING_QUEUE";
			sessionId: string;
			queue: QueuedAgentTurn[];
	  }
	| {
			type: "SET_LATEST_TOKEN_USAGE";
			sessionId: string;
			usage: TokenUsage | null;
	  }
	| {
			type: "SET_CODEX_GOAL";
			sessionId: string;
			goal: CodexGoal | null;
	  }
	| {
			type: "SET_CODEX_RUNTIME_STATUS";
			sessionId: string;
			status: CodexRuntimeStatus;
	  }
	| {
			type: "SET_RUNTIME_SLASH_COMMANDS";
			sessionId: string;
			commands: SlashCommand[];
	  }
	| {
			type: "REMOVE_PENDING_QUEUE_ITEM";
			sessionId: string;
			queuedTurnId: string;
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
	| {
			type: "SET_SESSION_MODEL";
			sessionId: string;
			modelId: string;
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

function getActiveSession(state: AgentChatState): ChatSession | null {
	if (!state.activeSessionId) return null;
	return state.sessionsById[state.activeSessionId] ?? null;
}

function displayBackendId(state: AgentChatState): string | null {
	return getActiveSession(state)?.backendId ?? state.selectedBackendId;
}

function modelsForDisplayBackend(state: AgentChatState): ModelInfo[] {
	const backendId = displayBackendId(state);
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

function upsertSession(
	state: AgentChatState,
	session: ChatSession,
): AgentChatState {
	return {
		...state,
		sessionsById: {
			...state.sessionsById,
			[session.id]: session,
		},
		error: null,
	};
}

function updateSessionInStore(
	state: AgentChatState,
	sessionId: string,
	updater: (session: ChatSession) => ChatSession,
): AgentChatState {
	const existing = state.sessionsById[sessionId];
	if (!existing) return state;
	return {
		...state,
		sessionsById: {
			...state.sessionsById,
			[sessionId]: updater(existing),
		},
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
		case "UPSERT_SESSION":
			return upsertSession(state, action.session);
		case "SET_ACTIVE_SESSION_ID":
			return {
				...state,
				activeSessionId: action.sessionId,
				error: null,
			};
		case "ADD_MESSAGE": {
			return updateSessionInStore(state, action.sessionId, (s) =>
				appendMessage(s, action.message),
			);
		}
		case "SET_TURN_PHASE": {
			// idle に戻ったら interrupting 楽観フラグをクリアする。
			const nextInterrupting =
				action.turnPhase === "idle" && state.interrupting[action.sessionId]
					? (() => {
							const { [action.sessionId]: _drop, ...rest } =
								state.interrupting;
							return rest;
						})()
					: state.interrupting;
			return {
				...state,
				turnPhases: {
					...state.turnPhases,
					[action.sessionId]: action.turnPhase,
				},
				interrupting: nextInterrupting,
			};
		}
		case "SET_INTERRUPTING": {
			if (!action.value) {
				const { [action.sessionId]: _drop, ...rest } = state.interrupting;
				return { ...state, interrupting: rest };
			}
			return {
				...state,
				interrupting: { ...state.interrupting, [action.sessionId]: true },
			};
		}
		case "SET_ERROR":
			return { ...state, error: action.error };
		case "UPDATE_SESSION_STATE":
			return updateSessionInStore(state, action.sessionId, (s) => ({
				...s,
				state: action.state,
			}));
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
		case "SET_PENDING_QUEUE":
			return {
				...state,
				pendingQueues: {
					...state.pendingQueues,
					[action.sessionId]: action.queue,
				},
			};
		case "SET_LATEST_TOKEN_USAGE":
			return {
				...state,
				latestTokenUsage: {
					...state.latestTokenUsage,
					[action.sessionId]: action.usage,
				},
			};
		case "SET_CODEX_GOAL":
			return {
				...state,
				codexGoals: {
					...state.codexGoals,
					[action.sessionId]: action.goal,
				},
			};
		case "SET_CODEX_RUNTIME_STATUS":
			return {
				...state,
				codexRuntimeStatuses: {
					...state.codexRuntimeStatuses,
					[action.sessionId]: {
						...state.codexRuntimeStatuses[action.sessionId],
						...action.status,
					},
				},
			};
		case "SET_RUNTIME_SLASH_COMMANDS":
			return {
				...state,
				runtimeSlashCommands: {
					...state.runtimeSlashCommands,
					[action.sessionId]: action.commands,
				},
			};
		case "REMOVE_PENDING_QUEUE_ITEM": {
			const queue = state.pendingQueues[action.sessionId] ?? [];
			return {
				...state,
				pendingQueues: {
					...state.pendingQueues,
					[action.sessionId]: queue.filter(
						(item) => item.id !== action.queuedTurnId,
					),
				},
			};
		}
		case "REORDER_SESSIONS":
			return { ...state, sessionOrder: action.sessionOrder };
		case "SET_STREAMING_MESSAGE": {
			const existing = state.sessionsById[action.sessionId];
			if (!existing) return state;
			const updated = applyMessageUpdate(existing, action.messageId, (m) => ({
				...m,
				parts: action.parts,
			}));
			if (!updated) return state;
			return {
				...state,
				sessionsById: {
					...state.sessionsById,
					[action.sessionId]: updated,
				},
			};
		}
		case "SET_AVAILABLE_MODELS": {
			const backendId = action.backendId ?? displayBackendId(state);
			if (!backendId) return { ...state, availableModels: action.models };
			return withBackendModels(state, backendId, action.models);
		}
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
			const { [action.sessionId]: _int, ...restInterrupting } =
				state.interrupting;
			const { [action.sessionId]: _pp, ...restPendingPermissions } =
				state.pendingPermissions;
			const { [action.sessionId]: _pq, ...restPendingQueues } =
				state.pendingQueues;
			const { [action.sessionId]: _tu, ...restLatestTokenUsage } =
				state.latestTokenUsage;
			const { [action.sessionId]: _cg, ...restCodexGoals } = state.codexGoals;
			const { [action.sessionId]: _crs, ...restCodexRuntimeStatuses } =
				state.codexRuntimeStatuses;
			const { [action.sessionId]: _rsc, ...restRuntimeSlashCommands } =
				state.runtimeSlashCommands;
			const { [action.sessionId]: _sm, ...restSessionModels } =
				state.sessionModels;
			const { [action.sessionId]: _sb, ...restSessionsById } =
				state.sessionsById;
			return {
				...state,
				turnPhases: restTurnPhases,
			interrupting: restInterrupting,
				pendingPermissions: restPendingPermissions,
				pendingQueues: restPendingQueues,
				latestTokenUsage: restLatestTokenUsage,
				codexGoals: restCodexGoals,
				codexRuntimeStatuses: restCodexRuntimeStatuses,
				runtimeSlashCommands: restRuntimeSlashCommands,
				sessionModels: restSessionModels,
				sessionsById: restSessionsById,
				activeSessionId:
					state.activeSessionId === action.sessionId
						? null
						: state.activeSessionId,
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
			const activeSession = getActiveSession(state);
			const nextDisplayBackendId =
				activeSession?.backendId ?? (activeSession ? null : selectedBackendId);
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
					: activeSession
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
	sessionsById: {},
	activeSessionId: null,
	turnPhases: {},
	interrupting: {},
	error: null,
	permissionMode: "edit",
	pendingPermissions: {},
	pendingQueues: {},
	latestTokenUsage: {},
	codexGoals: {},
	codexRuntimeStatuses: {},
	runtimeSlashCommands: {},
	availableModels: [],
	availableModelsByBackend: {},
	sessionModels: {},
	backends: [],
	selectedBackendId: null,
};

/** sessionsById から指定 session を解決する selector。reducer 外から参照する経路で利用。*/
export function selectSessionFromState(
	state: AgentChatState,
	sessionId: string | null | undefined,
): ChatSession | null {
	if (!sessionId) return null;
	return state.sessionsById[sessionId] ?? null;
}

export function selectActiveSession(state: AgentChatState): ChatSession | null {
	return selectSessionFromState(state, state.activeSessionId);
}
