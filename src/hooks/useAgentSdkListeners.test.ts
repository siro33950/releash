import { renderHook } from "@testing-library/react";
import { describe, expect, it, type Mock, vi } from "vitest";
import type { AgentSdkListenerRefs } from "./useAgentSdkListeners";

type ListenCallback = (event: { payload: unknown }) => void;
type UnlistenFn = Mock;

/** Test-friendly registry: tests can mutate the viewable id set directly. */
interface TestViewableRegistry {
	viewableIds: Set<string>;
	register: (sessionId: string) => () => void;
	getIds: () => Set<string>;
}

type TestRefs = Omit<AgentSdkListenerRefs, "dispatch" | "viewableRegistry"> & {
	dispatch: Mock;
	viewableRegistry: TestViewableRegistry;
};

/**
 * 旧 `activeSessionRef.current = { id }` 経路の互換 shim。テスト本文は
 * `setViewable(refs, "id")` で SDK listener が「現在 panel が表示中」と判断する
 * session id 集合を操作する。
 */
function setViewable(refs: TestRefs, ...ids: string[]): void {
	refs.viewableRegistry.viewableIds = new Set(ids);
}

function clearViewable(refs: TestRefs): void {
	refs.viewableRegistry.viewableIds = new Set();
}

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

vi.mock("./useSessionStore", async (importOriginal) => {
	const actual = await importOriginal<typeof import("./useSessionStore")>();
	return {
		...actual,
		updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
	};
});

const { useAgentSdkListeners } = await import("./useAgentSdkListeners");

function makeRefs(): TestRefs {
	const registry: TestViewableRegistry = {
		viewableIds: new Set<string>(),
		register: function (sessionId: string) {
			this.viewableIds.add(sessionId);
			return () => {
				this.viewableIds.delete(sessionId);
			};
		},
		getIds: function () {
			return new Set(this.viewableIds);
		},
	};
	return {
		dispatch: vi.fn(),
		viewableRegistry: registry,
		refreshSessions: vi.fn().mockResolvedValue(undefined),
	};
}

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

	it("registers listeners for typed agent session events", () => {
		listenResolvers = [];
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		const eventNames = listenResolvers.map((r) => r.eventName);
		expect(eventNames).toContain("agent-turn-usage-updated");
		expect(eventNames).toContain("agent-turn-prepared");
		expect(eventNames).toContain("agent-session-state-changed");
		expect(eventNames).toContain("agent-stall-observed");
		expect(eventNames).toContain("agent-stall-cleared");
		expect(eventNames).toContain("agent-streaming-delta");
		expect(eventNames).toContain("agent-pending-message-consumed");
		expect(eventNames).toContain("agent-permission-mode-changed");
		expect(eventNames).toContain("agent-models-updated");
		expect(eventNames).toContain("agent-session-context-carry-updated");
		expect(eventNames).not.toContain("agent-backend-models-updated");
		expect(eventNames).not.toContain("agent-streaming-started");
		expect(eventNames).not.toContain("agent-query-completed");
	});
});

describe("agent-stall-observed event", () => {
	it("dispatches SET_STALL_OBSERVATION without changing turn phase", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-stall-observed");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "streaming",
				idle_secs: "180",
				signal_count: "2",
				cap_reached: false,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STALL_OBSERVATION",
			sessionId: "session-1",
			observation: {
				turnPhase: "streaming",
				idleSecs: 180,
				signalCount: 2,
				capReached: false,
			},
		});
		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "SET_TURN_PHASE" }),
		);
	});
});

describe("agent-stall-cleared event", () => {
	it("clears the session stall observation", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-stall-cleared");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "CLEAR_STALL_OBSERVATION",
			sessionId: "session-1",
		});
	});
});

