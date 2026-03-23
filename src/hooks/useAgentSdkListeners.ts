import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch, MutableRefObject } from "react";
import { useEffect } from "react";
import {
	type ChatSession,
	getTextContent,
	type MessagePart,
	type PermissionMode,
	type PermissionRequest,
	type SessionState,
} from "@/types/session";
import { type AgentChatAction, appendToParts } from "./agentChatReducer";
import {
	updateMessageParts,
	updateSessionAgentInfo,
	updateSessionState,
} from "./useSessionStore";
import { setSlashCommands } from "./useSlashCommands";

export interface StreamingBuffer {
	parts: MessagePart[];
}

interface ContentBlockDelta {
	type: "content_block_delta";
	delta: { type: string; text?: string; thinking?: string };
}

interface StreamEvent {
	type: "stream_event";
	session_id?: string;
	chat_session_id?: string;
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
	chat_session_id?: string;
	message: {
		content: ContentBlock[];
	};
}

interface UserMessage {
	type: "user";
	session_id?: string;
	chat_session_id?: string;
	message: {
		content: ContentBlock[];
	};
}

interface ResultMessage {
	type: "result";
	session_id?: string;
	chat_session_id?: string;
	subtype?: string;
	errors?: string[];
}

type SdkMessage =
	| StreamEvent
	| AssistantMessage
	| UserMessage
	| ResultMessage
	| {
			type: string;
			session_id?: string;
			chat_session_id?: string;
			[key: string]: unknown;
	  };

interface QueryCompleted {
	exit_code: number;
	stderr: string;
	chat_session_id?: string;
}

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	streamingMessageIdsRef: MutableRefObject<Map<string, string>>;
	activeSessionRef: MutableRefObject<ChatSession | null>;
	streamingBuffersRef: MutableRefObject<Map<string, StreamingBuffer>>;
	lastPromptsRef: MutableRefObject<Map<string, string>>;
	refreshSessions: () => Promise<void>;
	handleRetry: (content: string, chatSessionId: string) => Promise<void>;
	isRetryingRef: MutableRefObject<Set<string>>;
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

function bufferAppend(
	buf: StreamingBuffer,
	partType: "text" | "thinking",
	chunk: string,
): void {
	buf.parts = appendToParts(buf.parts, partType, chunk);
}

function getBufferTextContent(buf: StreamingBuffer): string {
	return getTextContent(buf.parts);
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
	messageId: string | null,
	dispatch: Dispatch<AgentChatAction>,
	streamingBuffersRef: MutableRefObject<Map<string, StreamingBuffer>>,
): void {
	if (msg.type !== "permission_request" || !chatSessionId) return;
	const req = msg as unknown as PermissionRequest & {
		tool_name?: string;
	};
	dispatch({
		type: "SET_PENDING_PERMISSION",
		sessionId: chatSessionId,
		request: req as PermissionRequest,
	});
	if (messageId) {
		dispatch({
			type: "ADD_PERMISSION_PART",
			messageId,
			request: req as PermissionRequest,
		});
		const buf = streamingBuffersRef.current.get(chatSessionId);
		if (buf) {
			buf.parts.push({
				type: "permission",
				request: req as PermissionRequest,
				status: "pending",
			});
		}
	}
}

function handlePermissionModeSync(
	msg: SdkMessage,
	dispatch: Dispatch<AgentChatAction>,
): void {
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
}

