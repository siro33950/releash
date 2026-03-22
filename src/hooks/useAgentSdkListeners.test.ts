import { renderHook } from "@testing-library/react";
import { describe, expect, it, type Mock, vi } from "vitest";

type ListenCallback = (event: { payload: unknown }) => void;
type UnlistenFn = Mock;

let listenResolvers: Array<{
	resolve: (fn: UnlistenFn) => void;
	eventName: string;
}> = [];

const listenCallbacks: Map<string, ListenCallback> = new Map();

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn((eventName: string, cb: ListenCallback) => {
		listenCallbacks.set(eventName, cb);
		return new Promise<UnlistenFn>((resolve) => {
			listenResolvers.push({ resolve, eventName });
		});
	}),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("./useSessionStore", async (importOriginal) => {
	const actual = await importOriginal<typeof import("./useSessionStore")>();
	return {
		...actual,
		updateMessageParts: vi.fn().mockResolvedValue(undefined),
		updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
		updateSessionState: vi.fn().mockResolvedValue(undefined),
	};
});

const {
	useAgentSdkListeners,
	extractStreamingDelta,
	extractToolResultContent,
	shouldRetry,
} = await import("./useAgentSdkListeners");

import type { StreamingBuffer } from "./useAgentSdkListeners";

function makeRefs() {
	return {
		dispatch: vi.fn(),
		streamingMessageIdsRef: { current: new Map<string, string>() },
		activeSessionRef: { current: null },
		streamingBuffersRef: {
			current: new Map<string, StreamingBuffer>(),
		},
		lastPromptsRef: { current: new Map<string, string>() },
		refreshSessions: vi.fn().mockResolvedValue(undefined),
		handleRetry: vi.fn().mockResolvedValue(undefined),
		isRetryingRef: { current: new Set<string>() },
	};
}

describe("extractStreamingDelta", () => {
	it("extracts text from stream_event with text_delta", () => {
		const msg = {
			type: "stream_event" as const,
			event: {
				type: "content_block_delta" as const,
				delta: { type: "text_delta", text: "Hello" },
			},
		};
		expect(extractStreamingDelta(msg)).toEqual({ type: "text", text: "Hello" });
	});

	it("extracts thinking from stream_event with thinking_delta", () => {
		const msg = {
			type: "stream_event" as const,
			event: {
				type: "content_block_delta" as const,
				delta: { type: "thinking_delta", thinking: "thinking..." },
			},
		};
		expect(extractStreamingDelta(msg)).toEqual({
			type: "thinking",
			thinking: "thinking...",
		});
	});

	it("returns null for non-stream_event messages", () => {
		expect(extractStreamingDelta({ type: "assistant" } as never)).toBeNull();
		expect(extractStreamingDelta({ type: "result" } as never)).toBeNull();
		expect(extractStreamingDelta({ type: "user" } as never)).toBeNull();
	});

	it("returns null for stream_event without content_block_delta", () => {
		const msg = {
			type: "stream_event" as const,
			event: { type: "message_start" },
		};
		expect(extractStreamingDelta(msg)).toBeNull();
	});
});

describe("extractToolResultContent", () => {
	it("returns string content directly", () => {
		expect(extractToolResultContent("hello")).toBe("hello");
	});

	it("extracts text from array of content blocks", () => {
		const content = [
			{ type: "text", text: "line1" },
			{ type: "text", text: "line2" },
		];
		expect(extractToolResultContent(content)).toBe("line1\nline2");
	});

	it("returns empty string for undefined", () => {
		expect(extractToolResultContent(undefined)).toBe("");
	});

	it("returns empty string for empty array", () => {
		expect(extractToolResultContent([])).toBe("");
	});

	it("filters non-text blocks", () => {
		const content = [
			{ type: "image", text: undefined },
			{ type: "text", text: "only this" },
		];
		expect(extractToolResultContent(content)).toBe("only this");
	});
});

