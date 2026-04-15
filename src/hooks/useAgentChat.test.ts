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
		permissionMode: "acceptEdits",
	}),
	addMessage: vi.fn(),
	updateSessionState: vi.fn().mockResolvedValue(undefined),
	updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
	closeSession: vi.fn().mockResolvedValue(undefined),
	restoreSession: vi.fn().mockResolvedValue(undefined),
	listClosedSessions: vi.fn().mockResolvedValue([]),
	sendAgentMessage: vi.fn().mockResolvedValue({
		session: {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "acceptEdits",
		},
		humanMessage: {
			id: "msg-1",
			role: "human",
			parts: [{ type: "text", content: "hello" }],
			timestamp: 1001,
		},
		agentMessage: {
			id: "msg-2",
			role: "agent",
			parts: [],
			timestamp: 1002,
		},
		sessions: [],
	}),
	initAgentSessions: vi.fn().mockResolvedValue({
		sessions: [],
		activeSession: {
			session: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "acceptEdits",
			},
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		},
	}),
}));

describe("useAgentChat", () => {
	beforeEach(async () => {
		mockInvoke.mockClear();
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.sendAgentMessage).mockClear();
		vi.mocked(sessionStore.initAgentSessions).mockClear();
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

	it("sendMessage calls sendAgentMessage with permissionMode", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"acceptEdits",
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

	it("sendMessage creates session via Rust when no active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// sendAgentMessage is called with null chatSessionId (Rust creates session)
		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"acceptEdits",
		);
	});

	it("sendMessage passes existing session id on second message", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("first");
		});

		vi.mocked(sessionStore.sendAgentMessage).mockClear();

		await act(async () => {
			await result.current.sendMessage("second");
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"second",
			"acceptEdits",
		);
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

		// Wait for mount effect
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
			permissionMode: "acceptEdits",
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: mockSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(sessionStore.getSession).toHaveBeenCalledWith("s2");
		expect(result.current.activeSession).toEqual(mockSession);
	});

	it("selectSession restores model selection from backend response", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const mockSession = {
			id: "s2",
			worktreePath: "/repo",
			messages: [],
			state: "idle",
			createdAt: 1000,
			updatedAt: 1000,
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: mockSession,
			turnPhase: "idle",
			selectedModel: "claude-4",
			availableModels: [{ value: "claude-4", displayName: "Claude 4" }],
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(result.current.selectedModel).toBe("claude-4");
		expect(result.current.availableModels).toEqual([
			{ value: "claude-4", displayName: "Claude 4" },
		]);
	});

	it("selectSession restores streaming state from backend response", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const mockSession = {
			id: "s2",
			worktreePath: "/repo",
			messages: [
				{
					id: "msg-1",
					role: "human",
					parts: [{ type: "text", content: "hello" }],
					timestamp: 1001,
				},
				{
					id: "msg-2",
					role: "agent",
					parts: [{ type: "text", content: "streamed content" }],
					timestamp: 1002,
				},
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "acceptEdits",
		};

		// getSession returns merged data from Rust backend (including streaming parts)
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: mockSession,
			turnPhase: "streaming",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(sessionStore.getSession).toHaveBeenCalledWith("s2");
		const activeSession = result.current.activeSession;
		expect(activeSession?.id).toBe("s2");
		const agentMsg = activeSession?.messages.find((m) => m.id === "msg-2");
		const textParts = agentMsg?.parts.filter((p) => p.type === "text");
		expect(textParts).toHaveLength(1);
		expect((textParts?.[0] as { content: string }).content).toBe(
			"streamed content",
		);
		// isStreaming should be restored
		expect(result.current.isStreaming).toBe(true);
	});

	it("setPermissionMode changes mode used in next sendMessage", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.setPermissionMode("plan" as never);
		});

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"plan",
		);
	});

	it("setPermissionMode immediately invokes set_agent_permission_mode for active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setPermissionMode("bypassPermissions" as never);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_permission_mode", {
			chatSessionId: "s1",
			permissionMode: "bypassPermissions",
		});
	});

	it("setModel invokes set_agent_model with chatSessionId and modelId for active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setModel("claude-4");
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: "claude-4",
		});
	});

	it("setModel invokes set_agent_model with null modelId for Auto selection", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setModel(null);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: null,
		});
	});

	it("sendMessage after setModel invokes sendAgentMessage for the active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session via first message
		await act(async () => {
			await result.current.sendMessage("first");
		});
		mockInvoke.mockClear();
		vi.mocked(sessionStore.sendAgentMessage).mockClear();

		// Select model
		act(() => {
			result.current.setModel("claude-4");
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: "claude-4",
		});

		// Send second message — model sync is handled by Rust's start_agent_turn
		await act(async () => {
			await result.current.sendMessage("second");
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"second",
			"acceptEdits",
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

		// Wait for mount effect
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

		// Wait for mount effect
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
			permissionMode: "acceptEdits",
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
		// R4-02: New session starts with default model (null)
		expect(result.current.selectedModel).toBeNull();
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

		// Mock getSession for the adjacent session (returns GetSessionResponse)
		const s2Full = {
			id: "s2",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 900,
			updatedAt: 900,
			permissionMode: "acceptEdits",
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: s2Full,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

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

		// Wait for mount effect
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

		// Wait for mount effect
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
			permissionMode: "acceptEdits",
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: restoredSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

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
			permissionMode: "acceptEdits",
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: restoredSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);
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

		// Wait for mount effect
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

	it("initSessions calls initAgentSessions on mount", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [
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
			],
			activeSession: {
				session: {
					id: "s1",
					worktreePath: "/repo",
					messages: [],
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
					permissionMode: "acceptEdits",
				},
				turnPhase: "idle",
				selectedModel: null,
				availableModels: [],
			},
		} as never);

		renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect to complete
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(sessionStore.initAgentSessions).toHaveBeenCalledWith("/repo");
	});

	it("initSessions restores model selection from backend response", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [
				{
					id: "s1",
					worktreePath: "/repo",
					updatedAt: 1000,
					state: "active",
					firstMessage: "hi",
					messageCount: 1,
					createdAt: 1000,
				},
			],
			activeSession: {
				session: {
					id: "s1",
					worktreePath: "/repo",
					messages: [],
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
				},
				turnPhase: "idle",
				selectedModel: "claude-4",
				availableModels: [{ value: "claude-4", displayName: "Claude 4" }],
			},
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(result.current.selectedModel).toBe("claude-4");
		expect(result.current.availableModels).toEqual([
			{ value: "claude-4", displayName: "Claude 4" },
		]);
	});
});

