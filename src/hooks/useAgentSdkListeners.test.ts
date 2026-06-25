import { renderHook } from "@testing-library/react";
import { describe, expect, it, type Mock, vi } from "vitest";
import type {
	AgentSdkListenerRefs,
	ViewableSessionRegistry,
} from "./useAgentSdkListeners";

type ListenCallback = (event: { payload: unknown }) => void;
type UnlistenFn = Mock;

/** Test-friendly registry: tests can mutate the viewable id set directly. */
interface TestViewableRegistry extends ViewableSessionRegistry {
	viewableIds: Set<string>;
}

type TestRefs = Omit<
	AgentSdkListenerRefs,
	"dispatch" | "getLastStreamingSeq" | "hasMessage" | "viewableRegistry"
> & {
	dispatch: Mock;
	getLastStreamingSeq: Mock;
	hasMessage: Mock;
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

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("./useSessionStore", async (importOriginal) => {
	const actual = await importOriginal<typeof import("./useSessionStore")>();
	return {
		...actual,
		getSession: vi.fn().mockResolvedValue(null),
		resyncStreamingMessage: vi.fn().mockResolvedValue(null),
		updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
		updateSessionState: vi.fn().mockResolvedValue(undefined),
	};
});

const { useAgentSdkListeners } = await import("./useAgentSdkListeners");
const sessionStore = await import("./useSessionStore");

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
		hasMessage: vi.fn().mockReturnValue(false),
		getLastStreamingSeq: vi.fn().mockReturnValue(0),
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

	it("registers listeners for agent-sdk-message, agent-session-state-changed, agent-streaming-delta, agent-pending-message-consumed, agent-permission-mode-changed, agent-models-updated", () => {
		listenResolvers = [];
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		const eventNames = listenResolvers.map((r) => r.eventName);
		expect(eventNames).toContain("agent-sdk-message");
		expect(eventNames).toContain("agent-session-state-changed");
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
	it("dispatches APPLY_STREAMING_DELTA when agent-streaming-delta is received in sequence for a viewable cached message", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(0);

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
				seq: 1,
				parts,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 1,
			parts,
		});
	});

	it("does not resync consecutive deltas that arrive before React state ref reflects the first dispatch", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(0);
		vi.mocked(sessionStore.resyncStreamingMessage).mockClear();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 1,
				parts: [{ type: "text", content: "Hello" }],
			},
		});
		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 2,
				parts: [{ type: "text", content: " World" }],
			},
		});

		expect(sessionStore.resyncStreamingMessage).not.toHaveBeenCalled();
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 1,
			parts: [{ type: "text", content: "Hello" }],
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 2,
			parts: [{ type: "text", content: " World" }],
		});
	});

	it("skips APPLY_STREAMING_DELTA when the session is not viewable", async () => {
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
				seq: 1,
				parts: [{ type: "text", content: "noop" }],
			},
		});

		const calls = (refs.dispatch as Mock).mock.calls.map(
			(call) => (call[0] as { type: string }).type,
		);
		expect(calls).not.toContain("APPLY_STREAMING_DELTA");
		expect(calls).not.toContain("UPSERT_SESSION");
	});

	it("requests focused resync when a delta seq gap is detected", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(1);
		vi.mocked(sessionStore.resyncStreamingMessage).mockResolvedValueOnce({
			session_id: "session-1",
			message_id: "msg-001",
			seq: 3,
			parts: [{ type: "text", content: "resynced" }],
		});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "late" }],
			},
		});

		expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledWith(
			"session-1",
			"msg-001",
			1,
		);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 3,
			parts: [{ type: "text", content: "resynced" }],
		});
	});

	it("logs resync failures and retries on a later delta", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(1);
		const error = new Error("resync failed");
		const consoleError = vi
			.spyOn(console, "error")
			.mockImplementation(() => undefined);
		vi.mocked(sessionStore.resyncStreamingMessage)
			.mockReset()
			.mockRejectedValueOnce(error)
			.mockResolvedValueOnce({
				session_id: "session-1",
				message_id: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "resynced" }],
			});

		try {
			renderHook(() => useAgentSdkListeners(refs));
			for (const { resolve } of listenResolvers) resolve(vi.fn());

			const cb = listenCallbacks.get("agent-streaming-delta");
			expect(cb).toBeDefined();

			const first = cb?.({
				payload: {
					chat_session_id: "session-1",
					message_id: "msg-001",
					seq: 3,
					parts: [{ type: "text", content: "late" }],
				},
			}) as Promise<void> | undefined;
			await expect(first).resolves.toBeUndefined();
			expect(consoleError).toHaveBeenCalledWith(
				"Failed to resync streaming message:",
				error,
			);

			await cb?.({
				payload: {
					chat_session_id: "session-1",
					message_id: "msg-001",
					seq: 3,
					parts: [{ type: "text", content: "late retry" }],
				},
			});

			expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledTimes(2);
			expect(sessionStore.resyncStreamingMessage).toHaveBeenNthCalledWith(
				2,
				"session-1",
				"msg-001",
				1,
			);
			expect(refs.dispatch).toHaveBeenCalledWith({
				type: "SET_STREAMING_MESSAGE",
				sessionId: "session-1",
				messageId: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "resynced" }],
			});
		} finally {
			consoleError.mockRestore();
			vi.mocked(sessionStore.resyncStreamingMessage)
				.mockReset()
				.mockResolvedValue(null);
		}
	});

	it("applies the next delta after a resync snapshot before React state ref reflects the snapshot seq", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(1);
		vi.mocked(sessionStore.resyncStreamingMessage)
			.mockReset()
			.mockResolvedValueOnce({
				session_id: "session-1",
				message_id: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "resynced" }],
			});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "late" }],
			},
		});
		refs.dispatch.mockClear();

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 4,
				parts: [{ type: "text", content: "next" }],
			},
		});

		expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledTimes(1);
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 4,
			parts: [{ type: "text", content: "next" }],
		});
	});

	it("logs hydration failures and retries hydration on a later delta", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(false);
		refs.getLastStreamingSeq.mockReturnValue(0);
		const error = new Error("hydrate failed");
		const consoleError = vi
			.spyOn(console, "error")
			.mockImplementation(() => undefined);
		vi.mocked(sessionStore.resyncStreamingMessage)
			.mockReset()
			.mockResolvedValue(null);
		vi.mocked(sessionStore.getSession)
			.mockReset()
			.mockRejectedValueOnce(error)
			.mockResolvedValueOnce(null);

		try {
			renderHook(() => useAgentSdkListeners(refs));
			for (const { resolve } of listenResolvers) resolve(vi.fn());

			const cb = listenCallbacks.get("agent-streaming-delta");
			expect(cb).toBeDefined();

			const first = cb?.({
				payload: {
					chat_session_id: "session-1",
					message_id: "msg-001",
					seq: 1,
					parts: [{ type: "text", content: "late" }],
				},
			}) as Promise<void> | undefined;
			await expect(first).resolves.toBeUndefined();
			expect(consoleError).toHaveBeenCalledWith(
				"Failed to hydrate streaming message:",
				error,
			);

			await cb?.({
				payload: {
					chat_session_id: "session-1",
					message_id: "msg-001",
					seq: 1,
					parts: [{ type: "text", content: "late retry" }],
				},
			});

			expect(sessionStore.getSession).toHaveBeenCalledTimes(2);
			expect(sessionStore.getSession).toHaveBeenNthCalledWith(2, "session-1");
			expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledTimes(2);
		} finally {
			consoleError.mockRestore();
			vi.mocked(sessionStore.getSession).mockReset().mockResolvedValue(null);
			vi.mocked(sessionStore.resyncStreamingMessage)
				.mockReset()
				.mockResolvedValue(null);
		}
	});

	it("hydrates missing messages per session and leaves seq retryable while uncached", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "free-session", "workflow-step-session");
		refs.hasMessage.mockReturnValue(false);
		refs.getLastStreamingSeq.mockReturnValue(0);
		vi.mocked(sessionStore.resyncStreamingMessage)
			.mockReset()
			.mockImplementation(async (sessionId, messageId) => ({
				session_id: sessionId,
				message_id: messageId,
				seq: 1,
				parts: [{ type: "text", content: `snapshot:${sessionId}` }],
			}));
		let resolveFreeHydrate: ((value: null) => void) | undefined;
		vi.mocked(sessionStore.getSession)
			.mockReset()
			.mockImplementation((sessionId) => {
				if (sessionId === "free-session") {
					return new Promise((resolve) => {
						resolveFreeHydrate = resolve;
					});
				}
				return Promise.resolve(null);
			});

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		const freeFirst = cb?.({
			payload: {
				chat_session_id: "free-session",
				message_id: "msg-free",
				seq: 1,
				parts: [{ type: "text", content: "free delta" }],
			},
		}) as Promise<void> | undefined;
		await vi.waitFor(() => {
			expect(sessionStore.getSession).toHaveBeenCalledWith("free-session");
		});

		await cb?.({
			payload: {
				chat_session_id: "workflow-step-session",
				message_id: "msg-workflow",
				seq: 1,
				parts: [{ type: "text", content: "workflow delta" }],
			},
		});
		expect(sessionStore.getSession).toHaveBeenCalledWith(
			"workflow-step-session",
		);

		resolveFreeHydrate?.(null);
		await freeFirst;
		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "SET_STREAMING_MESSAGE" }),
		);
		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "APPLY_STREAMING_DELTA" }),
		);

		refs.hasMessage.mockReturnValue(true);
		refs.dispatch.mockClear();
		await cb?.({
			payload: {
				chat_session_id: "free-session",
				message_id: "msg-free",
				seq: 1,
				parts: [{ type: "text", content: "free delta" }],
			},
		});
		await cb?.({
			payload: {
				chat_session_id: "workflow-step-session",
				message_id: "msg-workflow",
				seq: 1,
				parts: [{ type: "text", content: "workflow delta" }],
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "free-session",
			messageId: "msg-free",
			seq: 1,
			parts: [{ type: "text", content: "free delta" }],
		});
		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "APPLY_STREAMING_DELTA",
			sessionId: "workflow-step-session",
			messageId: "msg-workflow",
			seq: 1,
			parts: [{ type: "text", content: "workflow delta" }],
		});
	});

	it("runs another resync when a newer delta arrives while resync is in flight", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");
		refs.hasMessage.mockReturnValue(true);
		refs.getLastStreamingSeq.mockReturnValue(1);

		let resolveFirst:
			| ((value: {
					session_id: string;
					message_id: string;
					seq: number;
					parts: Array<{ type: "text"; content: string }>;
			  }) => void)
			| undefined;
		let resolveSecond:
			| ((value: {
					session_id: string;
					message_id: string;
					seq: number;
					parts: Array<{ type: "text"; content: string }>;
			  }) => void)
			| undefined;
		vi.mocked(sessionStore.resyncStreamingMessage)
			.mockReset()
			.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveFirst = resolve;
					}),
			)
			.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveSecond = resolve;
					}),
			);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-delta");
		expect(cb).toBeDefined();

		const first = cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 3,
				parts: [{ type: "text", content: "late" }],
			},
		});
		await vi.waitFor(() => {
			expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledWith(
				"session-1",
				"msg-001",
				1,
			);
		});

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				seq: 4,
				parts: [{ type: "text", content: "newer" }],
			},
		});
		expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledTimes(1);

		resolveFirst?.({
			session_id: "session-1",
			message_id: "msg-001",
			seq: 3,
			parts: [{ type: "text", content: "up to 3" }],
		});
		await vi.waitFor(() => {
			expect(sessionStore.resyncStreamingMessage).toHaveBeenCalledWith(
				"session-1",
				"msg-001",
				3,
			);
		});
		resolveSecond?.({
			session_id: "session-1",
			message_id: "msg-001",
			seq: 4,
			parts: [{ type: "text", content: "up to 4" }],
		});
		await first;

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "msg-001",
			seq: 4,
			parts: [{ type: "text", content: "up to 4" }],
		});
	});
});