describe("agent-turn-prepared event", () => {
	it("mirrors prepared session and placeholder messages without viewable gating", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		clearViewable(refs);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-turn-prepared");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				session: {
					id: "session-1",
					worktreePath: "/repo",
					messages: [],
					state: "active",
					createdAt: 1,
					updatedAt: 2,
					permissionMode: "edit",
					selectedModel: "",
				},
				human_message: {
					id: "human-1",
					role: "human",
					content: "hello",
					timestamp: 3,
				},
				agent_message: {
					id: "agent-1",
					role: "agent",
					content: "",
					parts: [],
					timestamp: 4,
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "UPSERT_SESSION",
			session: expect.objectContaining({
				id: "session-1",
				messages: [],
				permissionMode: "edit",
			}),
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "ADD_MESSAGE",
			sessionId: "session-1",
			message: {
				id: "human-1",
				role: "human",
				parts: [{ type: "text", content: "hello" }],
				timestamp: 3,
				mentions: undefined,
			},
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "ADD_MESSAGE",
			sessionId: "session-1",
			message: {
				id: "agent-1",
				role: "agent",
				parts: [],
				timestamp: 4,
				mentions: undefined,
			},
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "CLEAR_STALL_OBSERVATION",
			sessionId: "session-1",
		});
	});

	it("ignores prepared sessions from another worktree", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.worktreePath = "/repo-a";

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-turn-prepared");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-b",
				session: {
					id: "session-b",
					worktreePath: "/repo-b",
					messages: [],
					state: "active",
					createdAt: 1,
					updatedAt: 2,
					permissionMode: "edit",
				},
				human_message: {
					id: "human-b",
					role: "human",
					content: "hello",
					timestamp: 3,
				},
				agent_message: {
					id: "agent-b",
					role: "agent",
					content: "",
					parts: [],
					timestamp: 4,
				},
			},
		});

		expect(refs.dispatch).not.toHaveBeenCalled();
	});
});

describe("agent-session-context-carry-updated event", () => {
	it("dispatches SET_CONTEXT_CARRY for a viewable session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-session-context-carry-updated");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				agent_session_id: null,
				context_carry: "failed",
				updated_at: 1234,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_CONTEXT_CARRY",
			sessionId: "session-1",
			agentSessionId: null,
			contextCarry: "failed",
			updatedAt: 1234,
		});
	});

	it("skips SET_CONTEXT_CARRY for hidden sessions", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		clearViewable(refs);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-session-context-carry-updated");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "hidden-session",
				agent_session_id: null,
				context_carry: "failed",
				updated_at: 1234,
			},
		});

		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "SET_CONTEXT_CARRY" }),
		);
	});
});

