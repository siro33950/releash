import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	AgentEditorContext,
	BackendInfo,
	ChatSession,
	CodexGoal,
	CodexRuntimeStatus,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	QueuedAgentTurn,
	SessionSummary,
	SlashCommand,
	TokenUsage,
	TurnPhase,
} from "@/types/session";
import {
	type AgentChatAction,
	INITIAL_STATE,
	reducer,
	selectActiveSession,
} from "./agentChatReducer";
import {
	type ActivityStatus,
	deriveActivityStatus,
} from "./deriveActivityStatus";
import { useAgentSdkListeners } from "./useAgentSdkListeners";
import {
	archiveOpenSession as archiveOpenSessionApi,
	archiveSession as archiveSessionApi,
	cancelAgentQueuedTurn,
	closeSession as closeSessionApi,
	createSession,
	forkSession as forkSessionApi,
	getSession,
	initAgentSessions,
	listAgentBackends,
	listClosedSessions,
	listSessions,
	readCodexModelCatalog,
	restoreSession as restoreSessionApi,
	rewindSessionToMessage as rewindSessionToMessageApi,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
	setSessionBackend,
	setSessionTitle as setSessionTitleApi,
} from "./useSessionStore";
import { useWorktreeSessionStatuses } from "./useWorktreeSessionStatuses";

export type { ActivityStatus } from "./deriveActivityStatus";

type RefreshSessionsOptions = { reconcileActiveSession?: boolean };
export type SendMessageOptions = {
	activateNewSession?: boolean;
	editorContext?: AgentEditorContext;
};

/**
 * SDK listener gating のために「現在 UI 上で表示中の session id 集合」を購読する registry。
 * 各 panel が表示開始時に register、unmount/離脱時に cleanup を呼ぶ。listener は本 set に
 * 含まれる session に対してのみ ADD_MESSAGE / SET_STREAMING_MESSAGE 等を dispatch する。
 */
interface ViewableSessionRegistry {
	register: (sessionId: string) => () => void;
	getIds: () => Set<string>;
}

