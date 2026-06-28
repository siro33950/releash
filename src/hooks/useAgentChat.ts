import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	AgentEditorContext,
	BackendInfo,
	ChatSession,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	PlanMode,
	QueuedAgentTurn,
	SessionSummary,
	SlashCommand,
	TokenUsage,
	TurnPhase,
} from "@/types/session";
import {
	getModelInfoBackend,
	getModelInfoId,
	normalizeModelSelectionId,
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
	type ActiveMessageEvictionPlan,
	archiveOpenSession as archiveOpenSessionApi,
	archiveSession as archiveSessionApi,
	cancelAgentQueuedTurn,
	closeSession as closeSessionApi,
	createSession,
	forkSession as forkSessionApi,
	getSession,
	getSessionPage,
	initAgentSessions,
	type LoadedMessagePage,
	listAgentBackends,
	listClosedSessions,
	listSessions,
	planAgentChatEviction,
	restoreSession as restoreSessionApi,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
	setSessionBackend,
	setSessionTitle as setSessionTitleApi,
} from "./useSessionStore";
import { useWorktreeSessionStatuses } from "./useWorktreeSessionStatuses";

export type { ActivityStatus } from "./deriveActivityStatus";

const DEFAULT_SESSION_TITLE = "NewSession";

function dispatchWorkspaceTreeRefresh(worktreePath: string): void {
	if (typeof window === "undefined") return;
	window.dispatchEvent(
		new CustomEvent("workspace-tree-refresh", {
			detail: { worktreePath },
		}),
	);
}

type RefreshSessionsOptions = { reconcileActiveSession?: boolean };
export type SendMessageOptions = {
	activateNewSession?: boolean;
	editorContext?: AgentEditorContext;
};
type GetSessionInitialPage = {
	nextCursor: string | null;
	hasMore: boolean;
	totalCount: number;
};
type SessionPageState = {
	nextCursor: string | null;
	hasMore: boolean;
	loading: boolean;
	loadedPages: LoadedMessagePage[];
};
export type OlderMessageEvictionOptions = {
	oldestVisibleIndex?: number;
	onEvicted?: (eviction: { count: number; direction: "older" }) => void;
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
	planMode: PlanMode;
	sessionAgentStates: Map<string, AgentState>;
	/**
	 * 送信先 session を明示する API。`sessionId === null` は「新規 session を作成して送る」を
	 * 表す。送信応答の `response.session` は shell として `UPSERT_SESSION` され、
	 * 永続化された human/agent message は別フィールドから追加される。
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
	forkSession: (sessionId: string) => Promise<void>;
	setSessionTitle: (sessionId: string, title: string | null) => Promise<string>;
	createNewSession: () => Promise<string | null>;
	reorderSessions: (sessionOrder: string[]) => void;
	setPermissionMode: (sessionId: string | null, mode: PermissionMode) => void;
	setPlanMode: (sessionId: string | null, enabled: PlanMode) => void;
	respondPermission: (
		sessionId: string,
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
	availableModels: ModelInfo[];
	availableModelsByBackend: Record<string, ModelInfo[]>;
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
	/** cursor paging で過去方向の message page を読み込む。*/
	loadOlderMessages: (sessionId: string) => Promise<void>;
	/** 可視範囲から離れた古い page を webview キャッシュから退避する。*/
	evictOlderMessages: (
		sessionId: string,
		options?: OlderMessageEvictionOptions,
	) => Promise<void>;
	/** per-session lookup（既存）。*/
	getSessionTurnPhase: (sessionId: string) => TurnPhase;
	getSessionInterrupting: (sessionId: string) => boolean;
	getSessionPermissionMode: (sessionId: string) => PermissionMode;
	getSessionPlanMode: (sessionId: string) => PlanMode;
	getSessionSelectedModel: (sessionId: string) => string | null;
	getSessionPendingQueue: (sessionId: string) => QueuedAgentTurn[];
	getSessionLatestTokenUsage: (sessionId: string) => TokenUsage | null;
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
	planMode: PlanMode,
): void {
	invoke("start_agent_session", {
		chatSessionId,
		cwd,
		permissionMode,
		planMode,
	}).catch((e) => {
		console.error(`Failed to start agent session ${chatSessionId}:`, e);
	});
}