describe("permissionMode per-session persistence", () => {
	it("selectSession restores permissionMode from session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Mock getSession returning a session with "plan" permissionMode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "s2",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "plan",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(result.current.permissionMode).toBe("plan");
	});

	it("switching between sessions preserves independent permissionModes", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Switch to Session A with "plan" mode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-a",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "plan",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-a");
		});

		expect(result.current.permissionMode).toBe("plan");

		// Switch to Session B with "bypassPermissions" mode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-b",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "bypassPermissions",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-b");
		});

		expect(result.current.permissionMode).toBe("bypassPermissions");

		// Switch back to Session A — mode should still be "plan"
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-a",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "plan",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-a");
		});

		expect(result.current.permissionMode).toBe("plan");
	});

	it("createNewSession resets permissionMode to default", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Change mode to plan via event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		// First send a message to create session
		await act(async () => {
			await result.current.sendMessage("hello");
		});
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "plan",
				},
			});
		});
		expect(result.current.permissionMode).toBe("plan");

		// Create new session (returns default "acceptEdits")
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce({
			id: "new-s",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 2000,
			updatedAt: 2000,
			permissionMode: "acceptEdits",
		} as never);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(result.current.permissionMode).toBe("acceptEdits");
	});

	it("permissionMode is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Set mode to "plan" via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "plan",
				},
			});
		});
		expect(result.current.permissionMode).toBe("plan");

		// Simulate turn completion via agent-session-state-changed event
		const stateCb = listenCallbacks.get("agent-session-state-changed");
		act(() => {
			stateCb?.({
				payload: {
					chat_session_id: "s1",
					turn_phase: "idle",
					exit_code: 0,
				},
			});
		});

		// permissionMode should still be "plan" after turn completion
		expect(result.current.permissionMode).toBe("plan");
	});

	it("permissionMode bypassPermissions is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Set mode to "bypassPermissions" via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "bypassPermissions",
				},
			});
		});
		expect(result.current.permissionMode).toBe("bypassPermissions");

		// Simulate turn completion via agent-session-state-changed event
		const stateCb = listenCallbacks.get("agent-session-state-changed");
		act(() => {
			stateCb?.({
				payload: {
					chat_session_id: "s1",
					turn_phase: "idle",
					exit_code: 0,
				},
			});
		});

		// permissionMode should still be "bypassPermissions" after turn completion
		expect(result.current.permissionMode).toBe("bypassPermissions");
	});

	it("permissionMode default (Ask) is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Set mode to "default" (Ask) via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "default",
				},
			});
		});
		expect(result.current.permissionMode).toBe("default");

		// Simulate turn completion via agent-session-state-changed event
		const stateCb = listenCallbacks.get("agent-session-state-changed");
		act(() => {
			stateCb?.({
				payload: {
					chat_session_id: "s1",
					turn_phase: "idle",
					exit_code: 0,
				},
			});
		});

		// permissionMode should still be "default" after turn completion
		expect(result.current.permissionMode).toBe("default");
	});
});

