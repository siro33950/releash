import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type { AgentState } from "@/types/protocol";
import type {
	ChatMessage,
	ChatSession,
	ModelInfo,
	PermissionMode,
	SessionState,
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
	listClosedSessions,
	listSessions,
	restoreSession as restoreSessionApi,
	sendAgentMessage,
} from "./useSessionStore";

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
	availableModels: ModelInfo[];
	selectedModel: string | null;
	setModel: (modelId: string | null) => void;
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
		session: { permissionMode?: string };
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
	if (response.availableModels.length > 0) {
		dispatch({
			type: "SET_AVAILABLE_MODELS",
			models: response.availableModels,
		});
	}
}

function deriveAgentState(
	turnPhase: TurnPhase,
	sessionState?: SessionState,
): AgentState {
	switch (turnPhase) {
		case "streaming":
			return "running";
		case "waiting_permission":
			return "waiting";
		case "idle":
			return sessionState === "error" ? "error" : "done";
	}
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
	const turnPhasesRef = useRef(state.turnPhases);
	turnPhasesRef.current = state.turnPhases;

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

	const sendMessage = useCallback(async (content: string) => {
		const trimmed = content.trim();
		if (!trimmed) return;

		try {
			const isNewSession = !activeSessionRef.current;
			const response = await sendAgentMessage(
				activeSessionRef.current?.id ?? null,
				worktreePathRef.current,
				trimmed,
				permissionModeRef.current,
			);
			if (isNewSession) {
				dispatch({ type: "SET_ACTIVE_SESSION", session: response.session });
			} else {
				dispatch({ type: "ADD_MESSAGE", message: response.humanMessage });
				if (response.agentMessage) {
					dispatch({ type: "ADD_MESSAGE", message: response.agentMessage });
				}
			}
			dispatch({ type: "SET_SESSIONS", sessions: response.sessions });
		} catch (e) {
			dispatch({
				type: "SET_ERROR",
				error: `メッセージ送信に失敗: ${e}`,
			});
		}
	}, []);

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
					// Start Bridge process for the restored session
					startAgentProcess(
						sessionId,
						worktreePathRef.current,
						response.session.permissionMode,
					);
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
			const session = await createSession(worktreePathRef.current);
			dispatch({ type: "SET_ACTIVE_SESSION", session });
			dispatch({ type: "SET_PERMISSION_MODE", mode: session.permissionMode });
			// Prewarm: start agent process in background
			startAgentProcess(
				session.id,
				worktreePathRef.current,
				session.permissionMode,
			);
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
		}).catch((e) => {
			console.error("Failed to set agent model:", e);
		});
	}, []);

	useAgentSdkListeners({
		dispatch,
		activeSessionRef,
		refreshSessions,
	});

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

	const sessionAgentStates = useMemo(() => {
		const map = new Map<string, AgentState>();
		for (const s of state.sessions) {
			const phase: TurnPhase = state.turnPhases[s.id] ?? "idle";
			map.set(s.id, deriveAgentState(phase, s.state));
		}
		return map;
	}, [state.sessions, state.turnPhases]);

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
	};
}