function loadedMessageCount(pages: LoadedMessagePage[]): number {
	return pages.reduce((sum, page) => sum + page.count, 0);
}

function initialLoadedPages(count: number): LoadedMessagePage[] {
	return count > 0 ? [{ requestCursor: null, count }] : [];
}

function loadedPagesEqual(
	left: LoadedMessagePage[],
	right: LoadedMessagePage[],
): boolean {
	if (left.length !== right.length) return false;
	return left.every(
		(page, index) =>
			page.requestCursor === right[index]?.requestCursor &&
			page.count === right[index]?.count,
	);
}

type PageWindowSnapshot = {
	messageCount: number;
	nextCursor: string | null;
	hasMore: boolean;
	loadedPages: LoadedMessagePage[];
};

function pageWindowUnchanged(
	currentPageState: SessionPageState,
	currentSession: ChatSession,
	snapshot: PageWindowSnapshot,
	plan: ActiveMessageEvictionPlan,
): boolean {
	return (
		!currentPageState.loading &&
		currentSession.messages.length === snapshot.messageCount &&
		currentPageState.nextCursor === snapshot.nextCursor &&
		currentPageState.hasMore === snapshot.hasMore &&
		loadedPagesEqual(currentPageState.loadedPages, snapshot.loadedPages) &&
		loadedMessageCount(plan.loadedPages) === snapshot.messageCount - plan.count
	);
}

function reportEvictionPlanSkipped(e: unknown): void {
	console.warn(
		`メッセージ退避計画の取得に失敗。退避をスキップし、次回トリガで再試行します: ${e}`,
	);
}

