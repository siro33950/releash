import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useReducer, useRef } from "react";
import type {
	ChatSession,
	PermissionMode,
	PermissionRequest,
	SessionSummary,
} from "@/types/session";
import { INITIAL_STATE, reducer } from "./agentChatReducer";
import { useAgentPtyListeners } from "./useAgentPtyListeners";
import {
	addMessage,
	createSession,
	getSession,
	listSessions,
	updateSessionAgentInfo,
} from "./useSessionStore";

export interface UseAgentChatResult {
	sessions: SessionSummary[];
	activeSession: ChatSession | null;
	isStreaming: boolean;
	error: string | null;
	permissionMode: PermissionMode;
	pendingPermission: PermissionRequest | null;
	sendMessage: (content: string) => Promise<void>;
	interrupt: () => void;
	selectSession: (sessionId: string) => Promise<void>;
	refreshSessions: () => Promise<void>;
	clearActiveSession: () => void;
	setPermissionMode: (mode: PermissionMode) => void;
	respondPermission: (
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
}

export function useAgentChat(worktreePath: string): UseAgentChatResult {
	const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
	const streamingMessageIdRef = useRef<string | null>(null);
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const activeSessionRef = useRef(state.activeSession);
	activeSessionRef.current = state.activeSession;
	const isRetryingRef = useRef(false);
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
			streamingMessageIdRef.current = agentMsg.id;
			dispatch({ type: "SET_STREAMING", streaming: true });

			invoke("execute_agent_query", {
				prompt,
				sessionId: agentSessionId,
				cwd: worktreePathRef.current,
				permissionMode: permissionModeRef.current,
			}).catch((e) => {
				console.error("execute_agent_query failed:", e);
				dispatch({ type: "SET_STREAMING", streaming: false });
				streamingMessageIdRef.current = null;
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

				isRetryingRef.current = false;

				await startQuery(session.id, trimmed, session.agentSessionId || null);

				await refreshSessions();
			} catch (e) {
				dispatch({ type: "SET_STREAMING", streaming: false });
				dispatch({
					type: "SET_ERROR",
					error: `メッセージ送信に失敗: ${e}`,
				});
			}
		},
		[refreshSessions, startQuery],
	);

	const clearActiveSession = useCallback(() => {
		dispatch({ type: "SET_ACTIVE_SESSION", session: null });
	}, []);

	const interrupt = useCallback(() => {
		invoke("interrupt_agent_query").catch((e) => {
			console.error("Failed to interrupt agent query:", e);
		});
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
			invoke("respond_agent_permission", {
				requestId,
				behavior: allow ? "allow" : "deny",
				message: allow ? null : "User denied",
				updatedInput: updatedInput ? JSON.stringify(updatedInput) : null,
			}).catch((e) => {
				console.error("Failed to respond to permission:", e);
			});
			dispatch({ type: "SET_PENDING_PERMISSION", request: null });
		},
		[],
	);

	const handleRetry = useCallback(
		async (content: string) => {
			const session = activeSessionRef.current;
			if (!session || isRetryingRef.current) return;

			isRetryingRef.current = true;

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
		streamingMessageIdRef,
		activeSessionRef,
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

	return {
		sessions: state.sessions,
		activeSession: state.activeSession,
		isStreaming: state.isStreaming,
		error: state.error,
		permissionMode: state.permissionMode,
		pendingPermission: state.pendingPermission,
		sendMessage,
		interrupt,
		selectSession,
		refreshSessions,
		clearActiveSession,
		setPermissionMode,
		respondPermission,
	};
}
