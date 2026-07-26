import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	AgentEditorContext,
	AgentStallObservation,
	BackendInfo,
	ChatMessage,
	ChatSession,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	PermissionRequest,
	PlanMode,
	QueuedAgentTurn,
	SessionNotice,
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
	type AgentSessionNoticeOperation,
	type AgentSessionNoticeSnapshot,
	type AgentSessionNoticeUpdate,
	archiveOpenSession as archiveOpenSessionApi,
	archiveSession as archiveSessionApi,
	cancelAgentQueuedTurn,
	closeSession as closeSessionApi,
	createSession,
	createWorkspaceSession,
	forkSession as forkSessionApi,
	type GetSessionResponse,
	getAgentSessionNotice,
	getSession,
	getSessionPage,
	initAgentSessions,
	type LoadedMessagePage,
	listAgentBackends,
	listClosedSessions,
	listSessions,
	planAgentChatEviction,
	requestAgentStop,
	respondAgentPermission,
	restoreSession as restoreSessionApi,
	resumeAgentQueue,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
	setSessionBackend,
	setSessionTitle as setSessionTitleApi,
	updateAgentSessionNotice,
} from "./useSessionStore";
import { useWorktreeSessionStatuses } from "./useWorktreeSessionStatuses";

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
type SendMessageOptions = {
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
	) => Promise<boolean>;
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
	createNewWorkspaceSession: (requestId: string) => Promise<string>;
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
	 * 各 panel が「自分が見たい node session を読み込む」用途で利用する。
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
	getSessionError: (sessionId: string) => string | null;
	dismissSessionError: (sessionId: string) => void;
	getSessionTurnPhase: (sessionId: string) => TurnPhase;
	getSessionInterrupting: (sessionId: string) => boolean;
	getSessionPermissionMode: (sessionId: string) => PermissionMode;
	getSessionPlanMode: (sessionId: string) => PlanMode;
	getSessionSelectedModel: (sessionId: string) => string | null;
	getSessionCanChangeBackend: (sessionId: string) => boolean;
	getSessionPendingPermission: (sessionId: string) => PermissionRequest | null;
	getSessionPendingQueue: (sessionId: string) => QueuedAgentTurn[];
	getSessionQueuePaused: (sessionId: string) => boolean;
	getSessionStallObservation: (
		sessionId: string,
	) => AgentStallObservation | null;
	getSessionNotice: (sessionId: string) => SessionNotice | null;
	getSessionLatestTokenUsage: (sessionId: string) => TokenUsage | null;
	getSessionRuntimeSlashCommands: (sessionId: string) => SlashCommand[];
	cancelQueuedTurn: (
		sessionId: string,
		queuedTurnId?: string | null,
	) => Promise<void>;
	resumeQueue: (sessionId: string) => Promise<void>;
}

function loadedMessageCount(pages: LoadedMessagePage[]): number {
	return pages.reduce((sum, page) => sum + page.count, 0);
}

function initialLoadedPages(count: number): LoadedMessagePage[] {
	return count > 0 ? [{ requestCursor: null, count }] : [];
}