export interface UseAgentChatResult {
	sessions: SessionSummary[];
	orderedSessions: SessionSummary[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	isStreaming: boolean;
	activityStatus: ActivityStatus;
	error: string | null;
	permissionMode: PermissionMode;
	sessionAgentStates: Map<string, AgentState>;
	/**
	 * 送信先 session を明示する API。`sessionId === null` は「新規 session を作成して送る」を
	 * 表す。送信応答の `response.session` は内部で `UPSERT_SESSION` され、各 panel は
	 * `getSessionById(id)` 経由で最新内容を観測する。
	 */
	sendMessage: (
		sessionId: string | null,
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
		options?: SendMessageOptions,
	) => Promise<void>;
	interrupt: (sessionId: string) => void;
	selectSession: (sessionId: string) => Promise<void>;
	refreshSessions: (
		options?: RefreshSessionsOptions,
	) => Promise<SessionSummary[] | undefined>;
	refreshClosedSessions: () => Promise<void>;
	closeSession: (sessionId: string) => Promise<void>;
	archiveSession: (sessionId: string) => Promise<void>;
	archiveOpenSession: (sessionId: string) => Promise<void>;
	restoreSession: (sessionId: string) => Promise<void>;
	rewindSessionToMessage: (
		sessionId: string,
		messageId: string,
		options?: { restoreWorktree?: boolean },
	) => Promise<void>;
	forkSession: (sessionId: string) => Promise<void>;
	setSessionTitle: (sessionId: string, title: string | null) => Promise<string>;
	createNewSession: () => Promise<void>;
	reorderSessions: (sessionOrder: string[]) => void;
	setPermissionMode: (sessionId: string | null, mode: PermissionMode) => void;
	respondPermission: (
		sessionId: string,
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
	availableModels: ModelInfo[];
	selectedModel: string | null;
	setModel: (sessionId: string, modelId: string) => void;
	backends: BackendInfo[];
	selectedBackendId: string | null;
	setBackend: (sessionId: string | null, backendId: string | null) => void;
	/**
	 * 任意 sessionId から最新の ChatSession を取得して sessionsById に upsert する。
	 * 各 panel が「自分が見たい step session を読み込む」用途で利用する。
	 * 内部状態の単一の正典は `sessionsById` であり、本関数の戻り値は upsert 後の
	 * snapshot（成功時）。
	 */
	loadSession: (sessionId: string) => Promise<ChatSession | null>;
	/** sessionsById から指定 session を取得する selector。*/
	getSessionById: (sessionId: string | null | undefined) => ChatSession | null;
	/** SDK イベント反映の gating 用に、現在 panel で表示している session を登録する。*/
	registerViewableSession: (sessionId: string) => () => void;
	/** per-session lookup（既存）。*/
	getSessionTurnPhase: (sessionId: string) => TurnPhase;
	getSessionInterrupting: (sessionId: string) => boolean;
	getSessionSelectedModel: (sessionId: string) => string | null;
	getSessionPendingQueue: (sessionId: string) => QueuedAgentTurn[];
	getSessionLatestTokenUsage: (sessionId: string) => TokenUsage | null;
	getSessionCodexGoal: (sessionId: string) => CodexGoal | null;
	getSessionCodexRuntimeStatus: (
		sessionId: string,
	) => CodexRuntimeStatus | null;
	getSessionRuntimeSlashCommands: (sessionId: string) => SlashCommand[];
	cancelQueuedTurn: (
		sessionId: string,
		queuedTurnId?: string | null,
	) => Promise<void>;
}

function startAgentProcess(
	chatSessionId: string,
	cwd: string,
	permissionMode: PermissionMode,
): void {
	invoke("start_agent_session", {
		chatSessionId,
		cwd,
		permissionMode,
	}).catch((e) => {
		console.error(`Failed to start agent session ${chatSessionId}:`, e);
	});
}

function dispatchSessionMeta(
	dispatch: React.Dispatch<AgentChatAction>,
	sessionId: string,
	response: {
		session: { permissionMode?: PermissionMode; backendId?: string | null };
		turnPhase: TurnPhase;
		selectedModel: string;
		availableModels: ModelInfo[];
		pendingQueue?: QueuedAgentTurn[];
		latestTokenUsage?: TokenUsage | null;
	},
) {
	dispatch({
		type: "SET_TURN_PHASE",
		sessionId,
		turnPhase: response.turnPhase,
	});
	if (response.session.permissionMode) {
		dispatch({
			type: "SET_PERMISSION_MODE",
			mode: response.session.permissionMode,
		});
	}
	dispatch({
		type: "SET_SESSION_MODEL",
		sessionId,
		modelId: response.selectedModel,
	});
	dispatch({
		type: "SET_AVAILABLE_MODELS",
		models: response.availableModels,
		backendId: response.session.backendId,
	});
	dispatch({
		type: "SET_PENDING_QUEUE",
		sessionId,
		queue: response.pendingQueue ?? [],
	});
	dispatch({
		type: "SET_LATEST_TOKEN_USAGE",
		sessionId,
		usage: response.latestTokenUsage ?? null,
	});
}

export function useAgentChat(
	worktreePath: string,
	workflowApprovalChatSessionId: string | null = null,
	workflowApprovalRunId: string | null = null,
): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const workflowApprovalChatSessionIdRef = useRef(
		workflowApprovalChatSessionId,
	);
	workflowApprovalChatSessionIdRef.current = workflowApprovalChatSessionId;
	const workflowApprovalRunIdRef = useRef(workflowApprovalRunId);
	workflowApprovalRunIdRef.current = workflowApprovalRunId;

	const activeSession = selectActiveSession(state);

	const activeSessionIdRef = useRef(state.activeSessionId);
	activeSessionIdRef.current = state.activeSessionId;
	const sessionsByIdRef = useRef(state.sessionsById);
	sessionsByIdRef.current = state.sessionsById;
	const sessionsRef = useRef(state.sessions);
	sessionsRef.current = state.sessions;
	const permissionModeRef = useRef(state.permissionMode);
	permissionModeRef.current = state.permissionMode;
	const turnPhasesRef = useRef(state.turnPhases);
	turnPhasesRef.current = state.turnPhases;
	const interruptingRef = useRef(state.interrupting);
	interruptingRef.current = state.interrupting;
	const selectedBackendIdRef = useRef(state.selectedBackendId);
	selectedBackendIdRef.current = state.selectedBackendId;

	// SDK listener gating: 表示中の session id 集合を管理する registry。
	// 各 panel が register したものは getIds() で参照される（listener が更新を gate する）。
	const viewableIdsRef = useRef<Map<string, number>>(new Map());
	const viewableRegistry = useMemo<ViewableSessionRegistry>(
		() => ({
			register: (sessionId: string) => {
				const map = viewableIdsRef.current;
				map.set(sessionId, (map.get(sessionId) ?? 0) + 1);
				return () => {
					const m = viewableIdsRef.current;
					const next = (m.get(sessionId) ?? 0) - 1;
					if (next <= 0) {
						m.delete(sessionId);
					} else {
						m.set(sessionId, next);
					}
				};
			},
			getIds: () => new Set(viewableIdsRef.current.keys()),
		}),
		[],
	);

	const refreshSessions = useCallback(
		async (options: RefreshSessionsOptions = {}): Promise<SessionSummary[]> => {
			try {
				const previousSessions = sessionsRef.current;
				const previousActiveSessionId = activeSessionIdRef.current;
				const sessions = await listSessions(worktreePathRef.current);
				dispatch({ type: "SET_SESSIONS", sessions });
				if (
					options.reconcileActiveSession === true &&
					previousActiveSessionId &&
					!sessions.some((session) => session.id === previousActiveSessionId)
				) {
					const previousIndex = previousSessions.findIndex(
						(session) => session.id === previousActiveSessionId,
					);
					// spec issues-1023: free chat tab bar に並ばない workflow step session は
					// 自由対話の active 候補としても選ばない（chat panel の本文を
					// workflow step transcript で乗っ取らない）。
					const freeChatSessions = sessions.filter(
						(session) => !session.workflowStepSession,
					);
					const nextSession =
						freeChatSessions.length > 0
							? freeChatSessions[
									Math.min(
										Math.max(previousIndex, 0),
										freeChatSessions.length - 1,
									)
								]
							: null;
					if (nextSession) {
						const response = await getSession(nextSession.id);
						if (activeSessionIdRef.current === previousActiveSessionId) {
							if (response) {
								dispatch({ type: "UPSERT_SESSION", session: response.session });
								dispatch({
									type: "SET_ACTIVE_SESSION_ID",
									sessionId: response.session.id,
								});
								dispatchSessionMeta(dispatch, nextSession.id, response);
							} else {
								dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
							}
						}
					} else if (activeSessionIdRef.current === previousActiveSessionId) {
						dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
					}
				}
				return sessions;
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッション一覧の取得に失敗: ${e}`,
				});
				return [];
			}
		},
		[],
	);

	const refreshClosedSessions = useCallback(async () => {
		try {
			const sessions = await listClosedSessions(worktreePathRef.current);
			dispatch({ type: "SET_CLOSED_SESSIONS", sessions });
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `クローズ済みセッション一覧の取得に失敗: ${e}`,
			});
		}
	}, []);

	const selectSession = useCallback(async (sessionId: string) => {
		try {
			const response = await getSession(sessionId);
			if (response) {
				dispatch({ type: "UPSERT_SESSION", session: response.session });
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId: response.session.id,
				});
				dispatchSessionMeta(dispatch, sessionId, response);
			} else {
				dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
			}
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッションの読み込みに失敗: ${e}`,
			});
		}
	}, []);

	const loadSession = useCallback(
		async (sessionId: string): Promise<ChatSession | null> => {
			try {
				const response = await getSession(sessionId);
				if (response) {
					dispatch({ type: "UPSERT_SESSION", session: response.session });
					dispatchSessionMeta(dispatch, sessionId, response);
					return response.session;
				}
				return null;
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `session の読み込みに失敗: ${e}`,
				});
				return null;
			}
		},
		[],
	);

	const interrupt = useCallback((sessionId: string) => {
		if (!sessionId) return;
		// 既に interrupt 要求済みなら握りつぶす（連打抑止）。
		if (interruptingRef.current[sessionId]) return;
		// 楽観的に interrupting 状態へ。turn が idle になった時点で reducer が
		// 自動クリアする。これで停止押下が即座に UI へ反映される。
		dispatch({ type: "SET_INTERRUPTING", sessionId, value: true });
		invoke("interrupt_agent_query", { chatSessionId: sessionId }).catch((e) => {
			console.error("Failed to interrupt agent query:", e);
			// 送信自体に失敗したら楽観フラグを戻す。
			dispatch({ type: "SET_INTERRUPTING", sessionId, value: false });
		});
	}, []);

	const sendMessage = useCallback(
		async (
			sessionId: string | null,
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
			options?: SendMessageOptions,
		) => {
			const trimmed = content.trim();
			if (!trimmed && (!images || images.length === 0)) return;

			try {
				const wPath = worktreePathRef.current;
				const pm = permissionModeRef.current;
				const backendId = sessionId ? null : selectedBackendIdRef.current;
				const workflowApprovalChatSessionId =
					workflowApprovalChatSessionIdRef.current;
				const workflowApprovalRunId = workflowApprovalRunIdRef.current;
				const response =
					sessionId &&
					workflowApprovalChatSessionId === sessionId &&
					workflowApprovalRunId
						? await sendWorkflowApprovalChatMessage(
								workflowApprovalRunId,
								trimmed,
								pm,
								images,
								mentions,
							)
						: options?.editorContext
							? await sendAgentMessage(
									sessionId,
									wPath,
									trimmed,
									pm,
									backendId,
									images,
									mentions,
									options.editorContext,
								)
							: await sendAgentMessage(
									sessionId,
									wPath,
									trimmed,
									pm,
									backendId,
									images,
									mentions,
								);
				const responseSessionId = response.session.id;
				dispatch({ type: "UPSERT_SESSION", session: response.session });
				dispatch({
					type: "SET_PENDING_QUEUE",
					sessionId: responseSessionId,
					queue: response.pendingQueue,
				});
				// 新規作成 session の場合、active を切り替える（既存 sessionId 指定で送った場合は
				// active を変更しない — Workflow panel から step session に送ったときに Main の
				// active を上書きしないため）。
				if (sessionId === null && options?.activateNewSession !== false) {
					dispatch({
						type: "SET_ACTIVE_SESSION_ID",
						sessionId: responseSessionId,
					});
				}
				dispatch({ type: "SET_SESSIONS", sessions: response.sessions });
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `メッセージ送信に失敗: ${e}`,
				});
			}
		},
		[],
	);

	const cancelQueuedTurn = useCallback(
		async (sessionId: string, queuedTurnId?: string | null) => {
			try {
				const response = await cancelAgentQueuedTurn(sessionId, queuedTurnId);
				dispatch({
					type: "SET_PENDING_QUEUE",
					sessionId: response.sessionId,
					queue: response.pendingQueue,
				});
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `キューのキャンセルに失敗: ${e}`,
				});
			}
		},
		[],
	);

	const closeSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				const sessions = sessionsRef.current;
				const idx = sessions.findIndex((s) => s.id === sessionId);
				await closeSessionApi(sessionId);

				dispatch({ type: "CLEANUP_SESSION", sessionId });

				const isActive = activeSessionIdRef.current === sessionId;
				if (isActive) {
					// spec issues-1023: 閉じた後の active 候補も free chat に閉じる。
					const remaining = sessions.filter(
						(s) => s.id !== sessionId && !s.workflowStepSession,
					);
					const nextSession =
						remaining.length > 0
							? remaining[Math.min(idx, remaining.length - 1)]
							: null;
					if (nextSession) {
						const response = await getSession(nextSession.id);
						if (response) {
							dispatch({ type: "UPSERT_SESSION", session: response.session });
							dispatch({
								type: "SET_ACTIVE_SESSION_ID",
								sessionId: response.session.id,
							});
							dispatchSessionMeta(dispatch, nextSession.id, response);
						} else {
							dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
						}
					} else {
						dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
					}
				}

				await refreshSessions();
				await refreshClosedSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションクローズに失敗: ${e}`,
				});
			}
		},
		[refreshSessions, refreshClosedSessions],
	);

	const restoreSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				let restoredWorkflowStep = false;
				const restoreResult = await restoreSessionApi(sessionId);
				restoredWorkflowStep = restoreResult.restoredWorkflowStep === true;
				const response = await getSession(sessionId);
				if (response) {
					dispatch({ type: "UPSERT_SESSION", session: response.session });
					dispatch({
						type: "SET_ACTIVE_SESSION_ID",
						sessionId: response.session.id,
					});
					dispatchSessionMeta(dispatch, sessionId, response);
					if (
						!restoredWorkflowStep &&
						(response.session.messages.length > 0 ||
							response.session.agentSessionId)
					) {
						startAgentProcess(
							sessionId,
							worktreePathRef.current,
							response.session.permissionMode,
						);
					}
				} else {
					dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
				}
				await refreshSessions();
				await refreshClosedSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッション復元に失敗: ${e}`,
				});
			}
		},
		[refreshSessions, refreshClosedSessions],
	);

	const archiveSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				await archiveSessionApi(sessionId);
				await refreshClosedSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションアーカイブに失敗: ${e}`,
				});
			}
		},
		[refreshClosedSessions],
	);

	const archiveOpenSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				const sessions = sessionsRef.current;
				const idx = sessions.findIndex((s) => s.id === sessionId);
				await archiveOpenSessionApi(sessionId);
				dispatch({ type: "CLEANUP_SESSION", sessionId });

				const isActive = activeSessionIdRef.current === sessionId;
				if (isActive) {
					const remaining = sessions.filter(
						(s) => s.id !== sessionId && !s.workflowStepSession,
					);
					const nextSession =
						remaining.length > 0
							? remaining[Math.min(idx, remaining.length - 1)]
							: null;
					if (nextSession) {
						const response = await getSession(nextSession.id);
						if (response) {
							dispatch({ type: "UPSERT_SESSION", session: response.session });
							dispatch({
								type: "SET_ACTIVE_SESSION_ID",
								sessionId: response.session.id,
							});
							dispatchSessionMeta(dispatch, nextSession.id, response);
						} else {
							dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
						}
					} else {
						dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
					}
				}

				await refreshSessions();
				await refreshClosedSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションアーカイブに失敗: ${e}`,
				});
			}
		},
		[refreshSessions, refreshClosedSessions],
	);

	const rewindSessionToMessageFn = useCallback(
		async (
			sessionId: string,
			messageId: string,
			options?: { restoreWorktree?: boolean },
		) => {
			try {
				const rewound = await rewindSessionToMessageApi(
					sessionId,
					messageId,
					options,
				);
				const response = await getSession(rewound.id);
				const activeSession = response?.session ?? rewound;
				dispatch({ type: "UPSERT_SESSION", session: activeSession });
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId: activeSession.id,
				});
				if (response) {
					dispatchSessionMeta(dispatch, activeSession.id, response);
				}
				await refreshSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションの巻き戻しに失敗: ${e}`,
				});
			}
		},
		[refreshSessions],
	);

	const forkSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				const forked = await forkSessionApi(sessionId);
				const response = await getSession(forked.id);
				const activeSession = response?.session ?? forked;
				dispatch({ type: "UPSERT_SESSION", session: activeSession });
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId: activeSession.id,
				});
				dispatch({
					type: "SET_PERMISSION_MODE",
					mode: activeSession.permissionMode,
				});
				if (response) {
					dispatchSessionMeta(dispatch, activeSession.id, response);
				}
				await refreshSessions();
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションのフォークに失敗: ${e}`,
				});
			}
		},
		[refreshSessions],
	);

	const setSessionTitleFn = useCallback(
		async (sessionId: string, title: string | null): Promise<string> => {
			try {
				const summary = await setSessionTitleApi(sessionId, title);
				await refreshSessions();
				await refreshClosedSessions();
				return summary.firstMessage || "New session";
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `セッションタイトル変更に失敗: ${e}`,
				});
				throw e;
			}
		},
		[refreshSessions, refreshClosedSessions],
	);

	const createNewSession = useCallback(async () => {
		try {
			const activeSessionSnapshot = activeSessionIdRef.current
				? sessionsByIdRef.current[activeSessionIdRef.current]
				: undefined;
			const backendId =
				activeSessionSnapshot?.backendId ?? selectedBackendIdRef.current;
			const session = await createSession(
				worktreePathRef.current,
				permissionModeRef.current,
				backendId,
			);
			const response = await getSession(session.id);
			const activeSession = response?.session ?? session;
			dispatch({ type: "UPSERT_SESSION", session: activeSession });
			dispatch({
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: activeSession.id,
			});
			dispatch({
				type: "SET_PERMISSION_MODE",
				mode: activeSession.permissionMode,
			});
			if (response) {
				dispatchSessionMeta(dispatch, session.id, response);
			}
			await refreshSessions();
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション作成に失敗: ${e}`,
			});
		}
	}, [refreshSessions]);

	const reorderSessions = useCallback((sessionOrder: string[]) => {
		dispatch({ type: "REORDER_SESSIONS", sessionOrder });
	}, []);

	const setPermissionMode = useCallback(
		(sessionId: string | null, mode: PermissionMode) => {
			// state.permissionMode は store 全体に 1 つしか存在しないグローバル値。
			// 非表示 (non-viewable) の session からの呼び出しで UI 表示用の
			// permissionMode が上書きされるのを防ぐため、SDK event listener 側で
			// SET_PERMISSION_MODE を viewableRegistry でガードしているのと同様に、
			// UI 起点でも viewable な session の操作のみ dispatch する。
			// sessionId が null の場合は session 非依存の global default 設定として扱う。
			const isViewable =
				sessionId === null || viewableIdsRef.current.has(sessionId);
			if (isViewable) {
				dispatch({ type: "SET_PERMISSION_MODE", mode });
			}
			// Persist to Rust and sync to Bridge
			if (sessionId) {
				invoke("set_agent_permission_mode", {
					chatSessionId: sessionId,
					permissionMode: mode,
				}).catch((e) => {
					console.error("Failed to set agent permission mode:", e);
				});
			}
		},
		[],
	);

	const respondPermission = useCallback(
		(
			sessionId: string,
			requestId: string,
			allow: boolean,
			updatedInput?: Record<string, unknown>,
		) => {
			if (!sessionId) return;
			invoke("respond_agent_permission", {
				chatSessionId: sessionId,
				requestId,
				behavior: allow ? "allow" : "deny",
				message: allow ? null : "User denied",
				updatedInput: updatedInput ? JSON.stringify(updatedInput) : null,
			}).catch((e) => {
				console.error("Failed to respond to permission:", e);
				dispatch({
					type: "SET_ERROR",
					error: `パーミッション応答に失敗: ${e}`,
				});
			});
			dispatch({
				type: "SET_PENDING_PERMISSION",
				sessionId,
				request: null,
			});
		},
		[],
	);

	const setModel = useCallback((sessionId: string, modelId: string) => {
		if (!sessionId) return;
		invoke("set_agent_model", {
			chatSessionId: sessionId,
			modelId,
		})
			.then(() => {
				dispatch({
					type: "SET_SESSION_MODEL",
					sessionId,
					modelId,
				});
			})
			.catch((e) => {
				console.error("Failed to set agent model:", e);
			});
	}, []);

	const setBackend = useCallback(
		(sessionId: string | null, backendId: string | null) => {
			// sessionId === null は「新規 session 用の backend 既定」を保存する経路。
			if (!sessionId) {
				dispatch({ type: "SET_SELECTED_BACKEND", backendId });
				return;
			}
			// 既存 session の backend 変更は active session 経由の従来制約（メッセージ 0 件かつ
			// agent 未起動かつ非ストリーミング）を満たす場合のみ受理する。
			const activeId = activeSessionIdRef.current;
			const activeSession = activeId
				? (sessionsByIdRef.current[activeId] ?? null)
				: null;
			if (
				!backendId ||
				!activeSession ||
				activeSession.id !== sessionId ||
				activeSession.messages.length > 0 ||
				activeSession.agentSessionId
			) {
				return;
			}
			setSessionBackend(sessionId, backendId)
				.then((response) => {
					if (activeSessionIdRef.current === sessionId) {
						dispatch({ type: "UPSERT_SESSION", session: response.session });
						dispatchSessionMeta(dispatch, sessionId, response);
					}
				})
				.catch((e) => {
					dispatch({
						type: "SET_ERROR",
						error: `Agent の変更に失敗: ${e}`,
					});
				});
		},
		[],
	);

	useAgentSdkListeners({
		dispatch,
		viewableRegistry,
		refreshSessions,
		hasMessage: (sessionId, messageId) =>
			(sessionsByIdRef.current[sessionId]?.messages ?? []).some(
				(m) => m.id === messageId,
			),
	});

	// activeSession は Main panel が表示している session として実質 viewable。
	// BoundSessionChat も独自に register するが、Main panel 経由でない経路（テスト等）
	// でも listener gating が機能するように、本 hook 側でも自動登録する。
	useEffect(() => {
		if (!state.activeSessionId) return;
		const cleanup = viewableRegistry.register(state.activeSessionId);
		return cleanup;
	}, [state.activeSessionId, viewableRegistry]);

	const fetchBackends = useCallback(async () => {
		try {
			const result = await listAgentBackends();
			const hasCodex = result.backends.some(
				(backend) => backend.id === "codex" && backend.available,
			);
			if (hasCodex) {
				try {
					const models = await readCodexModelCatalog();
					if (models.length > 0) {
						result.backends = result.backends.map((backend) =>
							backend.id === "codex"
								? { ...backend, availableModels: models }
								: backend,
						);
					}
				} catch (e) {
					console.warn("Failed to fetch Codex model catalog:", e);
				}
			}
			dispatch({
				type: "SET_BACKENDS",
				backends: result.backends,
				defaultId: result.defaultId,
			});
		} catch (e) {
			console.error("Failed to fetch agent backends:", e);
		}
	}, []);

	const initSessions = useCallback(async () => {
		try {
			const response = await initAgentSessions(worktreePathRef.current);
			dispatch({ type: "SET_SESSIONS", sessions: response.sessions });
			if (response.activeSession) {
				dispatch({
					type: "UPSERT_SESSION",
					session: response.activeSession.session,
				});
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId: response.activeSession.session.id,
				});
				dispatchSessionMeta(
					dispatch,
					response.activeSession.session.id,
					response.activeSession,
				);
			}
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション初期化に失敗: ${e}`,
			});
		}
	}, []);

	// Load sessions and backends on mount
	useEffect(() => {
		initSessions();
		fetchBackends();
	}, [initSessions, fetchBackends]);

	// Reset when worktreePath changes
	const prevWorktreePathRef = useRef(worktreePath);
	useEffect(() => {
		if (prevWorktreePathRef.current !== worktreePath) {
			prevWorktreePathRef.current = worktreePath;
			dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
			dispatch({ type: "SET_PERMISSION_MODE", mode: "edit" });
			initSessions();
			refreshClosedSessions();
		}
	}, [worktreePath, initSessions, refreshClosedSessions]);

	const activeTurnPhase: TurnPhase =
		state.turnPhases[state.activeSessionId ?? ""] ?? "idle";
	const isStreaming =
		activeTurnPhase === "streaming" || activeTurnPhase === "waiting_permission";

	const orderedSessions = useMemo(() => {
		const sessionMap = new Map(state.sessions.map((s) => [s.id, s]));
		return state.sessionOrder
			.map((id) => sessionMap.get(id))
			.filter((s): s is SessionSummary => !!s);
	}, [state.sessions, state.sessionOrder]);

	// Rust 中央管理 (AgentStatusCenter) から SessionStatus を購読し、
	// session_id → agent_state の Map を生成する。フロント側で派生計算は行わない。
	const worktreeSessionStatuses = useWorktreeSessionStatuses(worktreePath);
	const sessionAgentStates = useMemo(() => {
		const map = new Map<string, AgentState>();
		for (const [sessionId, status] of worktreeSessionStatuses) {
			map.set(sessionId, status.agent_state);
		}
		return map;
	}, [worktreeSessionStatuses]);

	const activityStatus = useMemo(
		() => deriveActivityStatus(activeSession?.messages, activeTurnPhase),
		[activeSession?.messages, activeTurnPhase],
	);

	const turnPhasesState = state.turnPhases;
	const sessionModelsState = state.sessionModels;
	const pendingQueuesState = state.pendingQueues;
	const latestTokenUsageState = state.latestTokenUsage;
	const codexGoalsState = state.codexGoals;
	const codexRuntimeStatusesState = state.codexRuntimeStatuses;
	const runtimeSlashCommandsState = state.runtimeSlashCommands;
	const getSessionTurnPhase = useCallback(
		(sessionId: string): TurnPhase => turnPhasesState[sessionId] ?? "idle",
		[turnPhasesState],
	);
	const interruptingState = state.interrupting;
	const getSessionInterrupting = useCallback(
		(sessionId: string): boolean => interruptingState[sessionId] ?? false,
		[interruptingState],
	);
	const getSessionSelectedModel = useCallback(
		(sessionId: string): string | null => sessionModelsState[sessionId] ?? null,
		[sessionModelsState],
	);
	const getSessionPendingQueue = useCallback(
		(sessionId: string): QueuedAgentTurn[] =>
			pendingQueuesState[sessionId] ?? [],
		[pendingQueuesState],
	);
	const getSessionLatestTokenUsage = useCallback(
		(sessionId: string): TokenUsage | null =>
			latestTokenUsageState[sessionId] ?? null,
		[latestTokenUsageState],
	);
	const getSessionCodexGoal = useCallback(
		(sessionId: string) => codexGoalsState[sessionId] ?? null,
		[codexGoalsState],
	);
	const getSessionCodexRuntimeStatus = useCallback(
		(sessionId: string) => codexRuntimeStatusesState[sessionId] ?? null,
		[codexRuntimeStatusesState],
	);
	const getSessionRuntimeSlashCommands = useCallback(
		(sessionId: string): SlashCommand[] =>
			runtimeSlashCommandsState[sessionId] ?? [],
		[runtimeSlashCommandsState],
	);

	const sessionsByIdState = state.sessionsById;
	const getSessionById = useCallback(
		(sessionId: string | null | undefined): ChatSession | null => {
			if (!sessionId) return null;
			return sessionsByIdState[sessionId] ?? null;
		},
		[sessionsByIdState],
	);

	const registerViewableSession = useCallback(
		(sessionId: string) => viewableRegistry.register(sessionId),
		[viewableRegistry],
	);

	const selectedModel =
		state.sessionModels[state.activeSessionId ?? ""] ?? null;
	return {
		sessions: state.sessions,
		orderedSessions,
		closedSessions: state.closedSessions,
		activeSession,
		isStreaming,
		activityStatus,
		error: state.error,
		permissionMode: state.permissionMode,
		sessionAgentStates,
		sendMessage,
		interrupt,
		selectSession,
		refreshSessions,
		refreshClosedSessions,
		closeSession: closeSessionFn,
		archiveSession: archiveSessionFn,
		archiveOpenSession: archiveOpenSessionFn,
		restoreSession: restoreSessionFn,
		rewindSessionToMessage: rewindSessionToMessageFn,
		forkSession: forkSessionFn,
		setSessionTitle: setSessionTitleFn,
		createNewSession,
		reorderSessions,
		setPermissionMode,
		respondPermission,
		availableModels: state.availableModels,
		selectedModel,
		setModel,
		backends: state.backends,
		selectedBackendId: activeSession
			? (activeSession.backendId ?? state.selectedBackendId)
			: state.selectedBackendId,
		setBackend,
		loadSession,
		getSessionById,
		registerViewableSession,
		getSessionTurnPhase,
		getSessionInterrupting,
		getSessionSelectedModel,
		getSessionPendingQueue,
		getSessionLatestTokenUsage,
		getSessionCodexGoal,
		getSessionCodexRuntimeStatus,
		getSessionRuntimeSlashCommands,
		cancelQueuedTurn,
	};
}