function handleSystemMessage(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (msg.type !== "system" || !chatSessionId) return;
	const text =
		typeof (msg as Record<string, unknown>).message === "string"
			? ((msg as Record<string, unknown>).message as string)
			: typeof (msg as Record<string, unknown>).content === "string"
				? ((msg as Record<string, unknown>).content as string)
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

function handleSessionIdCapture(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	activeSessionRef: MutableRefObject<ChatSession | null>,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (!("session_id" in msg) || !msg.session_id || !chatSessionId) return;
	updateSessionAgentInfo(chatSessionId, msg.session_id).catch((e) =>
		console.error("Failed to persist agent session id:", e),
	);
	const session = activeSessionRef.current;
	if (session && session.id === chatSessionId && !session.agentSessionId) {
		dispatch({
			type: "SET_AGENT_SESSION_ID",
			agentSessionId: msg.session_id,
		});
	}
}

function handleStreamingContent(
	msg: SdkMessage,
	messageId: string,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	streamingBuffersRef: MutableRefObject<Map<string, StreamingBuffer>>,
): void {
	const buf = chatSessionId
		? streamingBuffersRef.current.get(chatSessionId)
		: undefined;
	const delta = extractStreamingDelta(msg);
	// dispatch updates the active session's UI via reducer;
	// buf accumulates for all sessions (used for persistence).
	if (delta) {
		if (delta.type === "text") {
			dispatch({
				type: "APPEND_STREAMING",
				messageId,
				chunk: delta.text,
			});
			if (buf) bufferAppend(buf, "text", delta.text);
		} else {
			dispatch({
				type: "APPEND_THINKING",
				messageId,
				chunk: delta.thinking,
			});
			if (buf) bufferAppend(buf, "thinking", delta.thinking);
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
				if (buf) {
					buf.parts.push({
						type: "tool_use",
						tool: toolBlock.name,
						input: toolBlock.input,
						id: toolBlock.id,
					});
				}
			}
		}
	}

	// Extract tool_result from user messages
	if (msg.type === "user") {
		const userMsg = msg as UserMessage;
		for (const block of userMsg.message.content) {
			if (block.type === "tool_result") {
				const resultBlock = block as ToolResultBlock;
				const content = extractToolResultContent(resultBlock.content);
				const isError = !!resultBlock.is_error;
				dispatch({
					type: "APPEND_TOOL_RESULT",
					messageId,
					content,
					isError,
					toolUseId: resultBlock.tool_use_id,
				});
				if (buf) {
					buf.parts.push({
						type: "tool_result",
						content,
						isError,
						toolUseId: resultBlock.tool_use_id,
					});
				}
			}
		}
	}
}

function handleResultErrors(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (msg.type !== "result" || !chatSessionId) return;
	const resultMsg = msg as ResultMessage;
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
	const {
		dispatch,
		streamingMessageIdsRef,
		activeSessionRef,
		streamingBuffersRef,
		lastPromptsRef,
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
			const chatSessionId = msg.chat_session_id;
			const messageId = chatSessionId
				? streamingMessageIdsRef.current.get(chatSessionId)
				: null;

			handleSupportedCommands(msg);
			handlePermissionRequest(
				msg,
				chatSessionId,
				messageId,
				dispatch,
				streamingBuffersRef,
			);
			handlePermissionModeSync(msg, dispatch);
			handleSystemMessage(msg, chatSessionId, dispatch);
			handleSessionIdCapture(msg, chatSessionId, activeSessionRef, dispatch);
			if (messageId) {
				handleStreamingContent(
					msg,
					messageId,
					chatSessionId,
					dispatch,
					streamingBuffersRef,
				);
			}
			handleResultErrors(msg, chatSessionId, dispatch);
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
	}, [dispatch, streamingMessageIdsRef, activeSessionRef, streamingBuffersRef]);

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

			const msgId = chatSessionId
				? streamingMessageIdsRef.current.get(chatSessionId)
				: null;
			if (chatSessionId) {
				streamingMessageIdsRef.current.delete(chatSessionId);
			}

			const buffer = chatSessionId
				? streamingBuffersRef.current.get(chatSessionId)
				: undefined;

			// Persist final message content from buffer (works for all sessions)
			if (msgId && chatSessionId && buffer) {
				updateMessageParts(chatSessionId, msgId, buffer.parts).catch((e) =>
					console.error("Failed to persist agent message:", e),
				);
			}

			// Retry logic: if --resume failed (error + empty content), retry without session ID
			if (
				chatSessionId &&
				info.exit_code !== 0 &&
				!isRetryingRef.current.has(chatSessionId)
			) {
				const lastPrompt = lastPromptsRef.current.get(chatSessionId);
				if (buffer && !getBufferTextContent(buffer).trim() && lastPrompt) {
					streamingBuffersRef.current.delete(chatSessionId);
					handleRetry(lastPrompt, chatSessionId);
					return;
				}
			}

			if (chatSessionId) {
				streamingBuffersRef.current.delete(chatSessionId);
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
	}, [
		dispatch,
		streamingMessageIdsRef,
		activeSessionRef,
		streamingBuffersRef,
		lastPromptsRef,
		refreshSessions,
		handleRetry,
		isRetryingRef,
	]);
}