function mergeAuthoritativeMessageWindow(
	current: ChatMessage[],
	authoritative: ChatMessage[],
): ChatMessage[] {
	if (current.length === 0) return authoritative;
	if (authoritative.length === 0) return current;

	const merged = current.slice();
	for (
		let authoritativeIndex = 0;
		authoritativeIndex < authoritative.length;
		authoritativeIndex++
	) {
		const message = authoritative[authoritativeIndex];
		const existingIndex = merged.findIndex(
			(candidate) => candidate.id === message.id,
		);
		if (existingIndex >= 0) {
			merged[existingIndex] = message;
			continue;
		}

		let insertionIndex = -1;
		for (
			let nextIndex = authoritativeIndex + 1;
			nextIndex < authoritative.length;
			nextIndex++
		) {
			const nextMessageId = authoritative[nextIndex]?.id;
			const nextExistingIndex = merged.findIndex(
				(candidate) => candidate.id === nextMessageId,
			);
			if (nextExistingIndex >= 0) {
				insertionIndex = nextExistingIndex;
				break;
			}
		}
		if (insertionIndex < 0) {
			for (
				let previousIndex = authoritativeIndex - 1;
				previousIndex >= 0;
				previousIndex--
			) {
				const previousMessageId = authoritative[previousIndex]?.id;
				const previousExistingIndex = merged.findIndex(
					(candidate) => candidate.id === previousMessageId,
				);
				if (previousExistingIndex >= 0) {
					insertionIndex = previousExistingIndex + 1;
					break;
				}
			}
		}
		if (insertionIndex < 0) {
			insertionIndex = merged.findIndex(
				(candidate) => candidate.timestamp > message.timestamp,
			);
		}
		merged.splice(
			insertionIndex < 0 ? merged.length : insertionIndex,
			0,
			message,
		);
	}
	return merged;
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

type NoticeRequestControllersRef = {
	current: Map<string, AbortController>;
};

function ensureNoticeController(
	requestControllers: NoticeRequestControllersRef,
	sessionId: string,
): AbortController {
	let controller = requestControllers.current.get(sessionId);
	if (!controller || controller.signal.aborted) {
		controller = new AbortController();
		requestControllers.current.set(sessionId, controller);
	}
	return controller;
}

function dispatchNoticeSnapshot(
	dispatch: React.Dispatch<AgentChatAction>,
	snapshot: AgentSessionNoticeSnapshot,
): void {
	dispatch({
		type: "SYNC_SESSION_ERROR",
		sessionId: snapshot.sessionId,
		revision: snapshot.revision,
		message: snapshot.notice?.message ?? null,
	});
}

async function syncSessionError(
	dispatch: React.Dispatch<AgentChatAction>,
	requestControllers: NoticeRequestControllersRef,
	sessionId: string,
	update: AgentSessionNoticeUpdate,
): Promise<void> {
	const controller = ensureNoticeController(requestControllers, sessionId);
	try {
		const snapshot = await updateAgentSessionNotice(sessionId, update);
		if (controller.signal.aborted) return;
		dispatchNoticeSnapshot(dispatch, snapshot);
	} catch (error) {
		console.error("Failed to synchronize agent session notice:", error);
	}
}

async function setSessionError(
	dispatch: React.Dispatch<AgentChatAction>,
	requestControllers: NoticeRequestControllersRef,
	sessionId: string | null,
	operation: AgentSessionNoticeOperation,
	message: string,
): Promise<void> {
	if (!sessionId) return;
	await syncSessionError(dispatch, requestControllers, sessionId, {
		action: "failure",
		operation,
		message,
	});
}

async function clearSessionError(
	dispatch: React.Dispatch<AgentChatAction>,
	requestControllers: NoticeRequestControllersRef,
	sessionId: string | null,
	operation: AgentSessionNoticeOperation,
): Promise<void> {
	if (!sessionId) return;
	await syncSessionError(dispatch, requestControllers, sessionId, {
		action: "success",
		operation,
	});
}

function cleanupSessionMirror(
	dispatch: React.Dispatch<AgentChatAction>,
	sessionsByIdRef: { current: Record<string, ChatSession> },
	requestControllers: NoticeRequestControllersRef,
	sessionId: string,
): void {
	requestControllers.current.get(sessionId)?.abort();
	requestControllers.current.delete(sessionId);
	const { [sessionId]: _removed, ...remainingSessions } =
		sessionsByIdRef.current;
	sessionsByIdRef.current = remainingSessions;
	dispatch({ type: "CLEANUP_SESSION", sessionId });
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
		canChangeBackend: boolean;
		pendingQueue?: QueuedAgentTurn[];
		queuePaused?: boolean;
		pendingPermissionRequest?: PermissionRequest | null;
		pendingPermissionStateRevision?: string | null;
		latestTokenUsage?: TokenUsage | null;
	},
) {
	dispatch({
		type: "SET_TURN_PHASE",
		sessionId,
		turnPhase: response.turnPhase,
		ignoreIfClearedPendingRequestId: response.pendingPermissionRequest?.id,
		pendingPermissionStateRevision:
			response.pendingPermissionStateRevision ?? null,
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
		type: "SET_CAN_CHANGE_BACKEND",
		sessionId,
		value: response.canChangeBackend,
	});
	dispatch({
		type: "SET_PENDING_QUEUE",
		sessionId,
		queue: response.pendingQueue ?? [],
	});
	dispatch({
		type: "SET_QUEUE_PAUSED",
		sessionId,
		value: response.queuePaused ?? false,
	});
	dispatch({
		type: "SET_PENDING_PERMISSION",
		sessionId,
		request: response.pendingPermissionRequest ?? null,
		ignoreIfCleared: response.pendingPermissionRequest != null,
		pendingPermissionStateRevision:
			response.pendingPermissionStateRevision ?? null,
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
	workflowApprovalExecutionId: string | null = null,
): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const workflowApprovalChatSessionIdRef = useRef(
		workflowApprovalChatSessionId,
	);
	workflowApprovalChatSessionIdRef.current = workflowApprovalChatSessionId;
	const workflowApprovalExecutionIdRef = useRef(workflowApprovalExecutionId);
	workflowApprovalExecutionIdRef.current = workflowApprovalExecutionId;

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
	const pendingQueuesRef = useRef(state.pendingQueues);
	pendingQueuesRef.current = state.pendingQueues;
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
	const sessionNoticeRequestControllersRef = useRef<
		Map<string, AbortController>
	>(new Map());
	const sessionSelectionIntentRef = useRef(0);
	const initSessionsGenerationRef = useRef(0);

	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;
		void listen<AgentSessionNoticeSnapshot>(
			"agent-session-notice-changed",
			(event) => {
				const sessionId = event.payload.sessionId;
				let controller =
					sessionNoticeRequestControllersRef.current.get(sessionId);
				if (!controller && !sessionsByIdRef.current[sessionId]) return;
				if (!controller) {
					controller = new AbortController();
					sessionNoticeRequestControllersRef.current.set(sessionId, controller);
				}
				if (controller.signal.aborted) return;
				dispatchNoticeSnapshot(dispatch, event.payload);
			},
		).then((nextUnlisten) => {
			if (cancelled) {
				nextUnlisten();
			} else {
				unlisten = nextUnlisten;
			}
		});
		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, []);

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
	const sessionReadbackGenerationsRef = useRef<Map<string, number>>(new Map());
	const sessionBodyGenerationsRef = useRef<Map<string, number>>(new Map());
	const sessionsNeedingReconciliationRef = useRef<Set<string>>(new Set());
	const sessionAuthorityReadEpochRef = useRef(0);
	const sessionAuthorityReadEpochsRef = useRef<Map<string, number>>(new Map());
	const markSessionForReconciliation = useCallback(
		(sessionId: string, options: { invalidateReadback?: boolean } = {}) => {
			sessionsNeedingReconciliationRef.current.add(sessionId);
			sessionAuthorityReadEpochRef.current += 1;
			sessionAuthorityReadEpochsRef.current.set(
				sessionId,
				sessionAuthorityReadEpochRef.current,
			);
			if (options.invalidateReadback !== false) {
				sessionReadbackGenerationsRef.current.set(
					sessionId,
					(sessionReadbackGenerationsRef.current.get(sessionId) ?? 0) + 1,
				);
			}
		},
		[],
	);
	const beginSessionAuthorityRead = useCallback((sessionId: string) => {
		sessionAuthorityReadEpochRef.current += 1;
		sessionAuthorityReadEpochsRef.current.set(
			sessionId,
			sessionAuthorityReadEpochRef.current,
		);
		const generation =
			(sessionReadbackGenerationsRef.current.get(sessionId) ?? 0) + 1;
		sessionReadbackGenerationsRef.current.set(sessionId, generation);
		return {
			generation,
			bodyGeneration: sessionBodyGenerationsRef.current.get(sessionId) ?? 0,
		};
	}, []);
	const applySessionAuthorityResponse = useCallback(
		(
			sessionId: string,
			token: { generation: number; bodyGeneration: number },
			response: GetSessionResponse,
			options: {
				requireViewable?: boolean;
				refreshWorkspace?: boolean;
				resetMessageWindow?: boolean;
			} = {},
		): boolean => {
			if (
				sessionReadbackGenerationsRef.current.get(sessionId) !==
					token.generation ||
				(options.requireViewable === true &&
					!viewableIdsRef.current.has(sessionId))
			) {
				return false;
			}
			const current = sessionsByIdRef.current[sessionId];
			const bodyChangedWhileReading =
				(sessionBodyGenerationsRef.current.get(sessionId) ?? 0) !==
				token.bodyGeneration;
			if (bodyChangedWhileReading && current != null) {
				dispatchSessionMeta(dispatch, sessionId, response);
				if (options.refreshWorkspace === true) {
					dispatchWorkspaceTreeRefresh(response.session.worktreePath);
				}
				return true;
			}
			const resetMessageWindow = options.resetMessageWindow === true;
			const messages = resetMessageWindow
				? response.session.messages
				: mergeAuthoritativeMessageWindow(
						current?.messages ?? [],
						response.session.messages,
					);
			const reconciledSession = {
				...response.session,
				messages,
			};
			const currentPageState = pageStateRef.current[sessionId];
			const pageAccountingIsValid =
				!resetMessageWindow &&
				current != null &&
				currentPageState != null &&
				loadedMessageCount(currentPageState.loadedPages) ===
					current.messages.length;
			if (!pageAccountingIsValid) {
				rememberInitialPage({
					...response,
					session: reconciledSession,
				});
			} else {
				const olderPages = currentPageState.loadedPages.slice(1);
				const olderCount = loadedMessageCount(olderPages);
				const latestCount = Math.max(0, messages.length - olderCount);
				const preservePagingCursor =
					currentPageState.loading || olderPages.length > 0;
				const reconciledPageState = {
					nextCursor: preservePagingCursor
						? currentPageState.nextCursor
						: (response.initialPage?.nextCursor ?? currentPageState.nextCursor),
					hasMore: preservePagingCursor
						? currentPageState.hasMore
						: (response.initialPage?.hasMore ?? currentPageState.hasMore),
					loading: currentPageState.loading,
					loadedPages:
						messages.length === 0
							? []
							: [{ requestCursor: null, count: latestCount }, ...olderPages],
				};
				if (currentPageState.loading) {
					Object.assign(currentPageState, reconciledPageState);
				} else {
					pageStateRef.current[sessionId] = reconciledPageState;
				}
			}
			dispatch({ type: "UPSERT_SESSION", session: reconciledSession });
			dispatchSessionMeta(dispatch, sessionId, response);
			sessionsNeedingReconciliationRef.current.delete(sessionId);
			if (options.refreshWorkspace === true) {
				dispatchWorkspaceTreeRefresh(response.session.worktreePath);
			}
			return true;
		},
		[rememberInitialPage],
	);
	const readSessionFromAuthority = useCallback(
		async (
			sessionId: string,
			options: {
				requireViewable?: boolean;
				refreshWorkspace?: boolean;
				resetMessageWindow?: boolean;
			} = {},
		): Promise<{ response: GetSessionResponse | null; applied: boolean }> => {
			const token = beginSessionAuthorityRead(sessionId);
			const response = await getSession(sessionId);
			return {
				response,
				applied:
					response != null &&
					applySessionAuthorityResponse(sessionId, token, response, options),
			};
		},
		[applySessionAuthorityResponse, beginSessionAuthorityRead],
	);
	const reconcileSessionFromAuthority = useCallback(
		async (sessionId: string): Promise<void> => {
			try {
				await readSessionFromAuthority(sessionId, {
					requireViewable: true,
					refreshWorkspace: true,
				});
			} catch (error) {
				console.error("Failed to reconcile visible agent session:", error);
			}
		},
		[readSessionFromAuthority],
	);
	const viewableRegistry = useMemo<ViewableSessionRegistry>(
		() => ({
			register: (sessionId: string) => {
				const controller = ensureNoticeController(
					sessionNoticeRequestControllersRef,
					sessionId,
				);
				void getAgentSessionNotice(sessionId)
					.then((snapshot) => {
						if (controller.signal.aborted) return;
						dispatchNoticeSnapshot(dispatch, snapshot);
					})
					.catch((error) => {
						console.error("Failed to query agent session notice:", error);
					});
				touchSessionAccess(sessionId);
				const map = viewableIdsRef.current;
				map.set(sessionId, (map.get(sessionId) ?? 0) + 1);
				const mirrorMayBeStale =
					sessionsNeedingReconciliationRef.current.has(sessionId) ||
					(pendingQueuesRef.current[sessionId]?.length ?? 0) > 0 ||
					(turnPhasesRef.current[sessionId] ?? "idle") !== "idle";
				if (mirrorMayBeStale) {
					void reconcileSessionFromAuthority(sessionId);
				}
				return () => {
					const m = viewableIdsRef.current;
					const next = (m.get(sessionId) ?? 0) - 1;
					if (next <= 0) {
						m.delete(sessionId);
						sessionReadbackGenerationsRef.current.set(
							sessionId,
							(sessionReadbackGenerationsRef.current.get(sessionId) ?? 0) + 1,
						);
						if (!sessionsByIdRef.current[sessionId]) {
							sessionNoticeRequestControllersRef.current
								.get(sessionId)
								?.abort();
							sessionNoticeRequestControllersRef.current.delete(sessionId);
						}
						evictInactiveSessionsRef.current();
					} else {
						m.set(sessionId, next);
					}
				};
			},
			getIds: () => new Set(viewableIdsRef.current.keys()),
		}),
		[touchSessionAccess, reconcileSessionFromAuthority],
	);

	const dispatchWithMessageWindowTracking = useCallback(
		(action: AgentChatAction) => {
			const bodyMutationSessionId =
				action.type === "UPSERT_SESSION"
					? action.session.id
					: action.type === "ADD_MESSAGE" ||
							action.type === "SET_STREAMING_MESSAGE" ||
							action.type === "APPLY_STREAMING_DELTA" ||
							action.type === "MARK_AGENT_TURN_COMPLETED"
						? action.sessionId
						: null;
			if (bodyMutationSessionId) {
				sessionBodyGenerationsRef.current.set(
					bodyMutationSessionId,
					(sessionBodyGenerationsRef.current.get(bodyMutationSessionId) ?? 0) +
						1,
				);
			}
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
					const nextSession =
						sessions.length > 0
							? sessions[
									Math.min(Math.max(previousIndex, 0), sessions.length - 1)
								]
							: null;
					if (nextSession) {
						const { response } = await readSessionFromAuthority(
							nextSession.id,
							{ resetMessageWindow: true },
						);
						if (activeSessionIdRef.current === previousActiveSessionId) {
							if (response) {
								dispatch({
									type: "SET_ACTIVE_SESSION_ID",
									sessionId: response.session.id,
								});
							} else {
								dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
							}
						}
					} else if (activeSessionIdRef.current === previousActiveSessionId) {
						dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
					}
				}
				return sessions;
			} catch (error) {
				console.error("Failed to refresh agent sessions:", error);
				return [];
			}
		},
		[readSessionFromAuthority],
	);

	const refreshClosedSessions = useCallback(async () => {
		try {
			const sessions = await listClosedSessions(worktreePathRef.current);
			dispatch({ type: "SET_CLOSED_SESSIONS", sessions });
		} catch (error) {
			console.error("Failed to refresh closed agent sessions:", error);
		}
	}, []);

	const selectSession = useCallback(
		async (sessionId: string) => {
			const selectionIntent = sessionSelectionIntentRef.current + 1;
			sessionSelectionIntentRef.current = selectionIntent;
			try {
				const { response, applied } = await readSessionFromAuthority(
					sessionId,
					{
						resetMessageWindow: true,
					},
				);
				if (sessionSelectionIntentRef.current !== selectionIntent) return;
				if (response) {
					if (applied) {
						await clearSessionError(
							dispatch,
							sessionNoticeRequestControllersRef,
							sessionId,
							"load_session",
						);
					}
					if (sessionSelectionIntentRef.current !== selectionIntent) return;
					dispatch({
						type: "SET_ACTIVE_SESSION_ID",
						sessionId: response.session.id,
					});
				} else {
					dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
				}
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"load_session",
					`セッションの読み込みに失敗: ${e}`,
				);
			}
		},
		[readSessionFromAuthority],
	);

	const loadSession = useCallback(
		async (sessionId: string): Promise<ChatSession | null> => {
			try {
				const { response, applied } = await readSessionFromAuthority(
					sessionId,
					{
						resetMessageWindow: true,
					},
				);
				if (response && applied) {
					await clearSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"load_session",
					);
					return response.session;
				}
				return sessionsByIdRef.current[sessionId] ?? null;
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"load_session",
					`session の読み込みに失敗: ${e}`,
				);
				throw e;
			}
		},
		[readSessionFromAuthority],
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
				await clearSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"load_older",
				);
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
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"load_older",
					`過去メッセージの読み込みに失敗: ${e}`,
				);
			}
		},
		[touchSessionAccess],
	);

	const interrupt = useCallback((sessionId: string) => {
		if (!sessionId) return;
		const session = sessionsByIdRef.current[sessionId];
		if (!session?.sessionRevision || !session.activeTurnId) {
			console.error(
				"Cannot stop a session without a durable revision and turn identity",
			);
			return;
		}
		// 楽観的に interrupting 状態へ。turn が idle になった時点で reducer が
		// 自動クリアする。これで停止押下が即座に UI へ反映される。
		dispatch({ type: "SET_INTERRUPTING", sessionId, value: true });
		requestAgentStop(
			sessionId,
			session.activeTurnId,
			session.sessionRevision,
		).catch((e) => {
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
			if (!trimmed && (!images || images.length === 0)) return false;

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
				const workflowApprovalExecutionId =
					workflowApprovalExecutionIdRef.current;
				const response =
					sessionId &&
					workflowApprovalChatSessionId === sessionId &&
					workflowApprovalExecutionId
						? await sendWorkflowApprovalChatMessage(
								workflowApprovalExecutionId,
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
				if (response.type !== "accepted") return false;
				const responseSessionId = response.operation.receipt.session_id;
				await clearSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"send",
				);
				touchSessionAccess(responseSessionId);
				// Accepted is the composer-clear boundary. Projection refresh is
				// a readback only; failure never restores or automatically resends input.
				void readSessionFromAuthority(responseSessionId, {
					refreshWorkspace: true,
				}).catch((error) => {
					console.error("Failed to read accepted send projection:", error);
				});
				if (sessionId === null && options?.activateNewSession !== false) {
					dispatch({
						type: "SET_ACTIVE_SESSION_ID",
						sessionId: responseSessionId,
					});
				}
				return true;
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"send",
					`メッセージ送信に失敗: ${e}`,
				);
				return false;
			}
		},
		[
			resolvePermissionModeForSessionRef,
			resolvePlanModeForSessionRef,
			readSessionFromAuthority,
			touchSessionAccess,
		],
	);

	const cancelQueuedTurn = useCallback(
		async (sessionId: string, queuedTurnId?: string | null) => {
			try {
				const response = await cancelAgentQueuedTurn(sessionId, queuedTurnId);
				await clearSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"cancel_queue",
				);
				dispatch({
					type: "SET_PENDING_QUEUE",
					sessionId: response.sessionId,
					queue: response.pendingQueue,
				});
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"cancel_queue",
					`キューのキャンセルに失敗: ${e}`,
				);
			}
		},
		[],
	);

	const resumeQueue = useCallback(async (sessionId: string) => {
		try {
			await resumeAgentQueue(sessionId);
			await clearSessionError(
				dispatch,
				sessionNoticeRequestControllersRef,
				sessionId,
				"resume_queue",
			);
		} catch (e) {
			await setSessionError(
				dispatch,
				sessionNoticeRequestControllersRef,
				sessionId,
				"resume_queue",
				`キューの再開に失敗: ${e}`,
			);
		}
	}, []);

	const removeOpenSession = useCallback(
		async (
			sessionId: string,
			mutation: (sessionId: string) => Promise<void>,
			operation: "close_session" | "archive_session",
			failureLabel: string,
		) => {
			const sessions = sessionsRef.current;
			const idx = sessions.findIndex((s) => s.id === sessionId);
			const isActive = activeSessionIdRef.current === sessionId;
			try {
				await mutation(sessionId);
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					operation,
					`${failureLabel}: ${e}`,
				);
				return;
			}

			cleanupSessionMirror(
				dispatch,
				sessionsByIdRef,
				sessionNoticeRequestControllersRef,
				sessionId,
			);

			if (isActive) {
				const remaining = sessions.filter((s) => s.id !== sessionId);
				const nextSession =
					remaining.length > 0
						? remaining[Math.min(idx, remaining.length - 1)]
						: null;
				if (nextSession) {
					try {
						const { response } = await readSessionFromAuthority(
							nextSession.id,
							{ resetMessageWindow: true },
						);
						if (response) {
							dispatch({
								type: "SET_ACTIVE_SESSION_ID",
								sessionId: response.session.id,
							});
						} else {
							dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
						}
					} catch (e) {
						await setSessionError(
							dispatch,
							sessionNoticeRequestControllersRef,
							nextSession.id,
							"load_session",
							`セッションの読み込みに失敗: ${e}`,
						);
						dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
					}
				} else {
					dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
				}
			}

			await refreshSessions();
			await refreshClosedSessions();
		},
		[refreshSessions, refreshClosedSessions, readSessionFromAuthority],
	);

	const closeSessionFn = useCallback(
		(sessionId: string) =>
			removeOpenSession(
				sessionId,
				closeSessionApi,
				"close_session",
				"セッションクローズに失敗",
			),
		[removeOpenSession],
	);

	const restoreSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				await restoreSessionApi(sessionId);
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"restore_session",
					`セッション復元に失敗: ${e}`,
				);
				return;
			}
			await clearSessionError(
				dispatch,
				sessionNoticeRequestControllersRef,
				sessionId,
				"restore_session",
			);
			try {
				const { response } = await readSessionFromAuthority(sessionId, {
					resetMessageWindow: true,
				});
				if (response) {
					dispatch({
						type: "SET_ACTIVE_SESSION_ID",
						sessionId: response.session.id,
					});
				} else {
					dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: null });
				}
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"load_session",
					`セッションの読み込みに失敗: ${e}`,
				);
			}
			await refreshSessions();
			await refreshClosedSessions();
		},
		[refreshSessions, refreshClosedSessions, readSessionFromAuthority],
	);

	const archiveSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				await archiveSessionApi(sessionId);
				await refreshClosedSessions();
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"archive_session",
					`セッションアーカイブに失敗: ${e}`,
				);
			}
		},
		[refreshClosedSessions],
	);

	const archiveOpenSessionFn = useCallback(
		(sessionId: string) =>
			removeOpenSession(
				sessionId,
				archiveOpenSessionApi,
				"archive_session",
				"セッションアーカイブに失敗",
			),
		[removeOpenSession],
	);

	const forkSessionFn = useCallback(
		async (sessionId: string) => {
			let forked: ChatSession;
			try {
				forked = await forkSessionApi(sessionId);
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"fork_session",
					`セッションのフォークに失敗: ${e}`,
				);
				return;
			}
			dispatch({ type: "UPSERT_SESSION", session: forked });
			dispatch({ type: "SET_ACTIVE_SESSION_ID", sessionId: forked.id });
			dispatch({
				type: "SET_PERMISSION_MODE",
				sessionId: forked.id,
				mode: forked.permissionMode,
			});
			await clearSessionError(
				dispatch,
				sessionNoticeRequestControllersRef,
				sessionId,
				"fork_session",
			);
			try {
				await readSessionFromAuthority(forked.id, {
					resetMessageWindow: true,
				});
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					forked.id,
					"load_session",
					`セッションの読み込みに失敗: ${e}`,
				);
			}
			await refreshSessions();
		},
		[refreshSessions, readSessionFromAuthority],
	);

	const setSessionTitleFn = useCallback(
		async (sessionId: string, title: string | null): Promise<string> => {
			try {
				const summary = await setSessionTitleApi(sessionId, title);
				await refreshSessions();
				await refreshClosedSessions();
				await clearSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"set_title",
				);
				return summary.firstMessage || DEFAULT_SESSION_TITLE;
			} catch (e) {
				await setSessionError(
					dispatch,
					sessionNoticeRequestControllersRef,
					sessionId,
					"set_title",
					`セッションタイトル変更に失敗: ${e}`,
				);
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
			const { response } = await readSessionFromAuthority(session.id, {
				resetMessageWindow: true,
			});
			const activeSession = response?.session ?? session;
			if (!response) {
				dispatch({ type: "UPSERT_SESSION", session: activeSession });
			}
			dispatch({
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: activeSession.id,
			});
			dispatch({
				type: "SET_PERMISSION_MODE",
				sessionId: activeSession.id,
				mode: activeSession.permissionMode,
			});
			await refreshSessions();
			return activeSession.id;
		} catch (error) {
			console.error("Failed to create agent session:", error);
			return null;
		}
	}, [refreshSessions, readSessionFromAuthority]);

	const createNewWorkspaceSession = useCallback(
		async (requestId: string): Promise<string> => {
			const activeSessionSnapshot = activeSessionIdRef.current
				? sessionsByIdRef.current[activeSessionIdRef.current]
				: undefined;
			const backendId =
				activeSessionSnapshot?.backendId ?? selectedBackendIdRef.current;
			const modelId = activeSessionSnapshot
				? (sessionModelsRef.current[activeSessionSnapshot.id] ?? null)
				: null;
			const sessionId = await createWorkspaceSession(
				requestId,
				worktreePathRef.current,
				permissionModeRef.current,
				backendId,
				modelId,
			);
			const { response } = await readSessionFromAuthority(sessionId, {
				resetMessageWindow: true,
			});
			if (!response) {
				throw new Error(`Created Session is unavailable: ${sessionId}`);
			}
			const activeSession = response.session;
			dispatch({
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: activeSession.id,
			});
			dispatch({
				type: "SET_PERMISSION_MODE",
				sessionId: activeSession.id,
				mode: activeSession.permissionMode,
			});
			await refreshSessions();
			return activeSession.id;
		},
		[refreshSessions, readSessionFromAuthority],
	);

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
			if (!sessionId) {
				if (isViewable) {
					dispatch({ type: "SET_PERMISSION_MODE", sessionId, mode });
				}
				return;
			}
			invoke("set_agent_permission_mode", {
				chatSessionId: sessionId,
				permissionMode: mode,
			})
				.then(() => {
					if (isViewable) {
						dispatch({ type: "SET_PERMISSION_MODE", sessionId, mode });
					}
				})
				.catch((e) => {
					console.error("Failed to set agent permission mode:", e);
				});
		},
		[],
	);

	const setPlanMode = useCallback(
		(sessionId: string | null, enabled: PlanMode) => {
			const isViewable =
				sessionId === null || viewableIdsRef.current.has(sessionId);
			if (!sessionId) {
				if (isViewable) {
					dispatch({ type: "SET_PLAN_MODE", sessionId, enabled });
				}
				return;
			}
			invoke("set_agent_plan_mode", {
				chatSessionId: sessionId,
				planMode: enabled,
			})
				.then(() => {
					if (isViewable) {
						dispatch({ type: "SET_PLAN_MODE", sessionId, enabled });
					}
				})
				.catch((e) => {
					console.error("Failed to set agent plan mode:", e);
				});
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
			respondAgentPermission(sessionId, requestId, allow, updatedInput)
				.then(async (result) => {
					const status = result.operation.latest_status.type;
					if (status === "reconciliation_required" || status === "failed") {
						await setSessionError(
							dispatch,
							sessionNoticeRequestControllersRef,
							sessionId,
							"respond_permission",
							`パーミッション応答は ${status} です`,
						);
						return;
					}
					await clearSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"respond_permission",
					);
				})
				.catch(async (e) => {
					console.error("Failed to respond to permission:", e);
					await setSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"respond_permission",
						`パーミッション応答に失敗: ${e}`,
					);
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
		const selectedBackend = selectedModel
			? getModelInfoBackend(selectedModel)
			: "";
		const currentBackend = sessionsByIdRef.current[sessionId]?.backendId ?? "";
		const persistSelectedModel = () =>
			invoke("set_agent_model", {
				chatSessionId: sessionId,
				modelId: normalizedModelId,
			});

		if (
			selectedBackend &&
			currentBackend &&
			selectedBackend !== currentBackend
		) {
			void setSessionBackend(sessionId, selectedBackend)
				.then(async (response) => {
					const switchedModel = normalizeModelSelectionId(
						response.availableModels,
						response.selectedModel,
					);
					if (switchedModel !== normalizedModelId) {
						await persistSelectedModel();
					}
					await clearSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"set_backend",
					);
					dispatch({
						type: "SET_SESSION_MODEL",
						sessionId,
						modelId: normalizedModelId,
						backendId: selectedBackend,
					});
				})
				.catch(async (e) => {
					await setSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"set_backend",
						`Agent の変更に失敗: ${e}`,
					);
				});
			return;
		}

		persistSelectedModel()
			.then(() => {
				dispatch({
					type: "SET_SESSION_MODEL",
					sessionId,
					modelId: normalizedModelId,
					backendId: selectedBackend || undefined,
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
			const token = beginSessionAuthorityRead(sessionId);
			setSessionBackend(sessionId, backendId)
				.then(async (response) => {
					await clearSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"set_backend",
					);
					if (activeSessionIdRef.current === sessionId) {
						applySessionAuthorityResponse(sessionId, token, response, {
							resetMessageWindow: true,
						});
					}
				})
				.catch(async (e) => {
					await setSessionError(
						dispatch,
						sessionNoticeRequestControllersRef,
						sessionId,
						"set_backend",
						`Agent の変更に失敗: ${e}`,
					);
				});
		},
		[applySessionAuthorityResponse, beginSessionAuthorityRead],
	);

	const getStreamingDeltaDropReason = useCallback(
		(sessionId: string, messageId: string) => {
			const session = sessionsByIdRef.current[sessionId];
			if (!session) return "missing_session";
			return session.messages.some((message) => message.id === messageId)
				? null
				: "missing_message";
		},
		[],
	);

	useAgentSdkListeners({
		dispatch: dispatchWithMessageWindowTracking,
		viewableRegistry,
		refreshSessions,
		reconcileSession: reconcileSessionFromAuthority,
		markSessionForReconciliation,
		getStreamingDeltaDropReason,
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
		const initGeneration = initSessionsGenerationRef.current + 1;
		initSessionsGenerationRef.current = initGeneration;
		const requestedWorktreePath = worktreePathRef.current;
		const initAuthorityEpoch = sessionAuthorityReadEpochRef.current + 1;
		sessionAuthorityReadEpochRef.current = initAuthorityEpoch;
		try {
			const response = await initAgentSessions(requestedWorktreePath);
			if (
				initSessionsGenerationRef.current !== initGeneration ||
				worktreePathRef.current !== requestedWorktreePath
			) {
				return;
			}
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
				const sessionId = response.activeSession.session.id;
				dispatch({
					type: "SET_ACTIVE_SESSION_ID",
					sessionId,
				});
				const laterSessionReadEpoch =
					sessionAuthorityReadEpochsRef.current.get(sessionId) ?? 0;
				if (laterSessionReadEpoch <= initAuthorityEpoch) {
					const token = beginSessionAuthorityRead(sessionId);
					applySessionAuthorityResponse(
						sessionId,
						token,
						response.activeSession,
						{ resetMessageWindow: true },
					);
				}
			}
		} catch (error) {
			console.error("Failed to initialize agent sessions:", error);
		}
	}, [applySessionAuthorityResponse, beginSessionAuthorityRead]);

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
	const getSessionNotice = useCallback(
		(sessionId: string): SessionNotice | null =>
			worktreeSessionStatuses.get(sessionId)?.notice ?? null,
		[worktreeSessionStatuses],
	);

	const activityStatus = useMemo(
		() => deriveActivityStatus(activeSession?.messages, activeTurnPhase),
		[activeSession?.messages, activeTurnPhase],
	);

	const turnPhasesState = state.turnPhases;
	const sessionModelsState = state.sessionModels;
	const pendingQueuesState = state.pendingQueues;
	const queuePausedState = state.queuePaused;
	const stallObservationsState = state.stallObservations ?? {};
	const latestTokenUsageState = state.latestTokenUsage;
	const runtimeSlashCommandsState = state.runtimeSlashCommands;
	const canChangeBackendState = state.canChangeBackend;
	const sessionsByIdState = state.sessionsById;
	const sessionErrorsState = state.sessionErrors;
	const getSessionError = useCallback(
		(sessionId: string): string | null => sessionErrorsState[sessionId] ?? null,
		[sessionErrorsState],
	);
	const dismissSessionError = useCallback((sessionId: string) => {
		void syncSessionError(
			dispatch,
			sessionNoticeRequestControllersRef,
			sessionId,
			{ action: "dismiss" },
		);
	}, []);
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
	const getSessionCanChangeBackend = useCallback(
		(sessionId: string): boolean => canChangeBackendState[sessionId] ?? false,
		[canChangeBackendState],
	);
	const pendingPermissionsState = state.pendingPermissions;
	const getSessionPendingPermission = useCallback(
		(sessionId: string): PermissionRequest | null =>
			pendingPermissionsState[sessionId] ?? null,
		[pendingPermissionsState],
	);
	const getSessionPendingQueue = useCallback(
		(sessionId: string): QueuedAgentTurn[] =>
			pendingQueuesState[sessionId] ?? [],
		[pendingQueuesState],
	);
	const getSessionQueuePaused = useCallback(
		(sessionId: string): boolean => queuePausedState[sessionId] ?? false,
		[queuePausedState],
	);
	const getSessionStallObservation = useCallback(
		(sessionId: string): AgentStallObservation | null =>
			stallObservationsState[sessionId] ?? null,
		[stallObservationsState],
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
		createNewWorkspaceSession,
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
		getSessionError,
		dismissSessionError,
		getSessionTurnPhase,
		getSessionInterrupting,
		getSessionPermissionMode,
		getSessionPlanMode,
		getSessionSelectedModel,
		getSessionCanChangeBackend,
		getSessionPendingPermission,
		getSessionPendingQueue,
		getSessionQueuePaused,
		getSessionStallObservation,
		getSessionNotice,
		getSessionLatestTokenUsage,
		getSessionRuntimeSlashCommands,
		cancelQueuedTurn,
		resumeQueue,
	};
}