function dispatchSessionMeta(
	dispatch: React.Dispatch<AgentChatAction>,
	sessionId: string,
	response: {
		session: {
			permissionMode?: PermissionMode;
			planMode?: PlanMode;
			backendId?: string | null;
		};
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
			sessionId,
			mode: response.session.permissionMode,
		});
	}
	if (response.session.planMode !== undefined) {
		dispatch({
			type: "SET_PLAN_MODE",
			sessionId,
			enabled: response.session.planMode,
		});
	}
	dispatch({
		type: "SET_SESSION_MODEL",
		sessionId,
		modelId: response.selectedModel,
	});
	dispatch({
		type: "SET_AVAILABLE_MODELS",
		models: response.availableModels ?? [],
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
	const planModeRef = useRef(state.planMode);
	planModeRef.current = state.planMode;
	const sessionPermissionModesRef = useRef(state.sessionPermissionModes);
	sessionPermissionModesRef.current = state.sessionPermissionModes;
	const sessionPlanModesRef = useRef(state.sessionPlanModes);
	sessionPlanModesRef.current = state.sessionPlanModes;
	const turnPhasesRef = useRef(state.turnPhases);
	turnPhasesRef.current = state.turnPhases;
	const interruptingRef = useRef(state.interrupting);
	interruptingRef.current = state.interrupting;
	const selectedBackendIdRef = useRef(state.selectedBackendId);
	selectedBackendIdRef.current = state.selectedBackendId;
	const availableModelsRef = useRef(state.availableModels);
	availableModelsRef.current = state.availableModels;
	const sessionModelsRef = useRef(state.sessionModels);
	sessionModelsRef.current = state.sessionModels;
	const pageStateRef = useRef<Record<string, SessionPageState>>({});
	const evictInactiveSessionsRef = useRef<() => void>(() => {});
	const activeMessageEvictionsRef = useRef<Set<string>>(new Set());
	const sessionAccessSeqRef = useRef(0);
	const sessionEvictionRanksRef = useRef<Record<string, number>>({});

	const touchSessionAccess = useCallback((sessionId: string) => {
		const nextRank = sessionAccessSeqRef.current + 1;
		sessionAccessSeqRef.current = nextRank;
		sessionEvictionRanksRef.current[sessionId] = nextRank;
	}, []);

	const rememberInitialPage = useCallback(
		(response: {
			session: ChatSession;
			initialPage?: GetSessionInitialPage;
		}) => {
			touchSessionAccess(response.session.id);
			const page = response.initialPage;
			const loadedPages = initialLoadedPages(response.session.messages.length);
			pageStateRef.current[response.session.id] = {
				nextCursor: page?.nextCursor ?? null,
				hasMore: page?.hasMore ?? false,
				loading: false,
				loadedPages,
			};
		},
		[touchSessionAccess],
	);

	// SDK listener gating: 表示中の session id 集合を管理する registry。
	// 各 panel が register したものは getIds() で参照される（listener が更新を gate する）。
	const viewableIdsRef = useRef<Map<string, number>>(new Map());
	const viewableRegistry = useMemo<ViewableSessionRegistry>(
		() => ({
			register: (sessionId: string) => {
				touchSessionAccess(sessionId);
				const map = viewableIdsRef.current;
				map.set(sessionId, (map.get(sessionId) ?? 0) + 1);
				return () => {
					const m = viewableIdsRef.current;
					const next = (m.get(sessionId) ?? 0) - 1;
					if (next <= 0) {
						m.delete(sessionId);
						evictInactiveSessionsRef.current();
					} else {
						m.set(sessionId, next);
					}
				};
			},
			getIds: () => new Set(viewableIdsRef.current.keys()),
		}),
		[touchSessionAccess],
	);

	const dispatchWithMessageWindowTracking = useCallback(
		(action: AgentChatAction) => {
			if (action.type === "ADD_MESSAGE") {
				const pageState = pageStateRef.current[action.sessionId];
				const session = sessionsByIdRef.current[action.sessionId];
				const alreadyLoaded =
					session?.messages.some(
						(message) => message.id === action.message.id,
					) ?? false;
				if (pageState && !alreadyLoaded) {
					const latestPage = pageState.loadedPages[0];
					pageStateRef.current[action.sessionId] = {
						...pageState,
						loadedPages: latestPage
							? [
									{ ...latestPage, count: latestPage.count + 1 },
									...pageState.loadedPages.slice(1),
								]
							: [{ requestCursor: null, count: 1 }],
					};
				}
			}
			dispatch(action);
		},
		[],
	);

	const evictInactiveSessions = useCallback(async () => {
		const protectedIds = new Set(viewableIdsRef.current.keys());
		if (activeSessionIdRef.current) {
			protectedIds.add(activeSessionIdRef.current);
		}
		const sessions = Object.entries(sessionsByIdRef.current).map(
			([sessionId, session]) => ({
				sessionId,
				messageCount: session.messages.length,
				evictionRank: sessionEvictionRanksRef.current[sessionId] ?? 0,
				protected: protectedIds.has(sessionId),
				loading: pageStateRef.current[sessionId]?.loading ?? false,
			}),
		);
		if (!sessions.some((session) => session.messageCount > 0)) return;

		try {
			const plan = await planAgentChatEviction({ sessions });
			for (const sessionId of plan.evictSessionIds) {
				const session = sessionsByIdRef.current[sessionId];
				if (!session || session.messages.length === 0) continue;
				if (activeSessionIdRef.current === sessionId) continue;
				if (viewableIdsRef.current.has(sessionId)) continue;
				if (pageStateRef.current[sessionId]?.loading) continue;
				dispatch({ type: "EVICT_SESSION_BODY", sessionId });
				delete pageStateRef.current[sessionId];
			}
		} catch (e) {
			reportEvictionPlanSkipped(e);
		}
	}, []);
	evictInactiveSessionsRef.current = () => {
		void evictInactiveSessions();
	};

	useEffect(() => {
		for (const [sessionId, session] of Object.entries(state.sessionsById)) {
			if (session.messages.length === 0) continue;
			if (pageStateRef.current[sessionId]) continue;
			pageStateRef.current[sessionId] = {
				nextCursor: null,
				hasMore: false,
				loading: false,
				loadedPages: initialLoadedPages(session.messages.length),
			};
		}
	}, [state.sessionsById]);

	const evictOlderMessages = useCallback(
		async (sessionId: string, options: OlderMessageEvictionOptions = {}) => {
			if (activeMessageEvictionsRef.current.has(sessionId)) return;
			const pageState = pageStateRef.current[sessionId];
			const session = sessionsByIdRef.current[sessionId];
			if (!pageState || !session || pageState.loading) return;
			const messageCount = session.messages.length;
			if (messageCount === 0) return;
			if (loadedMessageCount(pageState.loadedPages) !== messageCount) return;
			const oldestVisibleIndex = options.oldestVisibleIndex ?? 0;
			const snapshot = {
				messageCount,
				nextCursor: pageState.nextCursor,
				hasMore: pageState.hasMore,
				loadedPages: pageState.loadedPages,
			};
			activeMessageEvictionsRef.current.add(sessionId);
			try {
				const plan = await planAgentChatEviction({
					active: {
						sessionId,
						messageCount,
						oldestVisibleIndex,
						loadedPages: pageState.loadedPages,
						turnPhase: turnPhasesRef.current[sessionId] ?? "idle",
					},
				});
				const activePlan = plan.active;
				if (!activePlan || activePlan.count <= 0) return;
				if (activePlan.sessionId !== sessionId) return;
				const currentPageState = pageStateRef.current[sessionId];
				const currentSession = sessionsByIdRef.current[sessionId];
				if (!currentPageState || !currentSession) return;
				if (
					!pageWindowUnchanged(
						currentPageState,
						currentSession,
						snapshot,
						activePlan,
					)
				) {
					return;
				}
				pageStateRef.current[sessionId] = {
					...currentPageState,
					nextCursor: activePlan.nextCursor,
					hasMore: activePlan.hasMore,
					loading: false,
					loadedPages: activePlan.loadedPages,
				};
				dispatch({
					type: "EVICT_OLDER_MESSAGES",
					sessionId,
					count: activePlan.count,
				});
				options.onEvicted?.({
					count: activePlan.count,
					direction: "older",
				});
			} catch (e) {
				reportEvictionPlanSkipped(e);
			} finally {
				activeMessageEvictionsRef.current.delete(sessionId);
			}
		},
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
								rememberInitialPage(response);
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
		[rememberInitialPage],
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

	const selectSession = useCallback(
		async (sessionId: string) => {
			try {
				const response = await getSession(sessionId);
				if (response) {
					rememberInitialPage(response);
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
		},
		[rememberInitialPage],
	);

	const loadSession = useCallback(
		async (sessionId: string): Promise<ChatSession | null> => {
			try {
				const response = await getSession(sessionId);
				if (response) {
					rememberInitialPage(response);
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
		[rememberInitialPage],
	);

	const loadOlderMessages = useCallback(
		async (sessionId: string) => {
			const pageState = pageStateRef.current[sessionId];
			if (
				!pageState?.hasMore ||
				!pageState.nextCursor ||
				pageState.loading ||
				activeMessageEvictionsRef.current.has(sessionId)
			) {
				return;
			}
			touchSessionAccess(sessionId);
			const requestCursor = pageState.nextCursor;
			pageState.loading = true;
			try {
				const page = await getSessionPage(sessionId, requestCursor);
				const currentPageState = pageStateRef.current[sessionId];
				if (
					!currentPageState?.loading ||
					currentPageState.nextCursor !== requestCursor
				) {
					return;
				}
				if (!page) {
					pageStateRef.current[sessionId] = {
						...currentPageState,
						nextCursor: null,
						hasMore: false,
						loading: false,
					};
					return;
				}
				const existingIds = new Set(
					(sessionsByIdRef.current[sessionId]?.messages ?? []).map(
						(message) => message.id,
					),
				);
				const newMessageCount = page.messages.filter(
					(message) => !existingIds.has(message.id),
				).length;
				dispatch({
					type: "PREPEND_MESSAGES",
					sessionId,
					messages: page.messages,
				});
				pageStateRef.current[sessionId] = {
					nextCursor: page.nextCursor,
					hasMore: page.hasMore,
					loading: false,
					loadedPages:
						newMessageCount > 0
							? [
									...currentPageState.loadedPages,
									{ requestCursor, count: newMessageCount },
								]
							: currentPageState.loadedPages,
				};
			} catch (e) {
				pageState.loading = false;
				dispatch({
					type: "SET_ERROR",
					error: `過去メッセージの読み込みに失敗: ${e}`,
				});
			}
		},
		[touchSessionAccess],
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

	const resolvePermissionModeForSessionRef = useCallback(
		(sessionId: string | null): PermissionMode => {
			if (!sessionId) return permissionModeRef.current;
			return (
				sessionPermissionModesRef.current[sessionId] ??
				sessionsByIdRef.current[sessionId]?.permissionMode ??
				permissionModeRef.current
			);
		},
		[],
	);

	const resolvePlanModeForSessionRef = useCallback(
		(sessionId: string | null): PlanMode => {
			if (!sessionId) return planModeRef.current;
			return (
				sessionPlanModesRef.current[sessionId] ??
				sessionsByIdRef.current[sessionId]?.planMode ??
				planModeRef.current
			);
		},
		[],
	);

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
				const pm = resolvePermissionModeForSessionRef(sessionId);
				const plan = resolvePlanModeForSessionRef(sessionId);
				const backendId = sessionId ? null : selectedBackendIdRef.current;
				const modelId =
					sessionId || !activeSessionIdRef.current
						? null
						: (sessionModelsRef.current[activeSessionIdRef.current] ?? null);
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
								plan,
								images,
								mentions,
							)
						: options?.editorContext
							? modelId
								? await sendAgentMessage(
										sessionId,
										wPath,
										trimmed,
										pm,
										plan,
										backendId,
										images,
										mentions,
										options.editorContext,
										modelId,
									)
								: await sendAgentMessage(
										sessionId,
										wPath,
										trimmed,
										pm,
										plan,
										backendId,
										images,
										mentions,
										options.editorContext,
									)
							: modelId
								? await sendAgentMessage(
										sessionId,
										wPath,
										trimmed,
										pm,
										plan,
										backendId,
										images,
										mentions,
										undefined,
										modelId,
									)
								: await sendAgentMessage(
										sessionId,
										wPath,
										trimmed,
										pm,
										plan,
										backendId,
										images,
										mentions,
									);
				const responseSessionId = response.session.id;
				touchSessionAccess(responseSessionId);
				dispatch({ type: "UPSERT_SESSION", session: response.session });
				if (!response.queuedTurn) {
					dispatchWithMessageWindowTracking({
						type: "ADD_MESSAGE",
						sessionId: responseSessionId,
						message: response.humanMessage,
					});
					if (response.agentMessage) {
						dispatchWithMessageWindowTracking({
							type: "ADD_MESSAGE",
							sessionId: responseSessionId,
							message: response.agentMessage,
						});
					}
				}
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
				dispatchWorkspaceTreeRefresh(response.session.worktreePath);
			} catch (e) {
				dispatch({
					type: "SET_ERROR",
					error: `メッセージ送信に失敗: ${e}`,
				});
			}
		},
		[
			dispatchWithMessageWindowTracking,
			resolvePermissionModeForSessionRef,
			resolvePlanModeForSessionRef,
			touchSessionAccess,
		],
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
							rememberInitialPage(response);
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
		[refreshSessions, refreshClosedSessions, rememberInitialPage],
	);

	const restoreSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				let restoredWorkflowStep = false;
				const restoreResult = await restoreSessionApi(sessionId);
				restoredWorkflowStep = restoreResult.restoredWorkflowStep === true;
				const response = await getSession(sessionId);
				if (response) {
					rememberInitialPage(response);
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
							response.session.planMode ?? false,
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
		[refreshSessions, refreshClosedSessions, rememberInitialPage],
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
							rememberInitialPage(response);
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
		[refreshSessions, refreshClosedSessions, rememberInitialPage],
	);

	const forkSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				const forked = await forkSessionApi(sessionId);
				const response = await getSession(forked.id);
				const activeSession = response?.session ?? forked;
				if (response) {
					rememberInitialPage(response);
				}
				dispatch({ type: "UPSERT_SESSION", session: activeSession });
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId: activeSession.id,
				});
				dispatch({
					type: "SET_PERMISSION_MODE",
					sessionId: activeSession.id,
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
		[refreshSessions, rememberInitialPage],
	);

	const setSessionTitleFn = useCallback(
		async (sessionId: string, title: string | null): Promise<string> => {
			try {
				const summary = await setSessionTitleApi(sessionId, title);
				await refreshSessions();
				await refreshClosedSessions();
				return summary.firstMessage || DEFAULT_SESSION_TITLE;
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

	const createNewSession = useCallback(async (): Promise<string | null> => {
		try {
			const activeSessionSnapshot = activeSessionIdRef.current
				? sessionsByIdRef.current[activeSessionIdRef.current]
				: undefined;
			const backendId =
				activeSessionSnapshot?.backendId ?? selectedBackendIdRef.current;
			const modelId = activeSessionSnapshot
				? (sessionModelsRef.current[activeSessionSnapshot.id] ?? null)
				: null;
			const session = modelId
				? await createSession(
						worktreePathRef.current,
						permissionModeRef.current,
						backendId,
						modelId,
					)
				: await createSession(
						worktreePathRef.current,
						permissionModeRef.current,
						backendId,
					);
			const response = await getSession(session.id);
			const activeSession = response?.session ?? session;
			if (response) {
				rememberInitialPage(response);
			}
			dispatch({ type: "UPSERT_SESSION", session: activeSession });
			dispatch({
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: activeSession.id,
			});
			dispatch({
				type: "SET_PERMISSION_MODE",
				sessionId: activeSession.id,
				mode: activeSession.permissionMode,
			});
			if (response) {
				dispatchSessionMeta(dispatch, session.id, response);
			}
			await refreshSessions();
			return activeSession.id;
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション作成に失敗: ${e}`,
			});
			return null;
		}
	}, [refreshSessions, rememberInitialPage]);

	const reorderSessions = useCallback((sessionOrder: string[]) => {
		dispatch({ type: "REORDER_SESSIONS", sessionOrder });
	}, []);

	const setPermissionMode = useCallback(
		(sessionId: string | null, mode: PermissionMode) => {
			// sessionId が null の場合は session 非依存の default 設定として扱う。
			// session 指定時は表示中 pane の mode map だけを更新し、active session
			// 以外の pane 操作で単一 session 表示用の mode を上書きしない。
			const isViewable =
				sessionId === null || viewableIdsRef.current.has(sessionId);
			if (isViewable) {
				dispatch({ type: "SET_PERMISSION_MODE", sessionId, mode });
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

	const setPlanMode = useCallback(
		(sessionId: string | null, enabled: PlanMode) => {
			const isViewable =
				sessionId === null || viewableIdsRef.current.has(sessionId);
			if (isViewable) {
				dispatch({ type: "SET_PLAN_MODE", sessionId, enabled });
			}
			if (sessionId) {
				invoke("set_agent_plan_mode", {
					chatSessionId: sessionId,
					planMode: enabled,
				}).catch((e) => {
					console.error("Failed to set agent plan mode:", e);
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
		},
		[],
	);

	const setModel = useCallback((sessionId: string, modelId: string) => {
		if (!sessionId) return;
		const normalizedModelId = normalizeModelSelectionId(
			availableModelsRef.current,
			modelId,
		);
		const selectedModel = availableModelsRef.current.find(
			(model) => getModelInfoId(model) === normalizedModelId,
		);
		invoke("set_agent_model", {
			chatSessionId: sessionId,
			modelId: normalizedModelId,
		})
			.then(() => {
				dispatch({
					type: "SET_SESSION_MODEL",
					sessionId,
					modelId: normalizedModelId,
					backendId: selectedModel
						? getModelInfoBackend(selectedModel)
						: undefined,
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
		dispatch: dispatchWithMessageWindowTracking,
		viewableRegistry,
		refreshSessions,
		worktreePath,
	});

	// activeSession は Main panel が表示している session として実質 viewable。
	// BoundSessionChat も独自に register するが、Main panel 経由でない経路（テスト等）
	// でも listener gating が機能するように、本 hook 側でも自動登録する。
	useEffect(() => {
		if (!state.activeSessionId) return;
		const cleanup = viewableRegistry.register(state.activeSessionId);
		return cleanup;
	}, [state.activeSessionId, viewableRegistry]);

	const hydratedSessionEvictionKey = useMemo(
		() =>
			[
				state.activeSessionId ?? "",
				...Object.entries(state.sessionsById)
					.filter(([, session]) => session.messages.length > 0)
					.map(([sessionId]) => sessionId)
					.sort(),
			].join("|"),
		[state.activeSessionId, state.sessionsById],
	);

	useEffect(() => {
		if (hydratedSessionEvictionKey.length === 0) return;
		void evictInactiveSessions();
	}, [evictInactiveSessions, hydratedSessionEvictionKey]);

	const fetchBackends = useCallback(async () => {
		try {
			const result = await listAgentBackends();
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
			dispatch({
				type: "SET_PERMISSION_MODE",
				mode: response.permissionMode ?? "edit",
			});
			dispatch({
				type: "SET_PLAN_MODE",
				enabled: response.planMode ?? false,
			});
			if (response.activeSession) {
				rememberInitialPage(response.activeSession);
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
	}, [rememberInitialPage]);

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
	const runtimeSlashCommandsState = state.runtimeSlashCommands;
	const sessionsByIdState = state.sessionsById;
	const getSessionTurnPhase = useCallback(
		(sessionId: string): TurnPhase => turnPhasesState[sessionId] ?? "idle",
		[turnPhasesState],
	);
	const interruptingState = state.interrupting;
	const getSessionInterrupting = useCallback(
		(sessionId: string): boolean => interruptingState[sessionId] ?? false,
		[interruptingState],
	);
	const sessionPermissionModesState = state.sessionPermissionModes;
	const permissionModeState = state.permissionMode;
	const getSessionPermissionMode = useCallback(
		(sessionId: string): PermissionMode =>
			sessionPermissionModesState[sessionId] ??
			sessionsByIdState[sessionId]?.permissionMode ??
			permissionModeState,
		[permissionModeState, sessionPermissionModesState, sessionsByIdState],
	);
	const sessionPlanModesState = state.sessionPlanModes;
	const planModeState = state.planMode;
	const getSessionPlanMode = useCallback(
		(sessionId: string): PlanMode =>
			sessionPlanModesState[sessionId] ??
			sessionsByIdState[sessionId]?.planMode ??
			planModeState,
		[planModeState, sessionPlanModesState, sessionsByIdState],
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
	const getSessionRuntimeSlashCommands = useCallback(
		(sessionId: string): SlashCommand[] =>
			runtimeSlashCommandsState[sessionId] ?? [],
		[runtimeSlashCommandsState],
	);

	const getSessionById = useCallback(
		(sessionId: string | null | undefined): ChatSession | null => {
			if (!sessionId) return null;
			return sessionsByIdState[sessionId] ?? null;
		},
		[sessionsByIdState],
	);

	const registerViewableSession = useCallback(
		(sessionId: string) => {
			return viewableRegistry.register(sessionId);
		},
		[viewableRegistry],
	);

	const selectedModel = state.activeSessionId
		? normalizeModelSelectionId(
				state.availableModels,
				state.sessionModels[state.activeSessionId] ?? null,
			) || null
		: null;
	return {
		sessions: state.sessions,
		orderedSessions,
		closedSessions: state.closedSessions,
		activeSession,
		isStreaming,
		activityStatus,
		error: state.error,
		permissionMode: state.permissionMode,
		planMode: state.planMode,
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
		forkSession: forkSessionFn,
		setSessionTitle: setSessionTitleFn,
		createNewSession,
		reorderSessions,
		setPermissionMode,
		setPlanMode,
		respondPermission,
		availableModels: state.availableModels,
		availableModelsByBackend: state.availableModelsByBackend,
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
		loadOlderMessages,
		evictOlderMessages,
		getSessionTurnPhase,
		getSessionInterrupting,
		getSessionPermissionMode,
		getSessionPlanMode,
		getSessionSelectedModel,
		getSessionPendingQueue,
		getSessionLatestTokenUsage,
		getSessionRuntimeSlashCommands,
		cancelQueuedTurn,
	};
}
