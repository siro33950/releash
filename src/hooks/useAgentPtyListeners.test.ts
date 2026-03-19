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

vi.mock("./useSessionStore", () => ({
	updateMessageContent: vi.fn().mockResolvedValue(undefined),
	updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
	updateSessionState: vi.fn().mockResolvedValue(undefined),
}));

const {
	useAgentPtyListeners,
	extractStreamingDelta,
	extractToolResultContent,
	shouldRetry,
} = await import("./useAgentPtyListeners");

function makeRefs() {
	return {
		dispatch: vi.fn(),
		streamingMessageIdRef: { current: null as string | null },
		activeSessionRef: { current: null },
		refreshSessions: vi.fn().mockResolvedValue(undefined),
		handleRetry: vi.fn().mockResolvedValue(undefined),
		isRetryingRef: { current: false },
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

describe("useAgentPtyListeners cancelled flag", () => {
	it("calls unlisten immediately if cleanup happens before listen resolves", async () => {
		listenResolvers = [];
		const refs = makeRefs();

		const { unmount } = renderHook(() => useAgentPtyListeners(refs));

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

		const { unmount } = renderHook(() => useAgentPtyListeners(refs));

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

		renderHook(() => useAgentPtyListeners(refs));

		const eventNames = listenResolvers.map((r) => r.eventName);
		expect(eventNames).toContain("agent-sdk-message");
		expect(eventNames).toContain("agent-query-completed");
		expect(eventNames).not.toContain("agent-state-changed");
	});
});

describe("useAgentPtyListeners callback behavior", () => {
	it("dispatches APPEND_STREAMING when stream_event with text_delta is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));

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

		renderHook(() => useAgentPtyListeners(refs));

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
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_AGENT_SESSION_ID",
			agentSessionId: "sdk-session-abc",
		});
	});

	it("dispatches SET_STREAMING false when agent-query-completed is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdRef.current = "msg-002";
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [{ id: "msg-002", role: "agent", content: "response text" }],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-query-completed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				exit_code: 0,
				stderr: "",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING",
			streaming: false,
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
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "stream_event",
				session_id: "sid-123",
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
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "assistant",
				session_id: "sid-123",
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
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "user",
				session_id: "sid-123",
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
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "user",
				session_id: "sid-123",
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

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		const request = {
			type: "permission_request",
			request_id: "req-001",
			tool_name: "Edit",
			input: { file_path: "/src/index.ts" },
			tool_use_id: "toolu_001",
			title: "Edit file",
		};

		cb?.({ payload: request });

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			request,
		});
	});

	it("dispatches SET_PENDING_PERMISSION null on agent-query-completed", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdRef.current = "msg-002";
		refs.activeSessionRef.current = {
			id: "session-1",
			worktreePath: "/repo",
			state: "idle",
			messages: [{ id: "msg-002", role: "agent", content: "response text" }],
			createdAt: Date.now(),
			agentSessionId: null,
		} as never;

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-query-completed");
		expect(cb).toBeDefined();

		cb?.({ payload: { exit_code: 0, stderr: "" } });

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			request: null,
		});
	});

	it("does NOT dispatch APPEND_STREAMING when streamingMessageIdRef is null", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdRef.current = null; // No active streaming

		renderHook(() => useAgentPtyListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-sdk-message");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				type: "stream_event",
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

		renderHook(() => useAgentPtyListeners(refs));
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

		renderHook(() => useAgentPtyListeners(refs));
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

		renderHook(() => useAgentPtyListeners(refs));
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

	it("does not dispatch SET_PLAN_MODE_ACTIVE for EnterPlanMode tool_use (removed)", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.streamingMessageIdRef.current = "msg-001";

		renderHook(() => useAgentPtyListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "assistant",
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
				{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				{ id: "m2", role: "agent", content: "", timestamp: 1001 },
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
				{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				{ id: "m2", role: "agent", content: "", timestamp: 1001 },
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
				{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				{ id: "m2", role: "agent", content: "", timestamp: 1001 },
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
				{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				{ id: "m2", role: "agent", content: "response", timestamp: 1001 },
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
				{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				{ id: "m2", role: "agent", content: "", timestamp: 1001 },
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
