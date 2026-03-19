import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch, MutableRefObject } from "react";
import { useEffect } from "react";
import type {
	ChatSession,
	PermissionMode,
	PermissionRequest,
	SessionState,
} from "@/types/session";
import type { AgentChatAction } from "./agentChatReducer";
import {
	updateMessageContent,
	updateSessionAgentInfo,
	updateSessionState,
} from "./useSessionStore";

interface ContentBlockDelta {
	type: "content_block_delta";
	delta: { type: string; text?: string; thinking?: string };
}

interface StreamEvent {
	type: "stream_event";
	session_id?: string;
	event: ContentBlockDelta | { type: string };
}

interface ToolUseBlock {
	type: "tool_use";
	id: string;
	name: string;
	input: Record<string, unknown>;
}

interface ToolResultBlock {
	type: "tool_result";
	tool_use_id: string;
	content?: string | Array<{ type: string; text?: string }>;
	is_error?: boolean;
}

type ContentBlock =
	| { type: string; text?: string }
	| ToolUseBlock
	| ToolResultBlock;

interface AssistantMessage {
	type: "assistant";
	session_id?: string;
	message: {
		content: ContentBlock[];
	};
}

interface UserMessage {
	type: "user";
	session_id?: string;
	message: {
		content: ContentBlock[];
	};
}

interface ResultMessage {
	type: "result";
	session_id?: string;
	subtype?: string;
}

type SdkMessage =
	| StreamEvent
	| AssistantMessage
	| UserMessage
	| ResultMessage
	| {
			type: string;
			session_id?: string;
			[key: string]: unknown;
	  };

interface QueryCompleted {
	exit_code: number;
	stderr: string;
}

interface AgentPtyListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	streamingMessageIdRef: MutableRefObject<string | null>;
	activeSessionRef: MutableRefObject<ChatSession | null>;
	refreshSessions: () => Promise<void>;
	handleRetry: (content: string) => Promise<void>;
	isRetryingRef: MutableRefObject<boolean>;
}

type StreamDelta =
	| { type: "text"; text: string }
	| { type: "thinking"; thinking: string };

export function extractToolResultContent(
	content: ToolResultBlock["content"],
): string {
	if (typeof content === "string") return content;
	if (Array.isArray(content)) {
		return content
			.filter((b) => b.type === "text" && b.text)
			.map((b) => b.text)
			.join("\n");
	}
	return "";
}

export function extractStreamingDelta(msg: SdkMessage): StreamDelta | null {
	if (msg.type !== "stream_event") return null;
	const streamMsg = msg as StreamEvent;
	if (streamMsg.event?.type !== "content_block_delta") return null;
	const delta = (streamMsg.event as ContentBlockDelta).delta;
	if (delta.type === "text_delta" && delta.text) {
		return { type: "text", text: delta.text };
	}
	if (delta.type === "thinking_delta" && delta.thinking) {
		return { type: "thinking", thinking: delta.thinking };
	}
	return null;
}

export function shouldRetry(
	exitCode: number,
	isRetrying: boolean,
	session: ChatSession | null,
): string | null {
	if (exitCode === 0 || isRetrying || !session?.agentSessionId) return null;
	const agentMsgs = session.messages.filter((m) => m.role === "agent");
	const lastAgentMsg = agentMsgs[agentMsgs.length - 1];
	if (!lastAgentMsg || lastAgentMsg.content.trim()) return null;
	const humanMsgs = session.messages.filter((m) => m.role === "human");
	const lastHumanMsg = humanMsgs[humanMsgs.length - 1];
	return lastHumanMsg?.content ?? null;
}

