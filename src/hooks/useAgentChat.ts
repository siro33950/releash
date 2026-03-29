import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	ChatSession,
	PermissionMode,
	PermissionRequest,
	SessionSummary,
} from "@/types/session";
import { INITIAL_STATE, reducer } from "./agentChatReducer";
import { useAgentSdkListeners } from "./useAgentSdkListeners";
import {
	addMessage,
	closeSession as closeSessionApi,
	createSession,
	getSession,
	listClosedSessions,
	listSessions,
	restoreSession as restoreSessionApi,
} from "./useSessionStore";

export interface UseAgentChatResult {
	sessions: SessionSummary[];
	orderedSessions: SessionSummary[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	isStreaming: boolean;
	error: string | null;
	permissionMode: PermissionMode;
	sessionAgentStates: Map<string, AgentState>;
	sendMessage: (content: string) => Promise<void>;
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

function deriveAgentState(
	sessionId: string,
	streamingSessionIds: string[],
	pendingPermissions: Record<string, PermissionRequest>,
	sessionFinalStates: Record<string, "done" | "error">,
): AgentState {
	if (streamingSessionIds.includes(sessionId)) {
		return sessionId in pendingPermissions ? "waiting" : "running";
	}
	return sessionFinalStates[sessionId] ?? "done";
}

export function useAgentChat(worktreePath: string): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const activeSessionRef = useRef(state.activeSession);
	activeSessionRef.current = state.activeSession;
	const sessionsRef = useRef(state.sessions);
	sessionsRef.current = state.sessions;
	const permissionModeRef = useRef(state.permissionMode);
	permissionModeRef.current = state.permissionMode;

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
				// Sync streaming state from backend
				if (response.isStreaming) {
					dispatch({ type: "START_STREAMING", sessionId });
				} else {
					dispatch({ type: "STOP_STREAMING", sessionId });
				}
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

	const startQuery = useCallback(async (sessionId: string, prompt: string) => {
		const agentMsg = await addMessage(sessionId, "agent", "");
		dispatch({ type: "ADD_MESSAGE", message: agentMsg });
		dispatch({ type: "START_STREAMING", sessionId });

		invoke("execute_agent_query", {
			prompt,
			chatSessionId: sessionId,
			cwd: worktreePathRef.current,
			permissionMode: permissionModeRef.current,
			streamingMessageId: agentMsg.id,
		}).catch((e) => {
			console.error("execute_agent_query failed:", e);
			dispatch({ type: "STOP_STREAMING", sessionId });
			dispatch({
				type: "SET_ERROR",
				error: `エージェント実行に失敗: ${e}`,
			});
		});
	}, []);

	const sendMessage = useCallback(
		async (content: string) => {
			const trimmed = content.trim();
			if (!trimmed) return;

			try {
				let session = activeSessionRef.current;
				if (!session) {
					session = await createSession(worktreePathRef.current);
					dispatch({ type: "SET_ACTIVE_SESSION", session });
				}

				const message = await addMessage(session.id, "human", trimmed);
				dispatch({ type: "ADD_MESSAGE", message });

				await startQuery(session.id, trimmed);

				await refreshSessions();
			} catch (e) {
				const sessionId = activeSessionRef.current?.id;
				if (sessionId) {
					dispatch({ type: "STOP_STREAMING", sessionId });
				}
				dispatch({
					type: "SET_ERROR",
					error: `メッセージ送信に失敗: ${e}`,
				});
			}
		},
		[refreshSessions, startQuery],
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
				// Start Bridge process for the restored session
				startAgentProcess(
					sessionId,
					worktreePathRef.current,
					permissionModeRef.current,
				);
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
			const session = await createSession(worktreePathRef.current);
			dispatch({ type: "SET_ACTIVE_SESSION", session });
			// Prewarm: start agent process in background
			startAgentProcess(
				session.id,
				worktreePathRef.current,
				permissionModeRef.current,
			);
			await refreshSessions();
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション作成に失敗: ${e}`,
			});
		}
	}, [refreshSessions]);

	const interrupt = useCallback(() => {
		const sessionId = activeSessionRef.current?.id;
		if (!sessionId) return;
		invoke("interrupt_agent_query", { chatSessionId: sessionId }).catch((e) => {
			console.error("Failed to interrupt agent query:", e);
		});
	}, []);

	const reorderSessions = useCallback((sessionOrder: string[]) => {
		dispatch({ type: "REORDER_SESSIONS", sessionOrder });
	}, []);

	const setPermissionMode = useCallback((mode: PermissionMode) => {
		dispatch({ type: "SET_USER_PERMISSION_MODE", mode });
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

	useAgentSdkListeners({
		dispatch,
		activeSessionRef,
		refreshSessions,
	});

	const initSessions = useCallback(async () => {
		const sessions = await refreshSessions();
		// Start Bridge processes for all existing sessions
		for (const session of sessions) {
			startAgentProcess(
				session.id,
				worktreePathRef.current,
				permissionModeRef.current,
			);
		}
		if (sessions.length > 0) {
			await selectSession(sessions[0].id);
		} else {
			await createNewSession();
		}
	}, [refreshSessions, selectSession, createNewSession]);

	// Load sessions on mount
	useEffect(() => {
		initSessions();
	}, [initSessions]);

	// Reset when worktreePath changes
	const prevWorktreePathRef = useRef(worktreePath);
	useEffect(() => {
		if (prevWorktreePathRef.current !== worktreePath) {
			prevWorktreePathRef.current = worktreePath;
			dispatch({ type: "SET_ACTIVE_SESSION", session: null });
			initSessions();
			refreshClosedSessions();
		}
	}, [worktreePath, initSessions, refreshClosedSessions]);

	const isStreaming = state.streamingSessionIds.includes(
		state.activeSession?.id ?? "",
	);
	const orderedSessions = useMemo(() => {
		const sessionMap = new Map(state.sessions.map((s) => [s.id, s]));
		return state.sessionOrder
			.map((id) => sessionMap.get(id))
			.filter((s): s is SessionSummary => !!s);
	}, [state.sessions, state.sessionOrder]);

	const sessionAgentStates = useMemo(() => {
		const map = new Map<string, AgentState>();
		for (const s of state.sessions) {
			map.set(
				s.id,
				deriveAgentState(
					s.id,
					state.streamingSessionIds,
					state.pendingPermissions,
					state.sessionFinalStates,
				),
			);
		}
		return map;
	}, [
		state.sessions,
		state.streamingSessionIds,
		state.pendingPermissions,
		state.sessionFinalStates,
	]);

	return {
		sessions: state.sessions,
		orderedSessions,
		closedSessions: state.closedSessions,
		activeSession: state.activeSession,
		isStreaming,
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
	};
}
