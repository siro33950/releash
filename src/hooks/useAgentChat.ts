import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	SessionSummary,
	TurnPhase,
} from "@/types/session";
import {
	type AgentChatAction,
	INITIAL_STATE,
	reducer,
} from "./agentChatReducer";
import { useAgentSdkListeners } from "./useAgentSdkListeners";
import {
	closeSession as closeSessionApi,
	createSession,
	getSession,
	initAgentSessions,
	listAgentBackends,
	listClosedSessions,
	listSessions,
	restoreSession as restoreSessionApi,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
	setSessionBackend,
} from "./useSessionStore";
import { useWorktreeSessionStatuses } from "./useWorktreeSessionStatuses";

export type ActivityStatus = { label: string } | null;

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
	sendMessage: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
	) => Promise<void>;
	interrupt: () => void;
	selectSession: (sessionId: string) => Promise<void>;
	refreshSessions: () => Promise<SessionSummary[] | undefined>;
	refreshClosedSessions: () => Promise<void>;
	closeSession: (sessionId: string) => Promise<void>;
	restoreSession: (sessionId: string) => Promise<void>;
	createNewSession: () => Promise<void>;
	reorderSessions: (sessionOrder: string[]) => void;
	setPermissionMode: (mode: PermissionMode) => void;
	respondPermission: (
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
	availableModels: ModelInfo[];
	selectedModel: string | null;
	setModel: (modelId: string | null) => void;
	backends: BackendInfo[];
	selectedBackendId: string | null;
	setBackend: (backendId: string | null) => void;
}

function startAgentProcess(
	chatSessionId: string,
	cwd: string,
	permissionMode: string,
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
		session: { permissionMode?: PermissionMode };
		turnPhase: TurnPhase;
		selectedModel: string | null;
		availableModels: ModelInfo[];
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
	});
}

function deriveActivityStatus(
	messages: ChatMessage[] | undefined,
	turnPhase: TurnPhase,
): ActivityStatus {
	if (turnPhase === "idle") return null;
	if (!messages || messages.length === 0) return null;
	const lastMsg = messages[messages.length - 1];
	if (lastMsg.role !== "agent") return null;
	if (lastMsg.parts.length === 0) return { label: "Thinking..." };

	const lastPart = lastMsg.parts[lastMsg.parts.length - 1];
	switch (lastPart.type) {
		case "thinking":
			return { label: "Thinking..." };
		case "text":
			return { label: "Writing..." };
		case "tool_use": {
			const tool = (
				lastPart as { tool: string; input?: Record<string, unknown> }
			).tool;
			const filePath = (lastPart as { input?: Record<string, unknown> }).input
				?.file_path as string | undefined;
			const fileName = filePath?.split("/").pop();
			switch (tool) {
				case "Read":
					return {
						label: fileName ? `Reading ${fileName}` : "Reading file...",
					};
				case "Write":
					return {
						label: fileName ? `Writing ${fileName}` : "Writing file...",
					};
				case "Edit":
					return {
						label: fileName ? `Editing ${fileName}` : "Editing file...",
					};
				case "Bash":
					return { label: "Running command..." };
				case "Grep":
					return { label: "Searching..." };
				case "Glob":
					return { label: "Finding files..." };
				case "Task":
					return { label: "Running background task..." };
				case "WebFetch":
					return { label: "Fetching web content..." };
				case "WebSearch":
					return { label: "Searching the web..." };
				default:
					return { label: `Using ${tool}...` };
			}
		}
		case "tool_result":
			return { label: "Processing result..." };
		case "permission":
			return { label: "Waiting for permission..." };
		case "task_status":
			return { label: "Running background task..." };
		case "error":
			return null;
		default:
			return { label: "Working..." };
	}
}