describe("agent-streaming-delta event", () => {
	it("dispatches APPLY_STREAMING_DELTA for append events from a viewable session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		const parts = [
			{ type: "text", content: "Hello World" },
			{ type: "tool_use", tool: "Read", input: { file_path: "/a" }, id: "t1" },
		];

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: "1",
				snapshot: false,
				parts,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: "1",
			parts,
		});
	});

	it("warns before dispatching append deltas for a missing session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.getStreamingDeltaDropReason = vi
			.fn()
			.mockReturnValue("missing_session");
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "missing-session",
				message_id: "msg-001",
				seq: "1",
				snapshot: false,
				parts: [{ type: "text", content: "Hello" }],
			},
		});

		expect(refs.getStreamingDeltaDropReason).toHaveBeenCalledWith(
			"missing-session",
			"msg-001",
		);
		expect(warn).toHaveBeenCalledWith(
			"Dropped agent-streaming-delta for missing session",
			{
				sessionId: "missing-session",
				messageId: "msg-001",
				seq: "1",
			},
		);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "missing-session",
			messageId: "msg-001",
			seq: "1",
			parts: [{ type: "text", content: "Hello" }],
		});
		warn.mockRestore();
	});

	it("warns before dispatching append deltas for a missing message", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.getStreamingDeltaDropReason = vi
			.fn()
			.mockReturnValue("missing_message");
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "missing-message",
				seq: "2",
				snapshot: false,
				parts: [{ type: "text", content: "Hello" }],
			},
		});

		expect(warn).toHaveBeenCalledWith(
			"Dropped agent-streaming-delta for missing message",
			{
				sessionId: "session-1",
				messageId: "missing-message",
				seq: "2",
			},
		);
		warn.mockRestore();
	});

	it("dispatches SET_STREAMING_MESSAGE for snapshot events from a viewable session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		const parts = [{ type: "text" as const, content: "resynced" }];
		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: "2",
				snapshot: true,
				parts,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "msg-001",
			parts,
		});
		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "APPLY_STREAMING_DELTA" }),
		);
	});

	it("mirrors backend message metadata before applying an unknown-message snapshot", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.getStreamingDeltaDropReason = vi
			.fn()
			.mockReturnValue("missing_message");
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		const parts = [{ type: "error" as const, content: "app server stopped" }];
		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "fatal-message-1",
				seq: "1",
				snapshot: true,
				parts,
				message: {
					id: "fatal-message-1",
					role: "agent",
					content: "app server stopped",
					parts,
					timestamp: 4321,
				},
			},
		});

		expect(refs.dispatch).toHaveBeenNthCalledWith(2, {
			type: "ADD_MESSAGE",
			sessionId: "session-1",
			message: {
				id: "fatal-message-1",
				role: "agent",
				parts,
				timestamp: 4321,
				mentions: undefined,
			},
		});
		expect(refs.dispatch).toHaveBeenNthCalledWith(3, {
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "fatal-message-1",
			parts,
		});
		expect(warn).not.toHaveBeenCalled();
		warn.mockRestore();
	});

	it("warns before dispatching snapshot deltas for a missing message", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		refs.getStreamingDeltaDropReason = vi
			.fn()
			.mockReturnValue("missing_message");
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		const parts = [{ type: "text" as const, content: "resynced" }];
		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "missing-message",
				seq: "3",
				snapshot: true,
				parts,
			},
		});

		expect(refs.getStreamingDeltaDropReason).toHaveBeenCalledWith(
			"session-1",
			"missing-message",
		);
		expect(warn).toHaveBeenCalledWith(
			"Dropped agent-streaming-delta for missing message",
			{
				sessionId: "session-1",
				messageId: "missing-message",
				seq: "3",
			},
		);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "missing-message",
			parts,
		});
		warn.mockRestore();
	});

	it("does not inspect seq continuity or drop duplicate-looking append events", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: "10",
				snapshot: false,
				parts: [{ type: "text", content: "first" }],
			},
		});
		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: "10",
				snapshot: false,
				parts: [{ type: "text", content: "duplicate-looking" }],
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: "10",
			parts: [{ type: "text", content: "first" }],
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: "10",
			parts: [{ type: "text", content: "duplicate-looking" }],
		});
	});

	it("dispatches streaming events even when the session is not viewable", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		clearViewable(refs);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-hidden",
				message_id: "msg-001",
				seq: "1",
				snapshot: true,
				parts: [{ type: "text", content: "noop" }],
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-hidden",
			messageId: "msg-001",
			parts: [{ type: "text", content: "noop" }],
		});
	});
});

