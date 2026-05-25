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

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("./useSlashCommands", () => ({
	setSlashCommands: vi.fn(),
}));

vi.mock("./useSessionStore", async (importOriginal) => {
	const actual = await importOriginal<typeof import("./useSessionStore")>();
	return {
		...actual,
		updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
		updateSessionState: vi.fn().mockResolvedValue(undefined),
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

	it("registers listeners for agent-sdk-message, agent-session-state-changed, agent-streaming-updated, agent-pending-message-consumed, agent-permission-mode-changed, agent-models-updated, agent-backend-models-updated", () => {
		listenResolvers = [];
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));

		const eventNames = listenResolvers.map((r) => r.eventName);
		expect(eventNames).toContain("agent-sdk-message");
		expect(eventNames).toContain("agent-session-state-changed");
		expect(eventNames).toContain("agent-streaming-updated");
		expect(eventNames).toContain("agent-pending-message-consumed");
		expect(eventNames).toContain("agent-permission-mode-changed");
		expect(eventNames).toContain("agent-models-updated");
		expect(eventNames).toContain("agent-backend-models-updated");
		expect(eventNames).not.toContain("agent-streaming-started");
		expect(eventNames).not.toContain("agent-query-completed");
	});
});

describe("agent-streaming-updated event", () => {
	it("dispatches SET_STREAMING_MESSAGE when agent-streaming-updated is received for a viewable session", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-updated");
		expect(cb).toBeDefined();

		const parts = [
			{ type: "text", content: "Hello World" },
			{ type: "tool_use", tool: "Read", input: { file_path: "/a" }, id: "t1" },
		];

		await cb?.({
			payload: {
				chat_session_id: "session-1",
				message_id: "msg-001",
				parts,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_STREAMING_MESSAGE",
			sessionId: "session-1",
			messageId: "msg-001",
			parts,
		});
	});

	it("skips SET_STREAMING_MESSAGE when the session is not viewable", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		clearViewable(refs);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-streaming-updated");
		expect(cb).toBeDefined();

		await cb?.({
			payload: {
				chat_session_id: "session-hidden",
				message_id: "msg-001",
				parts: [{ type: "text", content: "noop" }],
			},
		});

		const calls = (refs.dispatch as Mock).mock.calls.map(
			(call) => (call[0] as { type: string }).type,
		);
		expect(calls).not.toContain("SET_STREAMING_MESSAGE");
		expect(calls).not.toContain("UPSERT_SESSION");
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

	it("dispatches SET_TURN_PHASE and UPDATE_SESSION_STATE on idle with exit_code", () => {
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
			state: "idle",
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
	it("calls setSlashCommands when supported_commands message is received", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

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
				type: "supported_commands",
				commands,
			},
		});

		const { setSlashCommands } = await import("./useSlashCommands");
		expect(setSlashCommands).toHaveBeenCalledWith(commands);
	});

	it("does not call setSlashCommands when commands is not an array", async () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		const { setSlashCommands } = await import("./useSlashCommands");
		vi.mocked(setSlashCommands).mockClear();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-sdk-message");

		cb?.({
			payload: {
				type: "supported_commands",
				commands: "not-an-array",
			},
		});

		expect(setSlashCommands).not.toHaveBeenCalled();
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

	it("dispatches SET_SESSION_MODEL with null when no model is selected", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-models-updated");
		cb?.({
			payload: {
				chat_session_id: "session-1",
				available_models: [],
				selected_model: null,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_SESSION_MODEL",
			sessionId: "session-1",
			modelId: null,
		});
	});
});

describe("agent-backend-models-updated event", () => {
	it("dispatches SET_BACKEND_MODELS for backend-wide model updates", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-backend-models-updated");
		expect(cb).toBeDefined();

		const models = [{ value: "gpt-5.5" }, { value: "o3" }];

		cb?.({
			payload: {
				backend_id: "codex",
				available_models: models,
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_BACKEND_MODELS",
			backendId: "codex",
			models,
		});
		// session 単位の payload と異なり、SET_SESSION_MODEL は dispatch しない。
		const sessionModelCalls = refs.dispatch.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as { type: string }).type === "SET_SESSION_MODEL",
		);
		expect(sessionModelCalls).toHaveLength(0);
	});

	it("keeps backend-wide updates even when another active session backend is selected", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		setViewable(refs, "session-1");

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-backend-models-updated");
		expect(cb).toBeDefined();

		cb?.({
			payload: {
				backend_id: "codex",
				available_models: [{ value: "gpt-5.5" }],
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_BACKEND_MODELS",
			backendId: "codex",
			models: [{ value: "gpt-5.5" }],
		});
	});

	it("keeps backend-wide updates when no active session is set", () => {
		listenResolvers = [];
		listenCallbacks.clear();
		const refs = makeRefs();
		clearViewable(refs);

		renderHook(() => useAgentSdkListeners(refs));
		for (const { resolve } of listenResolvers) resolve(vi.fn());

		const cb = listenCallbacks.get("agent-backend-models-updated");
		cb?.({
			payload: {
				backend_id: "codex",
				available_models: [{ value: "gpt-5.5" }],
			},
		});

		expect(refs.dispatch).toHaveBeenCalledWith({
			type: "SET_BACKEND_MODELS",
			backendId: "codex",
			models: [{ value: "gpt-5.5" }],
		});
	});
});