describe("useAgentSdkListeners cancelled flag", () => {
	it("calls unlisten immediately if cleanup happens before listen resolves", async () => {
		listenResolvers = [];
		const refs = makeRefs();

		const { unmount } = renderHook(() => useAgentSdkListeners(refs));

		const pendingResolvers = [...listenResolvers];
		expect(pendingResolvers.length).toBeGreaterThanOrEqual(2);

		unmount();

		const unlistenFns: UnlistenFn[] = [];
		for (const { resolve } of pendingResolvers) {
			const unlisten = vi.fn();
			unlistenFns.push(unlisten);
			resolve(unlisten);
		}

		await vi.waitFor(() => {
			for (const unlisten of unlistenFns) {
				expect(unlisten).toHaveBeenCalledTimes(1);
			}
		});
	});

	it("stores unlisten and calls it on cleanup when listen resolves before unmount", async () => {
		listenResolvers = [];
		const refs = makeRefs();

		const { unmount } = renderHook(() => useAgentSdkListeners(refs));

		const unlistenFns: UnlistenFn[] = [];
		for (const { resolve } of listenResolvers) {
			const unlisten = vi.fn();
			unlistenFns.push(unlisten);
			resolve(unlisten);
		}

		await vi.waitFor(() => {
			for (const unlisten of unlistenFns) {
				expect(unlisten).not.toHaveBeenCalled();
			}
		});

		unmount();

		for (const unlisten of unlistenFns) {
			expect(unlisten).toHaveBeenCalledTimes(1);
		}
	});

	it("registers listeners for agent-sdk-message and agent-query-completed", () => {
		listenResolvers = [];
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		const eventNames = listenResolvers.map((r) => r.eventName);
		expect(eventNames).toContain("agent-sdk-message");
		expect(eventNames).toContain("agent-query-completed");
		expect(eventNames).not.toContain("agent-state-changed");
	});
});

describe("useAgentSdkListeners callback behavior", () => {
	it("dispatches APPEND_STREAMING when stream_event with text_delta is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));

		// Resolve all listeners so they're active
		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		// Simulate receiving a stream_event with text_delta
		cb?.({
			payload: {
				type: "stream_event",
				session_id: "sid-123",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "text_delta", text: "Hello" },
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPEND_STREAMING",
			messageId: "msg-001",
			chunk: "Hello",
		});
	});

	it("dispatches SET_AGENT_SESSION_ID when session_id is in message", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "system",
				subtype: "init",
				session_id: "sdk-session-abc",
				chat_session_id: "session-1",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_AGENT_SESSION_ID",
			agentSessionId: "sdk-session-abc",
		});
	});

	it("dispatches STOP_STREAMING when agent-query-completed is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-002");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [{ type: "text", content: "response text" }],
		});
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [{ id: "msg-002", role: "agent", parts: [] }],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-query-completed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				exit_code: 0,
				stderr: "",
				chat_session_id: "session-1",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "STOP_STREAMING",
			sessionId: "session-1",
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "UPDATE_SESSION_STATE",
			state: "idle",
		});
	});

	it("dispatches APPEND_THINKING when stream_event with thinking_delta is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "stream_event",
				session_id: "sid-123",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "thinking_delta", thinking: "Let me think..." },
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPEND_THINKING",
			messageId: "msg-001",
			chunk: "Let me think...",
		});
	});

	it("dispatches APPEND_TOOL_USE when assistant message with tool_use is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "assistant",
				session_id: "sid-123",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_use",
							id: "toolu_abc",
							name: "Read",
							input: { file_path: "/src/main.ts" },
						},
					],
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPEND_TOOL_USE",
			messageId: "msg-001",
			tool: "Read",
			input: { file_path: "/src/main.ts" },
			id: "toolu_abc",
		});
	});

	it("dispatches APPEND_TOOL_RESULT when user message with tool_result is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "user",
				session_id: "sid-123",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_result",
							tool_use_id: "toolu_abc",
							content: "file content here",
							is_error: false,
						},
					],
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPEND_TOOL_RESULT",
			messageId: "msg-001",
			content: "file content here",
			isError: false,
		});
	});

	it("dispatches APPEND_TOOL_RESULT with isError true for error results", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "user",
				session_id: "sid-123",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_result",
							tool_use_id: "toolu_abc",
							content: "Error: file not found",
							is_error: true,
						},
					],
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPEND_TOOL_RESULT",
			messageId: "msg-001",
			content: "Error: file not found",
			isError: true,
		});
	});

	it("dispatches SET_PENDING_PERMISSION when permission_request is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		const request = {
			type: "permission_request",
			chat_session_id: "session-1",
			request_id: "req-001",
			tool_name: "Edit",
			input: { file_path: "/src/index.ts" },
			tool_use_id: "toolu_001",
			title: "Edit file",
		};

		cb?.({ payload: request });

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request,
		});
	});

	it("dispatches SET_PENDING_PERMISSION null on agent-query-completed", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-002");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [{ type: "text", content: "response text" }],
		});
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [{ id: "msg-002", role: "agent", parts: [] }],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-query-completed");
		expect(cb).toBeDefined();

		cb?.({
			payload: { exit_code: 0, stderr: "", chat_session_id: "session-1" },
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request: null,
		});
	});

	it("does NOT dispatch APPEND_STREAMING when no streaming entry for session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		// No entry in streamingMessageIdsRef for "session-1"

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "stream_event",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "text_delta", text: "Hello" },
				},
			},
		});

		const appendCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "APPEND_STREAMING",
		);
		expect(appendCalls).toHaveLength(0);
	});
});

