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
	addMessage: vi.fn(),
	updateSessionState: vi.fn().mockResolvedValue(undefined),
	updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
	closeSession: vi.fn().mockResolvedValue(undefined),
	restoreSession: vi.fn().mockResolvedValue(undefined),
	listClosedSessions: vi.fn().mockResolvedValue([]),
}));

let addMessageCounter = 0;

describe("useAgentChat", () => {
	beforeEach(async () => {
		mockInvoke.mockClear();
		addMessageCounter = 0;
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.addMessage).mockImplementation(
			(_sid, role, content) => {
				addMessageCounter++;
				return Promise.resolve({
					id: `msg-${addMessageCounter}`,
					role,
					parts: [{ type: "text", content }],
					timestamp: 1000 + addMessageCounter,
				});
			},
		);
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
				chatSessionId: "s1",
				permissionMode: "acceptEdits",
			}),
		);
	});

	it("respondPermission invokes respond_agent_permission with chatSessionId", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission("req-001", true);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			chatSessionId: "s1",
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

		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission("req-002", false);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			chatSessionId: "s1",
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

	it("interrupt invokes interrupt_agent_query with chatSessionId", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.interrupt();
		});

		expect(mockInvoke).toHaveBeenCalledWith("interrupt_agent_query", {
			chatSessionId: "s1",
		});
	});

	it("selectSession calls getSession and updates activeSession", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const mockSession = {
			id: "s2",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "hi" }],
					timestamp: 1000,
				},
			],
			state: "idle",
			createdAt: 1000,
			updatedAt: 1000,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			mockSession as never,
		);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(sessionStore.getSession).toHaveBeenCalledWith("s2");
		expect(result.current.activeSession).toEqual(mockSession);
	});

	it("selectSession merges streaming buffer into session messages", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Send a message: creates human msg (msg-1) + agent msg (msg-2)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Agent message id is "msg-2" (the streaming message)
		const agentMsgId = "msg-2";

		// Simulate streaming data arriving via SDK messages
		const sdkCb = listenCallbacks.get("agent-sdk-message");
		expect(sdkCb).toBeDefined();

		act(() => {
			sdkCb?.({
				payload: {
					type: "stream_event",
					chat_session_id: "s1",
					event: {
						type: "content_block_delta",
						delta: { type: "text_delta", text: "streamed content" },
					},
				},
			});
		});

		// Switch to s2
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			id: "s2",
			worktreePath: "/repo",
			messages: [],
			state: "idle",
			createdAt: 2000,
			updatedAt: 2000,
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		// Switch back to s1 - getSession returns empty parts (simulating Rust store)
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "msg-1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1001,
				},
				{
					id: agentMsgId,
					role: "agent",
					parts: [],
					timestamp: 1002,
				},
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
		} as never);

		await act(async () => {
			await result.current.selectSession("s1");
		});

		// Buffer parts should be merged into the agent message
		const activeSession = result.current.activeSession;
		expect(activeSession?.id).toBe("s1");
		const agentMsg = activeSession?.messages.find((m) => m.id === agentMsgId);
		const textParts = agentMsg?.parts.filter((p) => p.type === "text");
		expect(textParts).toHaveLength(1);
		expect((textParts?.[0] as { content: string }).content).toBe(
			"streamed content",
		);
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
				chatSessionId: "s1",
				permissionMode: "plan",
			}),
		);
	});

	it("respondPermission for ExitPlanMode sends { behavior: allow } without updatedInput", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission("req-exitplan-001", true);
		});

		expect(mockInvoke).toHaveBeenCalledWith("respond_agent_permission", {
			chatSessionId: "s1",
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

		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

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
			chatSessionId: "s1",
			requestId: "req-003",
			behavior: "allow",
			message: null,
			updatedInput: JSON.stringify(updatedInput),
		});
	});

	it("orderedSessions returns sessions in sessionOrder", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Set sessions via refreshSessions
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([
			{
				id: "s1",
				worktreePath: "/repo",
				updatedAt: 1000,
				state: "active",
				firstMessage: "first",
				messageCount: 1,
				createdAt: 1000,
			},
			{
				id: "s2",
				worktreePath: "/repo",
				updatedAt: 900,
				state: "active",
				firstMessage: "second",
				messageCount: 1,
				createdAt: 900,
			},
		] as never);

		await act(async () => {
			await result.current.refreshSessions();
		});

		// Initial order should match insertion order (s1, s2 appended as new)
		expect(result.current.orderedSessions.map((s) => s.id)).toEqual([
			"s1",
			"s2",
		]);

		// Reorder
		act(() => {
			result.current.reorderSessions(["s2", "s1"]);
		});

		expect(result.current.orderedSessions.map((s) => s.id)).toEqual([
			"s2",
			"s1",
		]);
	});

	it("createNewSession calls createSession and sets active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const newSession = {
			id: "new-s",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 2000,
			updatedAt: 2000,
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(sessionStore.createSession).toHaveBeenCalledWith("/repo");
		expect(result.current.activeSession).toEqual(newSession);
		expect(mockInvoke).toHaveBeenCalledWith(
			"start_agent_session",
			expect.objectContaining({
				chatSessionId: "new-s",
				cwd: "/repo",
				permissionMode: "acceptEdits",
			}),
		);
	});

	it("closeSession on non-active session keeps activeSession unchanged", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Send a message to create s1 as active session
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		const activeSession = result.current.activeSession;
		expect(activeSession).not.toBeNull();

		// Set sessions list to include s1 and s2
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([
			{ id: "s1", worktreePath: "/repo", updatedAt: 1000, state: "active" },
			{ id: "s2", worktreePath: "/repo", updatedAt: 900, state: "active" },
		] as never);

		await act(async () => {
			// Refresh to populate sessions state
			await result.current.refreshSessions();
		});

		await act(async () => {
			await result.current.closeSession("s2");
		});

		expect(sessionStore.closeSession).toHaveBeenCalledWith("s2");
		expect(mockInvoke).toHaveBeenCalledWith("close_agent_session", {
			chatSessionId: "s2",
		});
		expect(result.current.activeSession?.id).toBe(activeSession?.id);
	});

	it("closeSession on active session selects adjacent session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Send message to create s1 as active
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Populate sessions with s1 and s2
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([
			{ id: "s1", worktreePath: "/repo", updatedAt: 1000, state: "active" },
			{ id: "s2", worktreePath: "/repo", updatedAt: 900, state: "active" },
		] as never);

		await act(async () => {
			await result.current.refreshSessions();
		});

		// Mock getSession for the adjacent session
		const s2Full = {
			id: "s2",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 900,
			updatedAt: 900,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(s2Full as never);

		await act(async () => {
			await result.current.closeSession("s1");
		});

		expect(sessionStore.closeSession).toHaveBeenCalledWith("s1");
		expect(mockInvoke).toHaveBeenCalledWith("close_agent_session", {
			chatSessionId: "s1",
		});
		expect(result.current.activeSession?.id).toBe("s2");
	});

	it("closeSession sets error on failure", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		vi.mocked(sessionStore.closeSession).mockRejectedValueOnce(
			new Error("close failed"),
		);

		await act(async () => {
			await result.current.closeSession("s1");
		});

		expect(result.current.error).toContain("セッションクローズに失敗");
	});

	it("restoreSession sets activeSession and refreshes lists", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const restoredSession = {
			id: "s-closed",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					parts: [{ type: "text", content: "old msg" }],
					timestamp: 500,
				},
			],
			state: "idle",
			createdAt: 500,
			updatedAt: 500,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			restoredSession as never,
		);

		await act(async () => {
			await result.current.restoreSession("s-closed");
		});

		expect(sessionStore.restoreSession).toHaveBeenCalledWith("s-closed");
		expect(result.current.activeSession?.id).toBe("s-closed");
	});

	it("restoreSession starts agent process for restored session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const restoredSession = {
			id: "s-closed",
			worktreePath: "/repo",
			messages: [],
			state: "idle",
			createdAt: 500,
			updatedAt: 500,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			restoredSession as never,
		);
		mockInvoke.mockClear();

		await act(async () => {
			await result.current.restoreSession("s-closed");
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"start_agent_session",
			expect.objectContaining({
				chatSessionId: "s-closed",
				cwd: "/repo",
				permissionMode: "acceptEdits",
			}),
		);
	});

	it("restoreSession sets error on failure", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect (auto-creates session when no sessions exist)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		vi.mocked(sessionStore.restoreSession).mockRejectedValueOnce(
			new Error("restore failed"),
		);

		await act(async () => {
			await result.current.restoreSession("s-closed");
		});

		expect(result.current.error).toContain("セッション復元に失敗");
	});

	it("initSessions starts agent process for each existing session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		// listSessions returns multiple sessions
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([
			{
				id: "s1",
				worktreePath: "/repo",
				updatedAt: 1000,
				state: "active",
				firstMessage: "hi",
				messageCount: 1,
				createdAt: 1000,
			},
			{
				id: "s2",
				worktreePath: "/repo",
				updatedAt: 900,
				state: "active",
				firstMessage: "hello",
				messageCount: 1,
				createdAt: 900,
			},
		] as never);

		// getSession for the first session (selectSession(sessions[0].id))
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
		} as never);

		mockInvoke.mockClear();

		renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect to complete
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const startCalls = mockInvoke.mock.calls.filter(
			(call: unknown[]) => call[0] === "start_agent_session",
		);
		expect(startCalls).toHaveLength(2);
		expect(startCalls[0][1]).toEqual(
			expect.objectContaining({ chatSessionId: "s1" }),
		);
		expect(startCalls[1][1]).toEqual(
			expect.objectContaining({ chatSessionId: "s2" }),
		);
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
		expect(mod.updateSessionAgentInfo).toBeDefined();
		expect(mod.closeSession).toBeDefined();
		expect(mod.restoreSession).toBeDefined();
		expect(mod.listClosedSessions).toBeDefined();
	});
});