describe("agent-session-state-changed event", () => {
	it("mirrors backend-owned queue pause changes", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-session-state-changed");
		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "streaming",
				exit_code: null,
				queue_paused: true,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_QUEUE_PAUSED",
			sessionId: "session-1",
			value: true,
		});
	});

	it("dispatches SET_TURN_PHASE when streaming phase is received", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "streaming",
				exit_code: null,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_TURN_PHASE",
			sessionId: "session-1",
			turnPhase: "streaming",
		});
	});

	it("mirrors session_state from idle completion payload", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 0,
				session_state: "done",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_TURN_PHASE",
			sessionId: "session-1",
			turnPhase: "idle",
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "UPDATE_SESSION_STATE",
			sessionId: "session-1",
			state: "done",
		});
	});

	it("does not derive session state from exit_code when payload omits session_state", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 0,
			},
		});

		expect(refs.dispatch).not.toHaveBeenCalledWith({
			type: "UPDATE_SESSION_STATE",
			sessionId: "session-1",
			state: "done",
		});
	});

	it("keeps interrupted idle events distinct from completed turns", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 0,
				completed_at: 1234,
				interrupted: true,
				session_state: "idle",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_TURN_PHASE",
			sessionId: "session-1",
			turnPhase: "idle",
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request: null,
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "UPDATE_SESSION_STATE",
			sessionId: "session-1",
			state: "idle",
		});
		expect(refs.dispatch).not.toHaveBeenCalledWith({
			type: "MARK_AGENT_TURN_COMPLETED",
			sessionId: "session-1",
			completedAt: 1234,
		});
	});

	it("finalizes the live error message timestamp for an interrupted crash", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-session-state-changed");
		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 1,
				completed_at: 4321,
				interrupted: true,
				session_state: "error",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "MARK_AGENT_TURN_COMPLETED",
			sessionId: "session-1",
			completedAt: 4321,
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "UPDATE_SESSION_STATE",
			sessionId: "session-1",
			state: "error",
		});
	});

	it("does not finalize prior history when an Idle-Fatal snapshot is retried after state change", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.getStreamingDeltaDropReason = vi
			.fn()
			.mockReturnValue("missing_message");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		listenCallbacks.get("agent-session-state-changed")?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 1,
				interrupted: true,
				session_state: "error",
			},
		});
		const parts = [{ type: "error" as const, content: "app server stopped" }];
		await listenCallbacks.get("agent-streaming-delta")?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "fatal-message-1",
				seq: "1",
				snapshot: true,
				parts,
				message: {
					id: "fatal-message-1",
					role: "agent",
					content: "app server stopped",
					parts,
					timestamp: 4321,
				},
			},
		});

		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "MARK_AGENT_TURN_COMPLETED" }),
		);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "ADD_MESSAGE",
			sessionId: "session-1",
			message: {
				id: "fatal-message-1",
				role: "agent",
				parts,
				timestamp: 4321,
				mentions: undefined,
			},
		});
	});

	it("mirrors pending permission request from session state payload", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		const request = {
			id: "req-001",
			toolName: "Edit",
			input: { file_path: "/src/index.ts" },
			toolUseId: "toolu_001",
			title: "Edit file",
		};

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "waiting_permission",
				exit_code: null,
				pending_permission_request: request,
				pending_permission_state_revision: "5",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_TURN_PHASE",
			sessionId: "session-1",
			turnPhase: "waiting_permission",
			pendingPermissionStateRevision: "5",
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request,
			pendingPermissionStateRevision: "5",
		});
	});

	it("dispatches SET_PENDING_PERMISSION null on idle state change", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "idle",
				exit_code: 0,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request: null,
		});
	});

	it("dispatches SET_PENDING_PERMISSION null when waiting payload omits request", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		for (const { resolve } of listenResolvers) {
			resolve(vi.fn());
		}

		const cb = listenCallbacks.get("agent-session-state-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				turn_phase: "waiting_permission",
				exit_code: null,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PENDING_PERMISSION",
			sessionId: "session-1",
			request: null,
		});
	});
});

describe("SET_PERMISSION_MODE from agent-permission-mode-changed event", () => {
	it("dispatches SET_PERMISSION_MODE when agent-permission-mode-changed is received for active session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-permission-mode-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				permission_mode: "ask",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_PERMISSION_MODE",
			sessionId: "session-1",
			mode: "ask",
		});
	});

	it("does not dispatch SET_PERMISSION_MODE when agent-permission-mode-changed is for non-active session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-2");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-permission-mode-changed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				permission_mode: "ask",
			},
		});

		const permModeCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_PERMISSION_MODE",
		);
		expect(permModeCalls).toHaveLength(0);
	});
});