describe("SET_PERMISSION_MODE from SDK system messages", () => {
	it("dispatches SET_PERMISSION_MODE when system init message has permissionMode", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "init",
				session_id: "sdk-session-abc",
				permissionMode: "plan",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PERMISSION_MODE",
			mode: "plan",
		});
	});

	it("dispatches RESTORE_USER_PERMISSION_MODE when system message has permissionMode: default", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "status",
				permissionMode: "default",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "RESTORE_USER_PERMISSION_MODE",
		});
	});

	it("does not dispatch SET_PERMISSION_MODE when system message has no permissionMode", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "init",
				session_id: "sdk-session-abc",
			},
		});

		const syncCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_PERMISSION_MODE",
		);
		expect(syncCalls).toHaveLength(0);
	});

	it("dispatches ADD_MESSAGE for system message with message field", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				chat_session_id: "session-1",
				message: "Not logged in. Please run 'claude login'.",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(1);
		const msg = (
			addMsgCalls[0][0] as {
				message: {
					role: string;
					parts: Array<{ type: string; content: string }>;
				};
			}
		).message;
		expect(msg.role).toBe("system");
		expect(msg.parts[0].content).toBe(
			"Not logged in. Please run 'claude login'.",
		);
	});

	it("dispatches ADD_MESSAGE for system message with content field", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				chat_session_id: "session-1",
				content: "API key expired",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(1);
		const msg = (
			addMsgCalls[0][0] as {
				message: {
					role: string;
					parts: Array<{ type: string; content: string }>;
				};
			}
		).message;
		expect(msg.role).toBe("system");
		expect(msg.parts[0].content).toBe("API key expired");
	});

	it("does not dispatch ADD_MESSAGE for system message without text content", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "init",
				chat_session_id: "session-1",
				session_id: "sdk-session-abc",
				permissionMode: "plan",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch ADD_MESSAGE for system message without chat_session_id", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				message: "Some system message",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch SET_PLAN_MODE_ACTIVE for EnterPlanMode tool_use (removed)", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "assistant",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_use",
							id: "toolu_plan_001",
							name: "EnterPlanMode",
							input: {},
						},
					],
				},
			},
		});

		const planModeCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_PLAN_MODE_ACTIVE",
		);
		expect(planModeCalls).toHaveLength(0);
	});
});