describe("permissionMode sync from agent-permission-mode-changed event", () => {
	it("syncs permissionMode when agent-permission-mode-changed event fires with plan", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends permission mode change for the active session
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "plan",
				},
			});
		});

		expect(result.current.permissionMode).toBe("plan");
	});

	it("restores resolved mode when Rust sends agent-permission-mode-changed after ExitPlanMode", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		expect(result.current.permissionMode).toBe("acceptEdits");

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends plan mode
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "plan",
				},
			});
		});
		expect(result.current.permissionMode).toBe("plan");

		// Rust resolves and sends the restored mode (acceptEdits)
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "acceptEdits",
				},
			});
		});
		expect(result.current.permissionMode).toBe("acceptEdits");
	});

	it("restores bypassPermissions when Rust sends agent-permission-mode-changed after plan override", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// User selects bypassPermissions
		act(() => {
			result.current.setPermissionMode("bypassPermissions");
		});
		expect(result.current.permissionMode).toBe("bypassPermissions");

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends plan mode
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "plan",
				},
			});
		});
		expect(result.current.permissionMode).toBe("plan");

		// Rust resolves and sends bypassPermissions back
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "bypassPermissions",
				},
			});
		});
		expect(result.current.permissionMode).toBe("bypassPermissions");
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
		expect(mod.sendAgentMessage).toBeDefined();
		expect(mod.initAgentSessions).toBeDefined();
	});
});