describe("agent-turn-usage-updated event", () => {
	it("dispatches latest token usage from typed Rust event", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-turn-usage-updated");

		cb?.({
			payload: {
				chatSessionId: "session-1",
				tokenUsage: {
					inputTokens: "15",
					outputTokens: "38",
					totalTokens: "53",
					contextWindowTokens: "200000",
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_LATEST_TOKEN_USAGE",
			sessionId: "session-1",
			usage: {
				inputTokens: 15,
				outputTokens: 38,
				totalTokens: 53,
				contextWindowTokens: 200000,
			},
		});
	});
});

describe("supported_commands handling", () => {
	it("stores runtime slash commands when supported commands are updated", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-supported-commands-updated");

		const commands = [
			{ name: "plan-spec", description: "Create plan spec" },
			{
				name: "review",
				description: "Code review",
				argumentHint: "<file>",
			},
		];

		cb?.({
			payload: {
				chat_session_id: "session-1",
				commands,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_RUNTIME_SLASH_COMMANDS",
			sessionId: "session-1",
			commands,
		});
	});
});

describe("agent-pending-message-consumed event", () => {
	it("dispatches ADD_MESSAGE when pending message is consumed for active session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-pending-message-consumed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				agent_message: {
					id: "msg-agent-001",
					role: "agent",
					parts: [
						{
							type: "system_notification",
							notificationType: "session_recovery",
							status: "recovered",
							label: "backend セッションを作り直したため文脈は引き継がれません",
						},
					],
					timestamp: 1234567,
				},
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "ADD_MESSAGE",
			sessionId: "session-1",
			message: {
				id: "msg-agent-001",
				role: "agent",
				parts: [
					{
						type: "system_notification",
						notificationType: "session_recovery",
						status: "recovered",
						label: "backend セッションを作り直したため文脈は引き継がれません",
					},
				],
				timestamp: 1234567,
			},
		});
	});

	it("does not dispatch ADD_MESSAGE when pending message is consumed for non-active session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-2");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-pending-message-consumed");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				chat_session_id: "session-1",
				agent_message: {
					id: "msg-agent-001",
					role: "agent",
					timestamp: 1234567,
				},
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});
});

describe("agent-models-updated event", () => {
	it("dispatches SET_AVAILABLE_MODELS and SET_SESSION_MODEL when agent-models-updated is received for the active session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-models-updated");
		expect(cb).toBeDefined();

		const models = [{ value: "claude-4" }, { value: "claude-3.5-sonnet" }];

		cb?.({
			payload: {
				chat_session_id: "session-1",
				available_models: models,
				selected_model: "claude-4",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_AVAILABLE_MODELS",
			models,
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_SESSION_MODEL",
			sessionId: "session-1",
			modelId: "claude-4",
		});
	});

	it("does not dispatch SET_AVAILABLE_MODELS for another session", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-2");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-models-updated");
		cb?.({
			payload: {
				chat_session_id: "session-1",
				available_models: [{ value: "claude-4" }],
				selected_model: "claude-4",
			},
		});

		const availCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_AVAILABLE_MODELS",
		);
		expect(availCalls).toHaveLength(0);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_SESSION_MODEL",
			sessionId: "session-1",
			modelId: "claude-4",
		});
	});

	it("dispatches SET_SESSION_MODEL with the non-null default model from Rust", () => {
		// 契約: agent-models-updated の selected_model は常に非 null（Rust が既存
		// セッションの None をデフォルトに解決してから送る）。null を SET_SESSION_MODEL
		// に流す経路は存在しない。
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-models-updated");
		cb?.({
			payload: {
				chat_session_id: "session-1",
				available_models: [{ value: "claude-opus-4-8" }],
				selected_model: "claude-opus-4-8",
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_SESSION_MODEL",
			sessionId: "session-1",
			modelId: "claude-opus-4-8",
		});
		const sessionModelCalls = (refs.dispatch.mock.calls as unknown[][]).filter(
			(call) => (call[0] as { type: string }).type === "SET_SESSION_MODEL",
		);
		for (const call of sessionModelCalls) {
			expect((call[0] as { modelId: unknown }).modelId).not.toBeNull();
		}
	});
});
