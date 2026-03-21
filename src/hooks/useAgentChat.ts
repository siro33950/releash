import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
	ChatSession,
	PermissionMode,
	PermissionRequest,
	SessionSummary,
} from "@/types/session";
import { INITIAL_STATE, reducer } from "./agentChatReducer";
import {
	type StreamingBuffer,
	useAgentPtyListeners,
} from "./useAgentPtyListeners";
import {
	addMessage,
	closeSession as closeSessionApi,
	createSession,
	getSession,
	listClosedSessions,
	listSessions,
	restoreSession as restoreSessionApi,
	updateSessionAgentInfo,
} from "./useSessionStore";

export interface UseAgentChatResult {
	sessions: SessionSummary[];
	orderedSessions: SessionSummary[];
	closedSessions: SessionSummary[];
	activeSession: ChatSession | null;
	isStreaming: boolean;
	error: string | null;
	permissionMode: PermissionMode;
	pendingPermission: PermissionRequest | null;
	sendMessage: (content: string) => Promise<void>;
	interrupt: () => void;
	selectSession: (sessionId: string) => Promise<void>;
	refreshSessions: () => Promise<void>;
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

export function useAgentChat(worktreePath: string): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const streamingMessageIdsRef = useRef<Map<string, string>>(new Map());
	const streamingBuffersRef = useRef<Map<string, StreamingBuffer>>(new Map());
	const lastPromptsRef = useRef<Map<string, string>>(new Map());
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const activeSessionRef = useRef(state.activeSession);
	activeSessionRef.current = state.activeSession;
	const sessionsRef = useRef(state.sessions);
	sessionsRef.current = state.sessions;
	const isRetryingRef = useRef<Set<string>>(new Set());
	const permissionModeRef = useRef(state.permissionMode);
	permissionModeRef.current = state.permissionMode;

	const refreshSessions = useCallback(async () => {
		try {
			const sessions = await listSessions(worktreePathRef.current);
			dispatch({ type: "SET_SESSIONS", sessions });
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッション一覧の取得に失敗: ${e}`,
			});
		}
	}, []);

	const selectSession = useCallback(async (sessionId: string) => {
		try {
			const session = await getSession(sessionId);
			if (session) {
				// Merge in-flight streaming data from ref buffer.
				// Backend returns the last-persisted snapshot, which lacks
				// content accumulated during an ongoing stream, so we overlay
				// the buffer's latest values onto the matching message.
				const buffer = streamingBuffersRef.current.get(sessionId);
				const msgId = streamingMessageIdsRef.current.get(sessionId);
				if (buffer && msgId) {
					session.messages = session.messages.map((m) =>
						m.id === msgId
							? {
									...m,
									parts: buffer.parts,
								}
							: m,
					);
				}
			}
			dispatch({ type: "SET_ACTIVE_SESSION", session });
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `セッションの読み込みに失敗: ${e}`,
			});
		}
	}, []);

	const startQuery = useCallback(
		async (
			sessionId: string,
			prompt: string,
			agentSessionId: string | null,
		) => {
			const agentMsg = await addMessage(sessionId, "agent", "");
			dispatch({ type: "ADD_MESSAGE", message: agentMsg });
			streamingMessageIdsRef.current.set(sessionId, agentMsg.id);
			// Keep a ref-based buffer alongside reducer state so that
			// streaming content is captured even for non-active sessions
			// (reducer only updates the active session's messages).
			// This buffer is the source-of-truth for persistence on
			// agent-query-completed and for merging in selectSession.
			streamingBuffersRef.current.set(sessionId, {
				parts: [],
			});
			lastPromptsRef.current.set(sessionId, prompt);
			dispatch({ type: "START_STREAMING", sessionId });

			invoke("execute_agent_query", {
				prompt,
				sessionId: agentSessionId,
				chatSessionId: sessionId,
				cwd: worktreePathRef.current,
				permissionMode: permissionModeRef.current,
			}).catch((e) => {
				console.error("execute_agent_query failed:", e);
				dispatch({ type: "STOP_STREAMING", sessionId });
				streamingMessageIdsRef.current.delete(sessionId);
				dispatch({
					type: "SET_ERROR",
					error: `エージェント実行に失敗: ${e}`,
				});
			});
		},
		[],
	);

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

				isRetryingRef.current.delete(session.id);

				await startQuery(session.id, trimmed, session.agentSessionId || null);

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

				await closeSessionApi(sessionId);

				const isActive = activeSessionRef.current?.id === sessionId;
				if (isActive) {
					const remaining = sessions.filter((s) => s.id !== sessionId);
					const nextSession =
						remaining.length > 0
							? remaining[Math.min(idx, remaining.length - 1)]
							: null;
					if (nextSession) {
						const full = await getSession(nextSession.id);
						dispatch({ type: "SET_ACTIVE_SESSION", session: full });
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
				const full = await getSession(sessionId);
				dispatch({ type: "SET_ACTIVE_SESSION", session: full });
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
			});
			dispatch({
				type: "SET_PENDING_PERMISSION",
				sessionId,
				request: null,
			});
			const msgId = streamingMessageIdsRef.current.get(sessionId);
			if (msgId) {
				const status = allow ? "allowed" : "denied";
				const answers =
					updatedInput?.answers && typeof updatedInput.answers === "object"
						? (updatedInput.answers as Record<string, string>)
						: undefined;
				dispatch({
					type: "RESOLVE_PERMISSION_PART",
					messageId: msgId,
					requestId,
					status,
					answers,
				});
			}
		},
		[],
	);

	const handleRetry = useCallback(
		async (content: string, chatSessionId: string) => {
			const session = activeSessionRef.current;
			if (
				!session ||
				session.id !== chatSessionId ||
				isRetryingRef.current.has(chatSessionId)
			)
				return;

			isRetryingRef.current.add(chatSessionId);

			dispatch({ type: "SET_AGENT_SESSION_ID", agentSessionId: null });
			await updateSessionAgentInfo(session.id, null).catch((e) =>
				console.error("Failed to clear agent session id:", e),
			);

			await startQuery(session.id, content, null);
		},
		[startQuery],
	);

	useAgentPtyListeners({
		dispatch,
		streamingMessageIdsRef,
		activeSessionRef,
		streamingBuffersRef,
		lastPromptsRef,
		refreshSessions,
		handleRetry,
		isRetryingRef,
	});

	// Load sessions on mount
	useEffect(() => {
		refreshSessions();
	}, [refreshSessions]);

	// Reset when worktreePath changes
	const prevWorktreePathRef = useRef(worktreePath);
	useEffect(() => {
		if (prevWorktreePathRef.current !== worktreePath) {
			prevWorktreePathRef.current = worktreePath;
			dispatch({ type: "SET_ACTIVE_SESSION", session: null });
			refreshSessions();
		}
	}, [worktreePath, refreshSessions]);

	const isStreaming = state.streamingSessionIds.includes(
		state.activeSession?.id ?? "",
	);
	const pendingPermission = state.activeSession?.id
		? (state.pendingPermissions[state.activeSession.id] ?? null)
		: null;

	const orderedSessions = useMemo(() => {
		const sessionMap = new Map(state.sessions.map((s) => [s.id, s]));
		return state.sessionOrder
			.map((id) => sessionMap.get(id))
			.filter((s): s is SessionSummary => !!s);
	}, [state.sessions, state.sessionOrder]);

	return {
		sessions: state.sessions,
		orderedSessions,
		closedSessions: state.closedSessions,
		activeSession: state.activeSession,
		isStreaming,
		error: state.error,
		permissionMode: state.permissionMode,
		pendingPermission,
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