describe("Worktree switch (unmount/remount) streaming persistence via Rust backend", () => {
	beforeEach(async () => {
		mockInvoke.mockClear();
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.sendAgentMessage).mockClear();
		vi.mocked(sessionStore.initAgentSessions).mockClear();
	});

	it("getSession with isStreaming restores streaming state on remount", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		// First mount: create session and send a message
		const { result, unmount } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Unmount (simulates Worktree switch)
		unmount();

		// Remount: Rust backend returns session with streaming parts already merged
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [
				{
					id: "s1",
					worktreePath: "/repo",
					updatedAt: 1000,
					state: "active",
					firstMessage: "hello",
					messageCount: 2,
					createdAt: 1000,
				},
			],
			activeSession: {
				session: {
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
							id: "msg-2",
							role: "agent",
							parts: [{ type: "text", content: "streaming response" }],
							timestamp: 1002,
						},
					],
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
					permissionMode: "acceptEdits",
				},
				turnPhase: "streaming",
				selectedModel: null,
				availableModels: [],
			},
		} as never);

		const { result: result2 } = renderHook(() => useAgentChat("/repo"));

		// Wait for initSessions to complete
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// The agent message should have parts from Rust backend
		const activeSession = result2.current.activeSession;
		expect(activeSession?.id).toBe("s1");
		const agentMsg = activeSession?.messages.find((m) => m.id === "msg-2");
		const agentTextParts = agentMsg?.parts.filter((p) => p.type === "text");
		expect(agentTextParts).toHaveLength(1);
		expect((agentTextParts?.[0] as { content: string }).content).toBe(
			"streaming response",
		);
		// Streaming state should be restored from backend
		expect(result2.current.isStreaming).toBe(true);
	});

	it("completed response persisted by Rust survives unmount", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result, unmount } = renderHook(() => useAgentChat("/repo"));

		// Send a message
		await act(async () => {
			await result.current.sendMessage("hello");
		});

		// Unmount
		unmount();

		// After completion + unmount, initAgentSessions returns Rust-persisted data
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [
				{
					id: "s1",
					worktreePath: "/repo",
					updatedAt: 1000,
					state: "idle",
					firstMessage: "hello",
					messageCount: 2,
					createdAt: 1000,
				},
			],
			activeSession: {
				session: {
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
							id: "msg-2",
							role: "agent",
							parts: [{ type: "text", content: "final response" }],
							timestamp: 1002,
						},
					],
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
					permissionMode: "acceptEdits",
				},
				turnPhase: "idle",
				selectedModel: null,
				availableModels: [],
			},
		} as never);

		const { result: result2 } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const activeSession = result2.current.activeSession;
		expect(activeSession?.id).toBe("s1");
		const agentMsg = activeSession?.messages.find((m) => m.id === "msg-2");
		const textParts = agentMsg?.parts.filter((p) => p.type === "text");
		expect(textParts).toHaveLength(1);
		expect((textParts?.[0] as { content: string }).content).toBe(
			"final response",
		);
		expect(result2.current.isStreaming).toBe(false);
	});

	it("shows completed state when switching back from another session after streaming ends", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Select session s1 with streaming turnPhase (simulates ongoing streaming)
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
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
						id: "msg-2",
						role: "agent",
						parts: [{ type: "text", content: "partial" }],
						timestamp: 1002,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "acceptEdits",
			},
			turnPhase: "streaming",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.selectSession("s1");
		});

		// Streaming state should be restored from backend
		expect(result.current.isStreaming).toBe(true);

		// Switch to another session: getSession for session s2
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "s2",
				worktreePath: "/repo",
				messages: [
					{
						id: "msg-3",
						role: "human",
						parts: [{ type: "text", content: "other" }],
						timestamp: 2000,
					},
				],
				state: "idle",
				createdAt: 2000,
				updatedAt: 2000,
				permissionMode: "acceptEdits",
			},
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		// Switch back to session s1 which has completed streaming
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
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
						id: "msg-2",
						role: "agent",
						parts: [{ type: "text", content: "completed response" }],
						timestamp: 1002,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "acceptEdits",
			},
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.selectSession("s1");
		});

		expect(result.current.activeSession?.id).toBe("s1");
		expect(result.current.isStreaming).toBe(false);
		const agentMsg = result.current.activeSession?.messages.find(
			(m) => m.id === "msg-2",
		);
		expect(agentMsg).toBeDefined();
		expect((agentMsg?.parts[0] as { content: string }).content).toBe(
			"completed response",
		);
	});
});