export function useAgentChat(
	worktreePath: string,
	workflowApprovalChatSessionId: string | null = null,
): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const workflowApprovalChatSessionIdRef = useRef(
		workflowApprovalChatSessionId,
	);
	workflowApprovalChatSessionIdRef.current = workflowApprovalChatSessionId;
	const activeSessionRef = useRef(state.activeSession);
	activeSessionRef.current = state.activeSession;
	const sessionsRef = useRef(state.sessions);
	sessionsRef.current = state.sessions;
	const permissionModeRef = useRef(state.permissionMode);
	permissionModeRef.current = state.permissionMode;
	const turnPhasesRef = useRef(state.turnPhases);
	turnPhasesRef.current = state.turnPhases;
	const selectedBackendIdRef = useRef(state.selectedBackendId);
	selectedBackendIdRef.current = state.selectedBackendId;

	const refreshSessions = useCallback(async (): Promise<SessionSummary[]> => {
		try {
			const sessions = await listSessions(worktreePathRef.current);
			dispatch({ type: "SET_SESSIONS", sessions });
			return sessions;
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション一覧の取得に失敗: ${e}`,
			});
			return [];
		}
	}, []);

	const selectSession = useCallback(async (sessionId: string) => {
		try {
			const response = await getSession(sessionId);
			if (response) {
				dispatch({ type: "SET_ACTIVE_SESSION", session: response.session });
				dispatchSessionMeta(dispatch, sessionId, response);
			} else {
				dispatch({ type: "SET_ACTIVE_SESSION", session: null });
			}
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッションの読み込みに失敗: ${e}`,
			});
		}
	}, []);

	const interrupt = useCallback(() => {
		const sessionId = activeSessionRef.current?.id;
		if (!sessionId) return;
		invoke("interrupt_agent_query", { chatSessionId: sessionId }).catch((e) => {
			console.error("Failed to interrupt agent query:", e);
		});
	}, []);

	const sendMessage = useCallback(
		async (
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
		) => {
			const trimmed = content.trim();
			if (!trimmed && (!images || images.length === 0)) return;

			try {
				const sessionId = activeSessionRef.current?.id ?? null;
				const wPath = worktreePathRef.current;
				const pm = permissionModeRef.current;
				const backendId = sessionId ? null : selectedBackendIdRef.current;
				const workflowApprovalChatSessionId =
					workflowApprovalChatSessionIdRef.current;
				const response =
					sessionId && workflowApprovalChatSessionId === sessionId
						? await sendWorkflowApprovalChatMessage(
								sessionId,
								wPath,
								trimmed,
								pm,
								images,
								mentions,
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
				// Only update if the user hasn't switched to a different session during await
				const currentSessionId = activeSessionRef.current?.id ?? null;
				if (
					currentSessionId === sessionId ||
					currentSessionId === response.session.id
				) {
					dispatch({
						type: "SET_ACTIVE_SESSION",
						session: response.session,
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

	const closeSessionFn = useCallback(
		async (sessionId: string) => {
			try {
				const sessions = sessionsRef.current;
				const idx = sessions.findIndex((s) => s.id === sessionId);

				// Close agent process gracefully
				invoke("close_agent_session", {
					chatSessionId: sessionId,
				}).catch((e) => {
					console.error("Failed to close agent session:", e);
				});

				await closeSessionApi(sessionId);

				dispatch({ type: "CLEANUP_SESSION", sessionId });

				const isActive = activeSessionRef.current?.id === sessionId;
				if (isActive) {
					const remaining = sessions.filter((s) => s.id !== sessionId);
					const nextSession =
						remaining.length > 0
							? remaining[Math.min(idx, remaining.length - 1)]
							: null;
					if (nextSession) {
						const response = await getSession(nextSession.id);
						dispatch({
							type: "SET_ACTIVE_SESSION",
							session: response?.session ?? null,
						});
						if (response) {
							dispatchSessionMeta(dispatch, nextSession.id, response);
						}
					} else {
						dispatch({
							type: "SET_ACTIVE_SESSION",
							session: null,
						});
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
				await restoreSessionApi(sessionId);
				const response = await getSession(sessionId);
				dispatch({
					type: "SET_ACTIVE_SESSION",
					session: response?.session ?? null,
				});
				if (response) {
					dispatchSessionMeta(dispatch, sessionId, response);
					if (
						response.session.messages.length > 0 ||
						response.session.agentSessionId
					) {
						startAgentProcess(
							sessionId,
							worktreePathRef.current,
							response.session.permissionMode,
						);
					}
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

	const createNewSession = useCallback(async () => {
		try {
			const backendId =
				activeSessionRef.current?.backendId ?? selectedBackendIdRef.current;
			const session = await createSession(worktreePathRef.current, backendId);
			const response = await getSession(session.id);
			const activeSession = response?.session ?? session;
			dispatch({ type: "SET_ACTIVE_SESSION", session: activeSession });
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

	const setPermissionMode = useCallback((mode: PermissionMode) => {
		dispatch({ type: "SET_PERMISSION_MODE", mode });
		// Persist to Rust and sync to Bridge
		const sessionId = activeSessionRef.current?.id;
		if (sessionId) {
			invoke("set_agent_permission_mode", {
				chatSessionId: sessionId,
				permissionMode: mode,
			}).catch((e) => {
				console.error("Failed to set agent permission mode:", e);
			});
		}
	}, []);

	const respondPermission = useCallback(
		(
			requestId: string,
			allow: boolean,
			updatedInput?: Record<string, unknown>,
		) => {
			const sessionId = activeSessionRef.current?.id;
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

	const setModel = useCallback((modelId: string | null) => {
		const sessionId = activeSessionRef.current?.id;
		if (!sessionId) return;
		invoke("set_agent_model", {
			chatSessionId: sessionId,
			modelId,
		})
			.then(() => {
				if (activeSessionRef.current?.id === sessionId) {
					dispatch({
						type: "SET_SESSION_MODEL",
						sessionId,
						modelId,
					});
				}
			})
			.catch((e) => {
				console.error("Failed to set agent model:", e);
			});
	}, []);

	const setBackend = useCallback((backendId: string | null) => {
		const activeSession = activeSessionRef.current;
		if (!activeSession) {
			dispatch({ type: "SET_SELECTED_BACKEND", backendId });
			return;
		}
		if (
			!backendId ||
			activeSession.messages.length > 0 ||
			activeSession.agentSessionId
		) {
			return;
		}
		setSessionBackend(activeSession.id, backendId)
			.then((response) => {
				if (activeSessionRef.current?.id === activeSession.id) {
					dispatch({ type: "SET_ACTIVE_SESSION", session: response.session });
					dispatchSessionMeta(dispatch, activeSession.id, response);
				}
			})
			.catch((e) => {
				dispatch({
					type: "SET_ERROR",
					error: `Agent の変更に失敗: ${e}`,
				});
			});
	}, []);

	useAgentSdkListeners({
		dispatch,
		activeSessionRef,
		refreshSessions,
	});

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
			if (response.activeSession) {
				dispatch({
					type: "SET_ACTIVE_SESSION",
					session: response.activeSession.session,
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
			dispatch({ type: "SET_ACTIVE_SESSION", session: null });
			dispatch({ type: "SET_PERMISSION_MODE", mode: "acceptEdits" });
			initSessions();
			refreshClosedSessions();
		}
	}, [worktreePath, initSessions, refreshClosedSessions]);

	const activeTurnPhase: TurnPhase =
		state.turnPhases[state.activeSession?.id ?? ""] ?? "idle";
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
		() => deriveActivityStatus(state.activeSession?.messages, activeTurnPhase),
		[state.activeSession?.messages, activeTurnPhase],
	);

	const selectedModel =
		state.sessionModels[state.activeSession?.id ?? ""] ?? null;
	return {
		sessions: state.sessions,
		orderedSessions,
		closedSessions: state.closedSessions,
		activeSession: state.activeSession,
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
		restoreSession: restoreSessionFn,
		createNewSession,
		reorderSessions,
		setPermissionMode,
		respondPermission,
		availableModels: state.availableModels,
		selectedModel,
		setModel,
		backends: state.backends,
		selectedBackendId: state.activeSession
			? (state.activeSession?.backendId ?? state.selectedBackendId)
			: state.selectedBackendId,
		setBackend,
	};
}
