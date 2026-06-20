import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	ContextCarryState,
	MessagePart,
	ModelInfo,
	PermissionMode,
	PermissionRequest,
	PlanMode,
	QueuedAgentTurn,
	SessionState,
	SessionSummary,
	SlashCommand,
	TokenUsage,
	TurnPhase,
} from "@/types/session";
import { getModelInfoBackend } from "@/types/session";

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
	planMode: PlanMode;
	pendingPermissions: Record<string, PermissionRequest>;
	pendingQueues: Record<string, QueuedAgentTurn[]>;
	latestTokenUsage: Record<string, TokenUsage | null>;
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
	| {
			type: "SET_CONTEXT_CARRY";
			sessionId: string;
			contextCarry: ContextCarryState | null;
			agentSessionId: string | null;
			updatedAt: number | null;
	  }
	| { type: "SET_PERMISSION_MODE"; mode: PermissionMode }
	| { type: "SET_PLAN_MODE"; enabled: PlanMode }
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
			type: "MARK_AGENT_TURN_COMPLETED";
			sessionId: string;
			completedAt: number;
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
			backendId?: string | null;
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

function allAvailableModels(
	availableModelsByBackend: Record<string, ModelInfo[]>,
): ModelInfo[] {
	return Object.values(availableModelsByBackend).flat();
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
		availableModels: allAvailableModels(availableModelsByBackend),
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

function applyContextCarryToSummary(
	summary: SessionSummary,
	action: Extract<AgentChatAction, { type: "SET_CONTEXT_CARRY" }>,
): SessionSummary {
	if (summary.id !== action.sessionId) return summary;
	return {
		...summary,
		agentSessionId: action.agentSessionId,
		contextCarry: action.contextCarry,
		updatedAt: action.updatedAt ?? summary.updatedAt,
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
							const { [action.sessionId]: _drop, ...rest } = state.interrupting;
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
		case "SET_CONTEXT_CARRY": {
			const nextState = updateSessionInStore(state, action.sessionId, (s) => ({
				...s,
				agentSessionId: action.agentSessionId,
				contextCarry: action.contextCarry,
				updatedAt: action.updatedAt ?? s.updatedAt,
			}));
			return {
				...nextState,
				sessions: nextState.sessions.map((summary) =>
					applyContextCarryToSummary(summary, action),
				),
				closedSessions: nextState.closedSessions.map((summary) =>
					applyContextCarryToSummary(summary, action),
				),
			};
		}
		case "SET_PERMISSION_MODE":
			return { ...state, permissionMode: action.mode };
		case "SET_PLAN_MODE":
			return { ...state, planMode: action.enabled };
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
		case "MARK_AGENT_TURN_COMPLETED": {
			const existing = state.sessionsById[action.sessionId];
			if (!existing) return state;
			let targetIndex = -1;
			for (let i = existing.messages.length - 1; i >= 0; i--) {
				if (existing.messages[i]?.role === "agent") {
					targetIndex = i;
					break;
				}
			}
			if (targetIndex === -1) return state;
			const targetMessage = existing.messages[targetIndex];
			if (!targetMessage) return state;
			const messages = existing.messages.slice();
			messages[targetIndex] = {
				...targetMessage,
				timestamp: action.completedAt,
			};
			return {
				...state,
				sessionsById: {
					...state.sessionsById,
					[action.sessionId]: { ...existing, messages },
				},
			};
		}
		case "SET_AVAILABLE_MODELS": {
			const models = action.models ?? [];
			const backendId =
				action.backendId ??
				models.find((model) => getModelInfoBackend(model))?.backend ??
				displayBackendId(state);
			if (!backendId) return { ...state, availableModels: models };
			return withBackendModels(state, backendId, models);
		}
		case "SET_SESSION_MODEL": {
			const nextState = {
				...state,
				sessionModels: {
					...state.sessionModels,
					[action.sessionId]: action.modelId,
				},
			};
			if (action.backendId === undefined || action.backendId === "") {
				return nextState;
			}
			return updateSessionInStore(nextState, action.sessionId, (session) => ({
				...session,
				backendId: action.backendId,
			}));
		}
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
			const nextState = {
				...state,
				backends: action.backends,
				selectedBackendId,
				availableModelsByBackend,
			};
			return {
				...nextState,
				availableModels: allAvailableModels(availableModelsByBackend),
			};
		}
		case "SET_SELECTED_BACKEND": {
			return {
				...state,
				selectedBackendId: action.backendId,
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
	planMode: false,
	pendingPermissions: {},
	pendingQueues: {},
	latestTokenUsage: {},
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