describe("shouldRetry", () => {
	it("returns null when exit code is 0", () => {
		const session = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1000,
				},
				{ id: "m2", role: "agent", parts: [], timestamp: 1001 },
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			agentSessionId: "sdk-sess-1",
		} as never;
		expect(shouldRetry(0, false, session)).toBeNull();
	});

	it("returns null when already retrying", () => {
		const session = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1000,
				},
				{ id: "m2", role: "agent", parts: [], timestamp: 1001 },
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			agentSessionId: "sdk-sess-1",
		} as never;
		expect(shouldRetry(1, true, session)).toBeNull();
	});

	it("returns null when no agentSessionId", () => {
		const session = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1000,
				},
				{ id: "m2", role: "agent", parts: [], timestamp: 1001 },
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			agentSessionId: null,
		} as never;
		expect(shouldRetry(1, false, session)).toBeNull();
	});

	it("returns null when last agent message has content", () => {
		const session = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1000,
				},
				{
					id: "m2",
					role: "agent",
					parts: [{ type: "text", content: "response" }],
					timestamp: 1001,
				},
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			agentSessionId: "sdk-sess-1",
		} as never;
		expect(shouldRetry(1, false, session)).toBeNull();
	});

	it("returns last human message content when retry conditions met", () => {
		const session = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1000,
				},
				{ id: "m2", role: "agent", parts: [], timestamp: 1001 },
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			agentSessionId: "sdk-sess-1",
		} as never;
		expect(shouldRetry(1, false, session)).toBe("hello");
	});

	it("returns null when session is null", () => {
		expect(shouldRetry(1, false, null)).toBeNull();
	});
});

describe("streaming buffer accumulation", () => {
	it("accumulates text delta into streamingBuffersRef", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [],
		});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "stream_event",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "text_delta", text: "Hello" },
				},
			},
		});
		cb?.({
			payload: {
				type: "stream_event",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "text_delta", text: " World" },
				},
			},
		});

		const buf = refs.streamingBuffersRef.current.get("session-1");
		const textParts = buf?.parts.filter((p) => p.type === "text");
		expect(textParts).toHaveLength(1);
		expect((textParts?.[0] as { content: string })?.content).toBe(
			"Hello World",
		);
	});

	it("accumulates thinking delta into streamingBuffersRef", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [],
		});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "stream_event",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "thinking_delta", thinking: "Let me " },
				},
			},
		});
		cb?.({
			payload: {
				type: "stream_event",
				chat_session_id: "session-1",
				event: {
					type: "content_block_delta",
					delta: { type: "thinking_delta", thinking: "think..." },
				},
			},
		});

		const buf = refs.streamingBuffersRef.current.get("session-1");
		const thinkingParts = buf?.parts.filter((p) => p.type === "thinking");
		expect(thinkingParts).toHaveLength(1);
		expect((thinkingParts?.[0] as { content: string })?.content).toBe(
			"Let me think...",
		);
	});

	it("accumulates tool_use and tool_result activities into buffer", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [],
		});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "assistant",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_use",
							id: "toolu_abc",
							name: "Read",
							input: { file_path: "/src/main.ts" },
						},
					],
				},
			},
		});

		cb?.({
			payload: {
				type: "user",
				chat_session_id: "session-1",
				message: {
					content: [
						{
							type: "tool_result",
							tool_use_id: "toolu_abc",
							content: "file content",
							is_error: false,
						},
					],
				},
			},
		});

		const buf = refs.streamingBuffersRef.current.get("session-1");
		const toolParts = buf?.parts.filter(
			(p) => p.type === "tool_use" || p.type === "tool_result",
		);
		expect(toolParts).toHaveLength(2);
		expect(toolParts?.[0]).toEqual({
			type: "tool_use",
			tool: "Read",
			input: { file_path: "/src/main.ts" },
			id: "toolu_abc",
		});
		expect(toolParts?.[1]).toEqual({
			type: "tool_result",
			content: "file content",
			isError: false,
		});
	});
});

