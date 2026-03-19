import { beforeEach, describe, expect, it, vi } from "vitest";

const mockInvoke = vi.fn().mockResolvedValue(undefined);

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const listenCallbacks: Map<string, (event: { payload: unknown }) => void> =
	new Map();

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(
		(eventName: string, cb: (event: { payload: unknown }) => void) => {
			listenCallbacks.set(eventName, cb);
			return Promise.resolve(vi.fn());
		},
	),
}));

vi.mock("./useSessionStore", () => ({
	listSessions: vi.fn().mockResolvedValue([]),
	getSession: vi.fn().mockResolvedValue(null),
	createSession: vi.fn().mockResolvedValue({
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
	}),
	addMessage: vi.fn().mockImplementation((_sid, role, content) =>
		Promise.resolve({
			id: `msg-${Date.now()}`,
			role,
			content,
			timestamp: Date.now(),
		}),
	),
	updateSessionState: vi.fn().mockResolvedValue(undefined),
	updateMessageContent: vi.fn().mockResolvedValue(undefined),
	updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
}));

describe("useAgentChat", () => {
	beforeEach(() => {
		mockInvoke.mockClear();
	});

	it("should define the hook", async () => {
		const mod = await import("./useAgentChat");
		expect(mod.useAgentChat).toBeDefined();
		expect(typeof mod.useAgentChat).toBe("function");
	});

	it("should not export buildClaudeCommand (removed)", async () => {
		const mod = await import("./useAgentChat");
		expect((mod as Record<string, unknown>).buildClaudeCommand).toBeUndefined();
	});

	it("sendMessage passes permissionMode to invoke", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"execute_agent_query",
			expect.objectContaining({
				permissionMode: "acceptEdits",
			}),
		);
	});

	it("respondPermission invokes respond_agent_permission", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.respondPermission("req-001", true);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			requestId: "req-001",
			behavior: "allow",
			message: null,
			updatedInput: null,
		});
	});

	it("respondPermission with deny sends deny behavior", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.respondPermission("req-002", false);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			requestId: "req-002",
			behavior: "deny",
			message: "User denied",
			updatedInput: null,
		});
	});

	it("sendMessage calls createSession when no active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(sessionStore.createSession).toHaveBeenCalledWith("/repo");
	});

	it("sendMessage does not call createSession on second message", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("first");
		});

		vi.mocked(sessionStore.createSession).mockClear();

		await act(async () => {
			await result.current.sendMessage("second");
		});

		expect(sessionStore.createSession).not.toHaveBeenCalled();
	});

	it("interrupt invokes interrupt_agent_query", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.interrupt();
		});

		expect(mockInvoke).toHaveBeenCalledWith("interrupt_agent_query");
	});

	it("selectSession calls getSession and updates activeSession", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const mockSession = {
			id: "s2",
			worktreePath: "/repo",
			messages: [{ id: "m1", role: "human", content: "hi", timestamp: 1000 }],
			state: "idle",
			createdAt: 1000,
			updatedAt: 1000,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			mockSession as never,
		);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(sessionStore.getSession).toHaveBeenCalledWith("s2");
		expect(result.current.activeSession).toEqual(mockSession);
	});

	it("setPermissionMode changes mode used in next sendMessage", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.setPermissionMode("plan" as never);
		});

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"execute_agent_query",
			expect.objectContaining({
				permissionMode: "plan",
			}),
		);
	});

	it("respondPermission for ExitPlanMode sends { behavior: allow } without updatedInput", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.respondPermission("req-exitplan-001", true);
		});

		// ExitPlanMode allow response must be exactly this format.
		// The bridge resolves canUseTool with result.result, which becomes { behavior: "allow" }.
		// If the SDK rejects this with ZodError, the issue is in the SDK's schema.
		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			requestId: "req-exitplan-001",
			behavior: "allow",
			message: null,
			updatedInput: null,
		});
	});

	it("respondPermission passes updatedInput as JSON string", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		const updatedInput = {
			questions: [
				{ question: "Pick one", header: "Q", options: [], multiSelect: false },
			],
			answers: { "Pick one": "A" },
		};

		act(() => {
			result.current.respondPermission("req-003", true, updatedInput);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			requestId: "req-003",
			behavior: "allow",
			message: null,
			updatedInput: JSON.stringify(updatedInput),
		});
	});
});

describe("permissionMode sync from SDK system messages", () => {
	it("syncs permissionMode when SDK system message has permissionMode: plan", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		const sdkCb = listenCallbacks.get("agent-sdk-message");
		expect(sdkCb).toBeDefined();

		// Simulate SDK system init with permissionMode: plan
		act(() => {
			sdkCb?.({
				payload: {
					type: "system",
					subtype: "init",
					session_id: "sdk-session-abc",
					permissionMode: "plan",
				},
			});
		});

		expect(result.current.permissionMode).toBe("plan");
	});

	it("restores user choice when SDK returns permissionMode: default", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// User selects acceptEdits (initial default)
		expect(result.current.permissionMode).toBe("acceptEdits");

		const sdkCb = listenCallbacks.get("agent-sdk-message");
		expect(sdkCb).toBeDefined();

		// SDK sends plan mode
		act(() => {
			sdkCb?.({
				payload: {
					type: "system",
					subtype: "init",
					session_id: "sdk-session-abc",
					permissionMode: "plan",
				},
			});
		});
		expect(result.current.permissionMode).toBe("plan");

		// SDK sends default (ExitPlanMode approved) → should restore user's acceptEdits
		act(() => {
			sdkCb?.({
				payload: {
					type: "system",
					subtype: "status",
					permissionMode: "default",
				},
			});
		});
		expect(result.current.permissionMode).toBe("acceptEdits");
	});
});

describe("useSessionStore", () => {
	it("should export all session operations", async () => {
		const mod = await import("./useSessionStore");
		expect(mod.listSessions).toBeDefined();
		expect(mod.getSession).toBeDefined();
		expect(mod.createSession).toBeDefined();
		expect(mod.addMessage).toBeDefined();
		expect(mod.updateSessionState).toBeDefined();
		expect(mod.updateMessageContent).toBeDefined();
		expect(mod.updateSessionAgentInfo).toBeDefined();
	});
});
