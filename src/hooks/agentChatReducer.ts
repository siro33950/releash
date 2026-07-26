import {
	compareCanonicalDecimal,
	isCanonicalDecimal,
} from "@/lib/canonicalDecimal";
import type {
	AgentStallObservation,
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
	 * 停止ボタン押下を即座に UI へ反映する。turnPhase が idle に
	 * 遷移した時点で自動クリアされる。
	 */
	interrupting: Record<string, boolean>;
	/** Rust-owned notice state の session_id 別 mirror。 */
	sessionErrors: Record<string, string>;
	/** 非同期 snapshot の逆転を拒否するための、session ごとの既知 revision。 */
	sessionErrorRevisions: Record<string, string>;
	permissionMode: PermissionMode;
	planMode: PlanMode;
	sessionPermissionModes: Record<string, PermissionMode>;
	sessionPlanModes: Record<string, PlanMode>;
	pendingPermissions: Record<string, PermissionRequest>;
	pendingPermissionStateRevisions: Record<string, string>;
	clearedPendingPermissionIds: Record<string, string>;
	pendingQueues: Record<string, QueuedAgentTurn[]>;
	queuePaused: Record<string, boolean>;
	stallObservations?: Record<string, AgentStallObservation>;
	latestTokenUsage: Record<string, TokenUsage | null>;
	runtimeSlashCommands: Record<string, SlashCommand[]>;
	canChangeBackend: Record<string, boolean>;
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
	| { type: "PREPEND_MESSAGES"; sessionId: string; messages: ChatMessage[] }
	| { type: "EVICT_SESSION_BODY"; sessionId: string }
	| { type: "EVICT_OLDER_MESSAGES"; sessionId: string; count: number }
	| {
			type: "SET_TURN_PHASE";
			sessionId: string;
			turnPhase: TurnPhase;
			ignoreIfClearedPendingRequestId?: string | null;
			pendingPermissionStateRevision?: string | null;
	  }
	| { type: "SET_INTERRUPTING"; sessionId: string; value: boolean }
	| {
			type: "SYNC_SESSION_ERROR";
			sessionId: string;
			revision: string;
			message: string | null;
	  }
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
	| {
			type: "SET_PERMISSION_MODE";
			mode: PermissionMode;
			sessionId?: string | null;
	  }
	| { type: "SET_PLAN_MODE"; enabled: PlanMode; sessionId?: string | null }
	| {
			type: "SET_PENDING_PERMISSION";
			sessionId: string;
			request: PermissionRequest | null;
			ignoreIfCleared?: boolean;
			pendingPermissionStateRevision?: string | null;
	  }
	| {
			type: "SET_PENDING_QUEUE";
			sessionId: string;
			queue: QueuedAgentTurn[];
	  }
	| { type: "SET_QUEUE_PAUSED"; sessionId: string; value: boolean }
	| {
			type: "SET_STALL_OBSERVATION";
			sessionId: string;
			observation: AgentStallObservation;
	  }
	| {
			type: "CLEAR_STALL_OBSERVATION";
			sessionId: string;
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
			type: "SET_CAN_CHANGE_BACKEND";
			sessionId: string;
			value: boolean;
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
			type: "APPLY_STREAMING_DELTA";
			sessionId: string;
			messageId: string;
			seq: string;
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

function appendStreamingDeltaParts(
	current: MessagePart[] | undefined,
	delta: MessagePart[],
): MessagePart[] {
	const next = [...(current ?? [])];
	for (const part of delta) {
		const last = next[next.length - 1];
		if (
			last?.type === "text" &&
			part.type === "text" &&
			(last.parentToolUseId ?? null) === (part.parentToolUseId ?? null)
		) {
			next[next.length - 1] = {
				...last,
				content: `${last.content}${part.content}`,
			};
			continue;
		} else if (
			last?.type === "thinking" &&
			part.type === "thinking" &&
			(last.parentToolUseId ?? null) === (part.parentToolUseId ?? null)
		) {
			next[next.length - 1] = {
				...last,
				content: `${last.content}${part.content}`,
			};
			continue;
		}

		next.push(part);
	}
	return next;
}

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
	if (session.messages.some((existing) => existing.id === message.id)) {
		return session;
	}
	return { ...session, messages: [...session.messages, message] };
}

function prependMessages(
	session: ChatSession,
	messages: ChatMessage[],
): ChatSession {
	if (messages.length === 0) return session;
	const existingIds = new Set(session.messages.map((message) => message.id));
	const newMessages = messages.filter(
		(message) => !existingIds.has(message.id),
	);
	if (newMessages.length === 0) return session;
	return { ...session, messages: [...newMessages, ...session.messages] };
}

function evictOlderMessages(session: ChatSession, count: number): ChatSession {
	if (count <= 0) return session;
	if (session.messages.length === 0) return session;
	return { ...session, messages: session.messages.slice(count) };
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
	const existing = state.sessionsById[session.id];
	const storedSession =
		existing && session.messages.length === 0
			? { ...session, messages: existing.messages }
			: session;
	const sessionPermissionModes = {
		...state.sessionPermissionModes,
		[storedSession.id]: storedSession.permissionMode,
	};
	const sessionPlanModes = {
		...state.sessionPlanModes,
		[storedSession.id]: storedSession.planMode ?? false,
	};
	const isActive = state.activeSessionId === storedSession.id;
	return {
		...state,
		sessionsById: {
			...state.sessionsById,
			[session.id]: storedSession,
		},
		permissionMode: isActive
			? storedSession.permissionMode
			: state.permissionMode,
		planMode: isActive ? (storedSession.planMode ?? false) : state.planMode,
		sessionPermissionModes,
		sessionPlanModes,
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

function normalizePermissionStateRevision(
	revision: string | null | undefined,
): string | null {
	return typeof revision === "string" && isCanonicalDecimal(revision)
		? revision
		: null;
}

function currentPermissionStateRevision(
	state: AgentChatState,
	sessionId: string,
): string {
	return state.pendingPermissionStateRevisions[sessionId] ?? "0";
}

function isOlderPermissionStateRevision(
	state: AgentChatState,
	sessionId: string,
	revision: string | null,
): boolean {
	return (
		revision !== null &&
		compareCanonicalDecimal(
			revision,
			currentPermissionStateRevision(state, sessionId),
		) < 0
	);
}

function withPermissionStateRevision(
	state: AgentChatState,
	sessionId: string,
	revision: string | null,
): Pick<AgentChatState, "pendingPermissionStateRevisions"> {
	if (revision === null) {
		return {
			pendingPermissionStateRevisions: state.pendingPermissionStateRevisions,
		};
	}
	return {
		pendingPermissionStateRevisions: {
			...state.pendingPermissionStateRevisions,
			[sessionId]: revision,
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
		case "SET_ACTIVE_SESSION_ID": {
			if (!action.sessionId) {
				return {
					...state,
					activeSessionId: null,
				};
			}
			const activeSession = state.sessionsById[action.sessionId];
			return {
				...state,
				activeSessionId: action.sessionId,
				permissionMode:
					state.sessionPermissionModes[action.sessionId] ??
					activeSession?.permissionMode ??
					state.permissionMode,
				planMode:
					state.sessionPlanModes[action.sessionId] ??
					activeSession?.planMode ??
					state.planMode,
			};
		}
		case "ADD_MESSAGE": {
			return updateSessionInStore(state, action.sessionId, (s) =>
				appendMessage(s, action.message),
			);
		}
		case "PREPEND_MESSAGES":
			return updateSessionInStore(state, action.sessionId, (s) =>
				prependMessages(s, action.messages),
			);
		case "EVICT_SESSION_BODY":
			return updateSessionInStore(state, action.sessionId, (s) =>
				s.messages.length === 0 ? s : { ...s, messages: [] },
			);
		case "EVICT_OLDER_MESSAGES":
			return updateSessionInStore(state, action.sessionId, (s) =>
				evictOlderMessages(s, action.count),
			);
		case "SET_TURN_PHASE": {
			const revision = normalizePermissionStateRevision(
				action.pendingPermissionStateRevision,
			);
			if (isOlderPermissionStateRevision(state, action.sessionId, revision)) {
				return state;
			}
			if (
				action.ignoreIfClearedPendingRequestId &&
				state.clearedPendingPermissionIds[action.sessionId] ===
					action.ignoreIfClearedPendingRequestId &&
				(revision === null ||
					compareCanonicalDecimal(
						revision,
						currentPermissionStateRevision(state, action.sessionId),
					) <= 0)
			) {
				return state;
			}
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
				...withPermissionStateRevision(state, action.sessionId, revision),
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
		case "SYNC_SESSION_ERROR": {
			const knownRevision = state.sessionErrorRevisions[action.sessionId];
			if (
				knownRevision !== undefined &&
				compareCanonicalDecimal(action.revision, knownRevision) <= 0
			) {
				return state;
			}
			const sessionErrorRevisions = {
				...state.sessionErrorRevisions,
				[action.sessionId]: action.revision,
			};
			if (action.message !== null) {
				return {
					...state,
					sessionErrorRevisions,
					sessionErrors: {
						...state.sessionErrors,
						[action.sessionId]: action.message,
					},
				};
			}
			if (!(action.sessionId in state.sessionErrors)) {
				return { ...state, sessionErrorRevisions };
			}
			const { [action.sessionId]: _drop, ...rest } = state.sessionErrors;
			return { ...state, sessionErrors: rest, sessionErrorRevisions };
		}
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
			if (action.sessionId) {
				return {
					...state,
					permissionMode:
						state.activeSessionId === action.sessionId
							? action.mode
							: state.permissionMode,
					sessionPermissionModes: {
						...state.sessionPermissionModes,
						[action.sessionId]: action.mode,
					},
				};
			}
			return { ...state, permissionMode: action.mode };
		case "SET_PLAN_MODE":
			if (action.sessionId) {
				return {
					...state,
					planMode:
						state.activeSessionId === action.sessionId
							? action.enabled
							: state.planMode,
					sessionPlanModes: {
						...state.sessionPlanModes,
						[action.sessionId]: action.enabled,
					},
				};
			}
			return { ...state, planMode: action.enabled };
		case "SET_PENDING_PERMISSION": {
			const revision = normalizePermissionStateRevision(
				action.pendingPermissionStateRevision,
			);
			if (isOlderPermissionStateRevision(state, action.sessionId, revision)) {
				return state;
			}
			if (action.request === null) {
				const clearedRequestId = state.pendingPermissions[action.sessionId]?.id;
				const { [action.sessionId]: _, ...rest } = state.pendingPermissions;
				if (!clearedRequestId) {
					return {
						...state,
						pendingPermissions: rest,
						...withPermissionStateRevision(state, action.sessionId, revision),
					};
				}
				return {
					...state,
					pendingPermissions: rest,
					...withPermissionStateRevision(state, action.sessionId, revision),
					clearedPendingPermissionIds: {
						...state.clearedPendingPermissionIds,
						[action.sessionId]: clearedRequestId,
					},
				};
			}
			if (
				action.ignoreIfCleared &&
				state.clearedPendingPermissionIds[action.sessionId] ===
					action.request.id &&
				(revision === null ||
					compareCanonicalDecimal(
						revision,
						currentPermissionStateRevision(state, action.sessionId),
					) <= 0)
			) {
				return state;
			}
			const { [action.sessionId]: _, ...restClearedPermissionIds } =
				state.clearedPendingPermissionIds;
			return {
				...state,
				pendingPermissions: {
					...state.pendingPermissions,
					[action.sessionId]: action.request,
				},
				...withPermissionStateRevision(state, action.sessionId, revision),
				clearedPendingPermissionIds: restClearedPermissionIds,
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
		case "SET_QUEUE_PAUSED":
			return {
				...state,
				queuePaused: {
					...state.queuePaused,
					[action.sessionId]: action.value,
				},
			};
		case "SET_STALL_OBSERVATION":
			return {
				...state,
				stallObservations: {
					...(state.stallObservations ?? {}),
					[action.sessionId]: action.observation,
				},
			};
		case "CLEAR_STALL_OBSERVATION": {
			const current = state.stallObservations ?? {};
			if (!current[action.sessionId]) return state;
			const { [action.sessionId]: _drop, ...rest } = current;
			return { ...state, stallObservations: rest };
		}
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
		case "APPLY_STREAMING_DELTA": {
			const existing = state.sessionsById[action.sessionId];
			if (!existing) return state;
			const updated = applyMessageUpdate(existing, action.messageId, (m) => ({
				...m,
				parts: appendStreamingDeltaParts(m.parts, action.parts),
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
			const { [action.sessionId]: _se, ...restSessionErrors } =
				state.sessionErrors;
			const { [action.sessionId]: _ser, ...restSessionErrorRevisions } =
				state.sessionErrorRevisions;
			const { [action.sessionId]: _tp, ...restTurnPhases } = state.turnPhases;
			const { [action.sessionId]: _int, ...restInterrupting } =
				state.interrupting;
			const { [action.sessionId]: _pp, ...restPendingPermissions } =
				state.pendingPermissions;
			const {
				[action.sessionId]: _ppr,
				...restPendingPermissionStateRevisions
			} = state.pendingPermissionStateRevisions;
			const { [action.sessionId]: _cpp, ...restClearedPendingPermissionIds } =
				state.clearedPendingPermissionIds;
			const { [action.sessionId]: _pq, ...restPendingQueues } =
				state.pendingQueues;
			const { [action.sessionId]: _qp, ...restQueuePaused } = state.queuePaused;
			const { [action.sessionId]: _so, ...restStallObservations } =
				state.stallObservations ?? {};
			const { [action.sessionId]: _tu, ...restLatestTokenUsage } =
				state.latestTokenUsage;
			const { [action.sessionId]: _rsc, ...restRuntimeSlashCommands } =
				state.runtimeSlashCommands;
			const { [action.sessionId]: _cbe, ...restCanChangeBackend } =
				state.canChangeBackend;
			const { [action.sessionId]: _sm, ...restSessionModels } =
				state.sessionModels;
			const { [action.sessionId]: _perm, ...restSessionPermissionModes } =
				state.sessionPermissionModes;
			const { [action.sessionId]: _plan, ...restSessionPlanModes } =
				state.sessionPlanModes;
			const { [action.sessionId]: _sb, ...restSessionsById } =
				state.sessionsById;
			return {
				...state,
				sessionErrors: restSessionErrors,
				sessionErrorRevisions: restSessionErrorRevisions,
				turnPhases: restTurnPhases,
				interrupting: restInterrupting,
				pendingPermissions: restPendingPermissions,
				pendingPermissionStateRevisions: restPendingPermissionStateRevisions,
				clearedPendingPermissionIds: restClearedPendingPermissionIds,
				pendingQueues: restPendingQueues,
				queuePaused: restQueuePaused,
				stallObservations: restStallObservations,
				latestTokenUsage: restLatestTokenUsage,
				runtimeSlashCommands: restRuntimeSlashCommands,
				canChangeBackend: restCanChangeBackend,
				sessionModels: restSessionModels,
				sessionPermissionModes: restSessionPermissionModes,
				sessionPlanModes: restSessionPlanModes,
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
		case "SET_CAN_CHANGE_BACKEND": {
			return {
				...state,
				canChangeBackend: {
					...state.canChangeBackend,
					[action.sessionId]: action.value,
				},
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
	sessionErrors: {},
	sessionErrorRevisions: {},
	permissionMode: "edit",
	planMode: false,
	sessionPermissionModes: {},
	sessionPlanModes: {},
	pendingPermissions: {},
	pendingPermissionStateRevisions: {},
	clearedPendingPermissionIds: {},
	pendingQueues: {},
	queuePaused: {},
	stallObservations: {},
	latestTokenUsage: {},
	runtimeSlashCommands: {},
	canChangeBackend: {},
	availableModels: [],
	availableModelsByBackend: {},
	sessionModels: {},
	backends: [],
	selectedBackendId: null,
};

/** sessionsById から指定 session を解決する selector。reducer 外から参照する経路で利用。*/
function selectSessionFromState(
	state: AgentChatState,
	sessionId: string | null | undefined,
): ChatSession | null {
	if (!sessionId) return null;
	return state.sessionsById[sessionId] ?? null;
}

export function selectActiveSession(state: AgentChatState): ChatSession | null {
	return selectSessionFromState(state, state.activeSessionId);
}
