import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch, MutableRefObject } from "react";
import { useEffect } from "react";
import type {
	ChatSession,
	MessagePart,
	PermissionMode,
	PermissionRequest,
	SessionState,
} from "@/types/session";
import {
	type AgentChatAction,
	resolvePermissionMode,
} from "./agentChatReducer";
import { updateSessionState } from "./useSessionStore";
import { setSlashCommands } from "./useSlashCommands";

interface PermissionRequestMessage {
	type: "permission_request";
	session_id?: string;
	chat_session_id?: string;
	request_id: string;
	tool_name: string;
	input: Record<string, unknown>;
	tool_use_id: string;
	title?: string;
	display_name?: string;
	description?: string;
	decision_reason?: string;
}

type SdkMessage =
	| PermissionRequestMessage
	| {
			type: string;
			session_id?: string;
			chat_session_id?: string;
			parent_tool_use_id?: string | null;
			[key: string]: unknown;
	  };

interface QueryCompleted {
	exit_code: number;
	stderr: string;
	chat_session_id?: string;
}

interface StreamingMessageUpdated {
	chat_session_id: string;
	message_id: string;
	parts: MessagePart[];
}

interface StreamingStarted {
	chat_session_id: string;
}

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	activeSessionRef: MutableRefObject<ChatSession | null>;
	userPermissionModeRef: MutableRefObject<PermissionMode>;
	refreshSessions: () => Promise<unknown>;
}

function handleSupportedCommands(msg: SdkMessage): void {
	if (
		msg.type === "supported_commands" &&
		"commands" in msg &&
		Array.isArray(msg.commands)
	) {
		setSlashCommands(
			msg.commands as {
				name: string;
				description: string;
				argumentHint?: string;
			}[],
		);
	}
}

function handlePermissionRequest(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (msg.type !== "permission_request" || !chatSessionId) return;
	const prMsg = msg as PermissionRequestMessage;
	const req: PermissionRequest = {
		request_id: prMsg.request_id,
		tool_name: prMsg.tool_name,
		input: prMsg.input,
		tool_use_id: prMsg.tool_use_id,
		title: prMsg.title,
		display_name: prMsg.display_name,
		description: prMsg.description,
		decision_reason: prMsg.decision_reason,
	};
	dispatch({
		type: "SET_PENDING_PERMISSION",
		sessionId: chatSessionId,
		request: req,
	});
}

function handlePermissionModeSync(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	userPermissionModeRef: MutableRefObject<PermissionMode>,
): void {
	if (
		msg.type === "system" &&
		"permissionMode" in msg &&
		typeof msg.permissionMode === "string"
	) {
		const sdkMode = msg.permissionMode as PermissionMode;
		if (sdkMode === "default") {
			dispatch({ type: "RESTORE_USER_PERMISSION_MODE" });
			// Sync restored userPermissionMode back to Bridge
			if (chatSessionId) {
				const restoredMode = resolvePermissionMode(
					userPermissionModeRef.current,
				);
				invoke("set_agent_permission_mode", {
					chatSessionId,
					permissionMode: restoredMode,
				}).catch((e) => {
					console.error("Failed to sync restored permission mode:", e);
				});
			}
		} else {
			dispatch({ type: "SET_PERMISSION_MODE", mode: sdkMode });
		}
	}
}

function handleSystemMessage(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	activeSessionRef: MutableRefObject<ChatSession | null>,
): void {
	if (msg.type !== "system" || !chatSessionId) return;
	// task subtypes are handled by Rust accumulation
	const subtype = typeof msg.subtype === "string" ? msg.subtype : undefined;
	if (
		subtype === "task_started" ||
		subtype === "task_notification" ||
		subtype === "task_progress"
	)
		return;
	// Skip dispatching for non-active sessions (Rust persists these)
	if (activeSessionRef.current?.id !== chatSessionId) return;
	const text =
		typeof msg.message === "string"
			? msg.message
			: typeof msg.content === "string"
				? msg.content
				: null;
	if (text) {
		dispatch({
			type: "ADD_MESSAGE",
			message: {
				id: `system-${Date.now()}`,
				role: "system",
				parts: [{ type: "text", content: text }],
				timestamp: Date.now(),
			},
		});
	}
}

function handleResultErrors(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	activeSessionRef: MutableRefObject<ChatSession | null>,
): void {
	if (msg.type !== "result" || !chatSessionId) return;
	// Skip dispatching for non-active sessions (Rust persists these)
	if (activeSessionRef.current?.id !== chatSessionId) return;
	const resultMsg = msg as {
		type: "result";
		errors?: string[];
	};
	if (resultMsg.errors && resultMsg.errors.length > 0) {
		dispatch({
			type: "ADD_MESSAGE",
			message: {
				id: `system-error-${Date.now()}`,
				role: "agent",
				parts: [{ type: "error", content: resultMsg.errors.join("\n") }],
				timestamp: Date.now(),
			},
		});
	}
}

export function useAgentSdkListeners(refs: AgentSdkListenerRefs): void {
	const { dispatch, activeSessionRef, userPermissionModeRef, refreshSessions } =
		refs;

	// Listen to SDK messages for meta events (permissions, commands, system messages)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SdkMessage>("agent-sdk-message", (event) => {
			const msg = event.payload;
			const chatSessionId = msg.chat_session_id;

			handleSupportedCommands(msg);
			handlePermissionRequest(msg, chatSessionId, dispatch);
			handlePermissionModeSync(
				msg,
				chatSessionId,
				dispatch,
				userPermissionModeRef,
			);
			handleSystemMessage(msg, chatSessionId, dispatch, activeSessionRef);
			handleResultErrors(msg, chatSessionId, dispatch, activeSessionRef);
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, activeSessionRef, userPermissionModeRef]);

	// Listen to agent-streaming-updated from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<StreamingMessageUpdated>("agent-streaming-updated", (event) => {
			const { chat_session_id, message_id, parts } = event.payload;
			dispatch({
				type: "SET_STREAMING_MESSAGE",
				sessionId: chat_session_id,
				messageId: message_id,
				parts,
			});
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch]);

	// Listen to agent-streaming-started from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<StreamingStarted>("agent-streaming-started", (event) => {
			const { chat_session_id } = event.payload;
			dispatch({ type: "START_STREAMING", sessionId: chat_session_id });
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch]);

	// Listen to agent-query-completed for completion/error handling
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<QueryCompleted>("agent-query-completed", (event) => {
			const info = event.payload;
			const chatSessionId = info.chat_session_id;

			if (chatSessionId) {
				dispatch({
					type: "STOP_STREAMING",
					sessionId: chatSessionId,
				});
				dispatch({
					type: "SET_SESSION_FINAL_STATE",
					sessionId: chatSessionId,
					state: info.exit_code === 0 ? "done" : "error",
				});
				dispatch({
					type: "SET_PENDING_PERMISSION",
					sessionId: chatSessionId,
					request: null,
				});
			}

			// Update session state
			const session = activeSessionRef.current;
			const newState: SessionState = info.exit_code === 0 ? "idle" : "error";
			if (chatSessionId && session && session.id === chatSessionId) {
				dispatch({ type: "UPDATE_SESSION_STATE", state: newState });
			}

			if (chatSessionId) {
				updateSessionState(chatSessionId, newState).catch((e) =>
					console.error("Failed to update session state:", e),
				);
			}

			refreshSessions().catch((e) =>
				console.error("Failed to refresh sessions:", e),
			);
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, activeSessionRef, refreshSessions]);
}