export function useAgentPtyListeners(refs: AgentPtyListenerRefs): void {
	const {
		dispatch,
		streamingMessageIdRef,
		activeSessionRef,
		refreshSessions,
		handleRetry,
		isRetryingRef,
	} = refs;

	// Listen to SDK messages for streaming agent responses and session_id capture
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SdkMessage>("agent-sdk-message", (event) => {
			const msg = event.payload;
			const messageId = streamingMessageIdRef.current;

			// Detect permission_request and dispatch
			if (msg.type === "permission_request") {
				const req = msg as unknown as PermissionRequest & {
					tool_name?: string;
				};
				dispatch({
					type: "SET_PENDING_PERMISSION",
					request: req as PermissionRequest,
				});
			}

			// Sync permissionMode from SDK system messages (init/status)
			if (
				msg.type === "system" &&
				"permissionMode" in msg &&
				typeof msg.permissionMode === "string"
			) {
				const sdkMode = msg.permissionMode as PermissionMode;
				if (sdkMode === "default") {
					dispatch({ type: "RESTORE_USER_PERMISSION_MODE" });
				} else {
					dispatch({ type: "SET_PERMISSION_MODE", mode: sdkMode });
				}
			}

			// Capture session_id from SDK messages
			if ("session_id" in msg && msg.session_id) {
				const session = activeSessionRef.current;
				if (session && !session.agentSessionId) {
					dispatch({
						type: "SET_AGENT_SESSION_ID",
						agentSessionId: msg.session_id,
					});
					updateSessionAgentInfo(session.id, msg.session_id).catch((e) =>
						console.error("Failed to persist agent session id:", e),
					);
				}
			}

			// Extract streaming delta from stream_event messages
			if (messageId) {
				const delta = extractStreamingDelta(msg);
				if (delta) {
					if (delta.type === "text") {
						dispatch({
							type: "APPEND_STREAMING",
							messageId,
							chunk: delta.text,
						});
					} else {
						dispatch({
							type: "APPEND_THINKING",
							messageId,
							chunk: delta.thinking,
						});
					}
				}

				// Extract tool_use from assistant messages
				if (msg.type === "assistant") {
					const assistantMsg = msg as AssistantMessage;
					for (const block of assistantMsg.message.content) {
						if (block.type === "tool_use") {
							const toolBlock = block as ToolUseBlock;
							dispatch({
								type: "APPEND_TOOL_USE",
								messageId,
								tool: toolBlock.name,
								input: toolBlock.input,
								id: toolBlock.id,
							});
						}
					}
				}

				// Extract tool_result from user messages
				if (msg.type === "user") {
					const userMsg = msg as UserMessage;
					for (const block of userMsg.message.content) {
						if (block.type === "tool_result") {
							const resultBlock = block as ToolResultBlock;
							dispatch({
								type: "APPEND_TOOL_RESULT",
								messageId,
								content: extractToolResultContent(resultBlock.content),
								isError: !!resultBlock.is_error,
							});
						}
					}
				}
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
	}, [dispatch, streamingMessageIdRef, activeSessionRef]);

	// Listen to agent-query-completed for completion/error handling
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<QueryCompleted>("agent-query-completed", (event) => {
			const info = event.payload;

			dispatch({ type: "SET_STREAMING", streaming: false });
			dispatch({ type: "SET_PENDING_PERMISSION", request: null });

			const msgId = streamingMessageIdRef.current;
			streamingMessageIdRef.current = null;

			const session = activeSessionRef.current;

			// Persist final message content
			if (msgId && session) {
				const finalMsg = session.messages.find((m) => m.id === msgId);
				if (finalMsg) {
					updateMessageContent(
						session.id,
						msgId,
						finalMsg.content,
						finalMsg.thinking,
						finalMsg.activities,
					).catch((e) => console.error("Failed to persist agent message:", e));
				}
			}

			// Retry logic: if --resume failed (error + empty content), retry without session ID
			const retryContent = shouldRetry(
				info.exit_code,
				isRetryingRef.current,
				session,
			);
			if (retryContent !== null) {
				handleRetry(retryContent);
				return;
			}

			// Update session state
			const newState: SessionState = info.exit_code === 0 ? "idle" : "error";
			dispatch({ type: "UPDATE_SESSION_STATE", state: newState });

			if (session) {
				updateSessionState(session.id, newState).catch((e) =>
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
	}, [
		dispatch,
		streamingMessageIdRef,
		activeSessionRef,
		refreshSessions,
		handleRetry,
		isRetryingRef,
	]);
}