describe("non-active session persistence", () => {
	it("persists message content from buffer for non-active session on completion", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-2", "msg-002");
		refs.streamingBuffersRef.current.set("session-2", {
			parts: [
				{ type: "thinking", content: "some thinking" },
				{ type: "text", content: "background response" },
			],
		});
		// Active session is session-1, not session-2
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-query-completed");

		cb?.({
			payload: {
				exit_code: 0,
				stderr: "",
				chat_session_id: "session-2",
			},
		});

		const { updateMessageParts } = await import("./useSessionStore");
		const calls = vi.mocked(updateMessageParts).mock.calls;
		const call = calls.find((c) => c[0] === "session-2");
		expect(call).toBeDefined();
		expect(call).toEqual([
			"session-2",
			"msg-002",
			[
				{ type: "thinking", content: "some thinking" },
				{ type: "text", content: "background response" },
			],
		]);
	});

	it("persists agentSessionId for non-active session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		// Active session is session-1
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [],
			createdAt: Date.now(),
			agentSessionId: "sdk-1",
		} as never;

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		// session-2 (non-active) receives session_id
		cb?.({
			payload: {
				type: "system",
				subtype: "init",
				session_id: "sdk-session-2",
				chat_session_id: "session-2",
			},
		});

		const { updateSessionAgentInfo } = await import("./useSessionStore");
		expect(updateSessionAgentInfo).toHaveBeenCalledWith(
			"session-2",
			"sdk-session-2",
		);

		// Should NOT dispatch SET_AGENT_SESSION_ID since session-2 is not active
		const setAgentCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_AGENT_SESSION_ID",
		);
		expect(setAgentCalls).toHaveLength(0);
	});

	it("retries for non-active session with empty buffer content", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-2", "msg-002");
		refs.streamingBuffersRef.current.set("session-2", {
			parts: [],
		});
		refs.lastPromptsRef.current.set("session-2", "hello");
		// Active session is session-1
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-query-completed");

		cb?.({
			payload: {
				exit_code: 1,
				stderr: "error",
				chat_session_id: "session-2",
			},
		});

		expect(refs.handleRetry).toHaveBeenCalledWith("hello", "session-2");
	});

	it("cleans up buffer after completion", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdsRef.current.set("session-1", "msg-001");
		refs.streamingBuffersRef.current.set("session-1", {
			parts: [{ type: "text", content: "done" }],
		});
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [{ id: "msg-001", role: "agent", parts: [] }],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-query-completed");

		cb?.({
			payload: {
				exit_code: 0,
				stderr: "",
				chat_session_id: "session-1",
			},
		});

		expect(refs.streamingBuffersRef.current.has("session-1")).toBe(false);
	});
});

describe("result error display", () => {
	it("dispatches ADD_MESSAGE when result message has errors", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "result",
				subtype: "error_during_execution",
				chat_session_id: "session-1",
				errors: ["Authentication failed", "Please log in"],
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(1);
		const msg = (
			addMsgCalls[0][0] as {
				message: {
					role: string;
					parts: Array<{ type: string; content: string }>;
				};
			}
		).message;
		expect(msg.role).toBe("agent");
		expect(msg.parts[0].type).toBe("error");
		expect(msg.parts[0].content).toBe("Authentication failed\nPlease log in");
	});

	it("does not dispatch ADD_MESSAGE when result has no errors", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "result",
				subtype: "success",
				chat_session_id: "session-1",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch ADD_MESSAGE when result errors is empty array", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "result",
				subtype: "error_during_execution",
				chat_session_id: "session-1",
				errors: [],
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});
});
