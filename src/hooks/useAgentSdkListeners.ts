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
	parent_tool_use_id?: string | null;
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
	parent_tool_use_id?: string | null;
	message: {
		content: ContentBlock[];
	};
}

interface UserMessage {
	type: "user";
	session_id?: string;
	chat_session_id?: string;
	parent_tool_use_id?: string | null;
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

interface TaskSystemMessage {
	type: "system";
	session_id?: string;
	chat_session_id?: string;
	parent_tool_use_id?: string | null;
	subtype: "task_started" | "task_notification" | "task_progress";
	tool_use_id: string;
	description?: string;
	summary?: string;
	status?: string;
}

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
	| StreamEvent
	| AssistantMessage
	| UserMessage
	| ResultMessage
	| TaskSystemMessage
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

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	streamingMessageIdsRef: MutableRefObject<Map<string, string>>;
	activeSessionRef: MutableRefObject<ChatSession | null>;
	streamingBuffersRef: MutableRefObject<Map<string, StreamingBuffer>>;
	lastPromptsRef: MutableRefObject<Map<string, string>>;
	refreshSessions: () => Promise<unknown>;
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
	parentToolUseId?: string,
): void {
	buf.parts = appendToParts(buf.parts, partType, chunk, parentToolUseId);
}

function getBufferTextContent(buf: StreamingBuffer): string {
	return getTextContent(buf.parts);
}

function isTaskSystemMessage(msg: SdkMessage): msg is TaskSystemMessage {
	return (
		msg.type === "system" &&
		"subtype" in msg &&
		typeof msg.subtype === "string" &&
		["task_started", "task_notification", "task_progress"].includes(
			msg.subtype,
		) &&
		"tool_use_id" in msg &&
		typeof msg.tool_use_id === "string"
	);
}

type TaskStatusValue =
	| "started"
	| "completed"
	| "failed"
	| "stopped"
	| "progress";

function resolveTaskStatus(msg: TaskSystemMessage): TaskStatusValue | null {
	switch (msg.subtype) {
		case "task_started":
			return "started";
		case "task_progress":
			return "progress";
		case "task_notification":
			if (
				msg.status === "completed" ||
				msg.status === "failed" ||
				msg.status === "stopped"
			)
				return msg.status;
			return null;
	}
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
	if (messageId) {
		const part: MessagePart = {
			type: "permission",
			request: req,
			status: "pending",
		};
		dispatch({
			type: "ADD_PERMISSION_PART",
			messageId,
			request: req,
		});
		const buf = streamingBuffersRef.current.get(chatSessionId);
		if (buf) buf.parts.push(part);
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
	activeSessionRef: MutableRefObject<ChatSession | null>,
): void {
	if (msg.type !== "system" || !chatSessionId) return;
	// task subtypes are handled by handleTaskMessage
	if (isTaskSystemMessage(msg)) return;
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
	const parentToolUseId =
		("parent_tool_use_id" in msg &&
			typeof msg.parent_tool_use_id === "string" &&
			msg.parent_tool_use_id) ||
		undefined;
	const delta = extractStreamingDelta(msg);
	// dispatch updates the active session's UI via reducer;
	// buf accumulates for all sessions (used for persistence).
	if (delta) {
		if (delta.type === "text") {
			dispatch({
				type: "APPEND_STREAMING",
				messageId,
				chunk: delta.text,
				parentToolUseId,
			});
			if (buf) bufferAppend(buf, "text", delta.text, parentToolUseId);
		} else {
			dispatch({
				type: "APPEND_THINKING",
				messageId,
				chunk: delta.thinking,
				parentToolUseId,
			});
			if (buf) bufferAppend(buf, "thinking", delta.thinking, parentToolUseId);
		}
	}

	// Extract tool_use from assistant messages
	if (msg.type === "assistant") {
		const assistantMsg = msg as AssistantMessage;
		const pId = assistantMsg.parent_tool_use_id || undefined;
		for (const block of assistantMsg.message.content) {
			if (block.type === "tool_use") {
				const toolBlock = block as ToolUseBlock;
				const part: MessagePart = {
					type: "tool_use",
					tool: toolBlock.name,
					input: toolBlock.input,
					id: toolBlock.id,
					...(pId && { parentToolUseId: pId }),
				};
				dispatch({
					type: "APPEND_TOOL_USE",
					messageId,
					tool: toolBlock.name,
					input: toolBlock.input,
					id: toolBlock.id,
					parentToolUseId: pId,
				});
				if (buf) buf.parts.push(part);
			}
		}
	}

	// Extract tool_result from user messages
	if (msg.type === "user") {
		const userMsg = msg as UserMessage;
		const pId = userMsg.parent_tool_use_id || undefined;
		for (const block of userMsg.message.content) {
			if (block.type === "tool_result") {
				const resultBlock = block as ToolResultBlock;
				const content = extractToolResultContent(resultBlock.content);
				const isError = !!resultBlock.is_error;
				const part: MessagePart = {
					type: "tool_result",
					content,
					isError,
					toolUseId: resultBlock.tool_use_id,
					...(pId && { parentToolUseId: pId }),
				};
				dispatch({
					type: "APPEND_TOOL_RESULT",
					messageId,
					content,
					isError,
					toolUseId: resultBlock.tool_use_id,
					parentToolUseId: pId,
				});
				if (buf) buf.parts.push(part);
			}
		}
	}
}

function handleTaskMessage(
	msg: SdkMessage,
	messageId: string | null | undefined,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	streamingBuffersRef: MutableRefObject<Map<string, StreamingBuffer>>,
): void {
	if (!chatSessionId || !messageId || !isTaskSystemMessage(msg)) return;

	const status = resolveTaskStatus(msg);
	if (!status) return;

	const part: MessagePart = {
		type: "task_status",
		taskToolUseId: msg.tool_use_id,
		status,
		...(msg.description && { description: msg.description }),
		...(msg.summary && { summary: msg.summary }),
	};

	dispatch({
		type: "APPEND_TASK_STATUS",
		messageId,
		taskToolUseId: msg.tool_use_id,
		status,
		description: msg.description,
		summary: msg.summary,
	});

	const buf = streamingBuffersRef.current.get(chatSessionId);
	if (buf) buf.parts.push(part);
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
				? (streamingMessageIdsRef.current.get(chatSessionId) ?? null)
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
			handleTaskMessage(
				msg,
				messageId,
				chatSessionId,
				dispatch,
				streamingBuffersRef,
			);
			handleSystemMessage(msg, chatSessionId, dispatch, activeSessionRef);
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
