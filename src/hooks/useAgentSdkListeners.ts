import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch, MutableRefObject } from "react";
import { useEffect } from "react";
import type {
	ChatSession,
	MessagePart,
	PermissionMode,
	PermissionRequest,
	SessionState,
	TurnPhase,
} from "@/types/session";
import type { AgentChatAction } from "./agentChatReducer";
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

interface SessionStateChanged {
	chat_session_id: string;
	turn_phase: TurnPhase;
	exit_code: number | null;
}

interface StreamingMessageUpdated {
	chat_session_id: string;
	message_id: string;
	parts: MessagePart[];
}

interface PendingMessageConsumed {
	chat_session_id: string;
	agent_message: {
		id: string;
		role: "agent";
		timestamp: number;
	};
}

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	activeSessionRef: MutableRefObject<ChatSession | null>;
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
	const { dispatch, activeSessionRef, refreshSessions } = refs;

	// Listen to SDK messages for meta events (permissions, commands, system messages)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SdkMessage>("agent-sdk-message", (event) => {
			const msg = event.payload;
			const chatSessionId = msg.chat_session_id;

			handleSupportedCommands(msg);
			handlePermissionRequest(msg, chatSessionId, dispatch);
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
	}, [dispatch, activeSessionRef]);

	// Listen to agent-permission-mode-changed from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<{ chat_session_id: string; permission_mode: string }>(
			"agent-permission-mode-changed",
			(event) => {
				const { chat_session_id, permission_mode } = event.payload;
				// Only update if the event is for the active session
				if (activeSessionRef.current?.id === chat_session_id) {
					dispatch({
						type: "SET_PERMISSION_MODE",
						mode: permission_mode as PermissionMode,
					});
				}
			},
		).then((fn) => {
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
	}, [dispatch, activeSessionRef]);

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

	// Listen to agent-session-state-changed (unified state event from Rust)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SessionStateChanged>("agent-session-state-changed", (event) => {
			const { chat_session_id, turn_phase, exit_code } = event.payload;

			dispatch({
				type: "SET_TURN_PHASE",
				sessionId: chat_session_id,
				turnPhase: turn_phase,
			});

			// Turn completed (idle with exit_code): update session state and clear permissions
			if (turn_phase === "idle" && exit_code != null) {
				dispatch({
					type: "SET_PENDING_PERMISSION",
					sessionId: chat_session_id,
					request: null,
				});

				const session = activeSessionRef.current;
				const newState: SessionState = exit_code === 0 ? "idle" : "error";
				if (session && session.id === chat_session_id) {
					dispatch({ type: "UPDATE_SESSION_STATE", state: newState });
				}

				updateSessionState(chat_session_id, newState).catch((e) =>
					console.error("Failed to update session state:", e),
				);

				refreshSessions().catch((e) =>
					console.error("Failed to refresh sessions:", e),
				);
			}
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

	// Listen to agent-pending-message-consumed (Rust auto-consumed pending message after turn_complete)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<PendingMessageConsumed>(
			"agent-pending-message-consumed",
			(event) => {
				const { chat_session_id, agent_message } = event.payload;
				if (activeSessionRef.current?.id !== chat_session_id) return;
				dispatch({
					type: "ADD_MESSAGE",
					message: {
						id: agent_message.id,
						role: agent_message.role,
						parts: [],
						timestamp: agent_message.timestamp,
					},
				});
			},
		).then((fn) => {
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
	}, [dispatch, activeSessionRef]);
}