describe("agent-session-state-changed event", () => {
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

	it("dispatches SET_TURN_PHASE and UPDATE_SESSION_STATE (done) on idle with exit_code", () => {
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
			request: {
				request_id: "req-001",
				tool_name: "Edit",
				input: { file_path: "/src/index.ts" },
				tool_use_id: "toolu_001",
				title: "Edit file",
				display_name: undefined,
				description: undefined,
				decision_reason: undefined,
			},
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
		setViewable(refs, "session-1");

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
		setViewable(refs, "session-1");

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
				permissionMode: "ask",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch ADD_MESSAGE for task system messages (task_started)", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "task_started",
				chat_session_id: "session-1",
				tool_use_id: "toolu_task_001",
				description: "Explore codebase",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch ADD_MESSAGE for task system messages (task_notification)", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "task_notification",
				chat_session_id: "session-1",
				tool_use_id: "toolu_task_001",
				status: "completed",
				summary: "Done",
			},
		});

		const addMsgCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) => (call[0] as { type: string }).type === "ADD_MESSAGE",
		);
		expect(addMsgCalls).toHaveLength(0);
	});

	it("does not dispatch ADD_MESSAGE for task system messages (task_progress)", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "system",
				subtype: "task_progress",
				chat_session_id: "session-1",
				tool_use_id: "toolu_task_001",
				description: "Processing files",
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

describe("result error display", () => {
	it("dispatches latest token usage from result modelUsage", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "result",
				chat_session_id: "session-1",
				modelUsage: {
					codex: {
						inputTokens: 12,
						outputTokens: 34,
						totalTokens: 46,
						contextWindowTokens: 200000,
					},
					planner: {
						inputTokens: 3,
						outputTokens: 4,
						totalTokens: 7,
						contextWindowTokens: 128000,
					},
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

	it("dispatches ADD_MESSAGE when result message has errors", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

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

	it("does not treat raw agent-sdk-message supported_commands as slash catalog", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "supported_commands",
				commands: "not-an-array",
			},
		});

		expect(refs.dispatch).not.toHaveBeenCalledWith(
			expect.objectContaining({ type: "SET_RUNTIME_SLASH_COMMANDS" }),
		);
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
				parts: [],
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
