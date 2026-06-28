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
	getSessionPage: vi.fn().mockResolvedValue(null),
	planAgentChatEviction: vi.fn().mockResolvedValue({
		active: null,
		evictSessionIds: [],
	}),
	createSession: vi.fn().mockResolvedValue({
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit",
	}),
	addMessage: vi.fn(),
	updateSessionAgentInfo: vi.fn().mockResolvedValue(undefined),
	closeSession: vi.fn().mockResolvedValue(undefined),
	archiveSession: vi.fn().mockResolvedValue(undefined),
	archiveOpenSession: vi.fn().mockResolvedValue(undefined),
	forkSession: vi.fn().mockResolvedValue({
		id: "s-forked",
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
		createdAt: 2000,
		updatedAt: 2000,
		permissionMode: "edit",
	}),
	setSessionTitle: vi.fn().mockResolvedValue({
		id: "s1",
		worktreePath: "/repo",
		state: "idle",
		createdAt: 1000,
		updatedAt: 1000,
		firstMessage: "Custom title",
		messageCount: 1,
		permissionMode: "edit",
	}),
	restoreSession: vi.fn().mockResolvedValue({ restoredWorkflowStep: false }),
	listClosedSessions: vi.fn().mockResolvedValue([]),
	listAgentBackends: vi.fn().mockResolvedValue({
		backends: [],
		defaultId: null,
	}),
	setSessionBackend: vi.fn().mockResolvedValue({
		session: {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit",
			backendId: "codex",
		},
		turnPhase: "idle",
		selectedModel: null,
		availableModels: [],
	}),
	sendAgentMessage: vi.fn().mockResolvedValue({
		session: {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit",
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
	sendWorkflowApprovalChatMessage: vi.fn().mockResolvedValue({
		session: {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit",
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
				permissionMode: "edit",
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
		vi.mocked(sessionStore.getSession).mockResolvedValue(null);
		vi.mocked(sessionStore.getSession).mockClear();
		vi.mocked(sessionStore.getSessionPage).mockResolvedValue(null);
		vi.mocked(sessionStore.getSessionPage).mockClear();
		vi.mocked(sessionStore.planAgentChatEviction).mockResolvedValue({
			active: null,
			evictSessionIds: [],
		});
		vi.mocked(sessionStore.planAgentChatEviction).mockClear();
		vi.mocked(sessionStore.sendAgentMessage).mockClear();
		vi.mocked(sessionStore.sendWorkflowApprovalChatMessage).mockClear();
		vi.mocked(sessionStore.initAgentSessions).mockClear();
		vi.mocked(sessionStore.restoreSession).mockResolvedValue({
			restoredWorkflowStep: false,
		});
		vi.mocked(sessionStore.restoreSession).mockClear();
		vi.mocked(sessionStore.setSessionBackend).mockClear();
	});

	const chatMessage = (id: string, content: string, timestamp: number) => ({
		id,
		role: "human",
		parts: [{ type: "text", content }],
		timestamp,
	});

	const chatSession = (
		id: string,
		messages: ReturnType<typeof chatMessage>[],
	) => ({
		id,
		worktreePath: "/repo",
		messages,
		state: "idle",
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit",
	});

	const sessionResponse = (
		session: ReturnType<typeof chatSession>,
		initialPage: {
			nextCursor: string | null;
			hasMore: boolean;
			totalCount: number;
		},
	) =>
		({
			session,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
			initialPage,
		}) as never;

	const pageResponse = (
		messages: ReturnType<typeof chatMessage>[],
		nextCursor: string | null,
		hasMore: boolean,
	) =>
		({
			messages,
			messageMetadata: messages.map((message) => ({
				messageId: message.id,
				tokenMeta: null,
				runMeta: null,
			})),
			nextCursor,
			hasMore,
			totalCount: messages.length,
			latestTokenUsage: null,
		}) as never;

	const messageRange = (start: number, end: number) =>
		Array.from({ length: end - start + 1 }, (_, index) => {
			const value = start + index;
			return chatMessage(`m${value}`, `message ${value}`, 1000 + value);
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("sendMessage passes images to sendAgentMessage", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		const images = [{ data: "aGVsbG8=", mediaType: "image/png" }];
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"check this",
				images,
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"check this",
			"edit",
			false,
			null,
			images,
			undefined,
		);
	});

	it("sendMessage passes mentions to sendAgentMessage", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		const mentions = [{ filePath: "src/main.rs", startLine: 10, endLine: 20 }];
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"check @src/main.rs:L10-L20",
				undefined,
				mentions,
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"check @src/main.rs:L10-L20",
			"edit",
			false,
			null,
			undefined,
			mentions,
		);
	});

	it("sendMessage with images only (empty text) calls sendAgentMessage", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		const images = [{ data: "aGVsbG8=", mediaType: "image/png" }];
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"",
				images,
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"",
			"edit",
			false,
			null,
			images,
			undefined,
		);
	});

	it("sendMessage without images does not pass images arg", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("sendMessage refreshes workspace tree after summaries change", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const dispatchSpy = vi.spyOn(window, "dispatchEvent");

		try {
			const { result } = renderHook(() => useAgentChat("/repo"));

			await act(async () => {
				await result.current.sendMessage(
					result.current.activeSession?.id ?? null,
					"hello",
				);
			});

			const refreshEvent = dispatchSpy.mock.calls
				.map(([event]) => event)
				.find(
					(event): event is CustomEvent<{ worktreePath: string }> =>
						event.type === "workspace-tree-refresh",
				);
			expect(refreshEvent?.detail).toEqual({ worktreePath: "/repo" });
		} finally {
			dispatchSpy.mockRestore();
		}
	});

	it("sendMessage passes selected backend when Rust creates the session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			result.current.setBackend(
				result.current.activeSession?.id ?? null,
				"codex",
			);
		});

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"edit",
			false,
			"codex",
			undefined,
			undefined,
		);
	});

	it("sendMessage uses workflow approval chat command for the active approval session", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		// Spec issues-1011 line 121: approval chat 経由は run_id 主語で送信する。
		const { result } = renderHook(() =>
			useAgentChat("/repo", "s1", "run-approval-1"),
		);

		await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"adjust policy",
			);
		});

		expect(sessionStore.sendWorkflowApprovalChatMessage).toHaveBeenCalledWith(
			"run-approval-1",
			"adjust policy",
			"edit",
			false,
			undefined,
			undefined,
		);
		expect(sessionStore.sendAgentMessage).not.toHaveBeenCalled();
	});

	it("sendMessage lets Rust route workflow step sessions from the generic entrypoint", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [
				{
					id: "s1",
					worktreePath: "/repo",
					state: "active",
					createdAt: 1,
					updatedAt: 1,
					firstMessage: "",
					messageCount: 0,
					permissionMode: "edit",
					workflowStepSession: true,
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
					permissionMode: "edit",
				},
				turnPhase: "idle",
				selectedModel: "claude-opus-4-8",
				availableModels: [],
			},
		} as never);
		const { result } = renderHook(() => useAgentChat("/repo"));

		await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"continue",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"continue",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("does not treat workflow parent chat session as a workflow step target", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"continue parent",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"continue parent",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("respondPermission invokes respond_agent_permission with chatSessionId", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission(
				result.current.activeSession?.id ?? "",
				"req-001",
				true,
			);
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission(
				result.current.activeSession?.id ?? "",
				"req-002",
				false,
			);
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		// sendAgentMessage is called with null chatSessionId (Rust creates session)
		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("sendMessage appends returned messages when response session is a shell", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage(null, "hello");
		});

		expect(
			result.current.activeSession?.messages.map((message) => message.id),
		).toEqual(["msg-1", "msg-2"]);
	});

	it("sendMessage can create a new session without activating it", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.sendAgentMessage).mockResolvedValueOnce({
			session: {
				id: "side",
				worktreePath: "/repo",
				messages: [],
				state: "active",
				createdAt: 1001,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			humanMessage: {
				id: "msg-side-human",
				role: "human",
				parts: [{ type: "text", content: "side prompt" }],
				timestamp: 1001,
			},
			agentMessage: {
				id: "msg-side-agent",
				role: "agent",
				parts: [],
				timestamp: 1002,
			},
			sessions: [
				{
					id: "s1",
					worktreePath: "/repo",
					createdAt: 1000,
					updatedAt: 1000,
					firstMessage: "",
					messageCount: 0,
					state: "active",
					permissionMode: "edit",
				},
				{
					id: "side",
					worktreePath: "/repo",
					createdAt: 1001,
					updatedAt: 1001,
					firstMessage: "side prompt",
					messageCount: 1,
					state: "active",
					permissionMode: "edit",
				},
			],
			pendingQueue: [],
			pendingQueueCount: 0,
			queuedTurn: null,
		});

		const { result } = renderHook(() => useAgentChat("/repo"));

		await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));

		await act(async () => {
			await result.current.sendMessage(
				null,
				"side prompt",
				undefined,
				undefined,
				{ activateNewSession: false },
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"side prompt",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
		expect(result.current.activeSession?.id).toBe("s1");
		expect(result.current.getSessionById("side")?.id).toBe("side");
	});

	it("sendMessage passes existing session id on second message", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"first",
			);
		});

		vi.mocked(sessionStore.sendAgentMessage).mockClear();

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"second",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"second",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("interrupt invokes interrupt_agent_query with chatSessionId", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.interrupt(result.current.activeSession?.id ?? "");
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
			permissionMode: "edit",
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

	it("loadOlderMessages prepends cursor pages without duplicates and advances cursor", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		const session = chatSession("s1", [chatMessage("m3", "latest", 1003)]);
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(session, {
				nextCursor: "3",
				hasMore: true,
				totalCount: 4,
			}),
		);
		vi.mocked(sessionStore.getSessionPage).mockImplementation(
			async (_sessionId, cursor) => {
				if (cursor === "3") {
					return pageResponse(
						[
							chatMessage("m1", "oldest loaded", 1001),
							chatMessage("m2", "older loaded", 1002),
							chatMessage("m3", "duplicate latest", 1003),
						],
						"1",
						true,
					);
				}
				if (cursor === "1") {
					return pageResponse(
						[
							chatMessage("m0", "very old", 1000),
							chatMessage("m1", "duplicate old", 1001),
						],
						null,
						false,
					);
				}
				throw new Error(`unexpected cursor ${cursor}`);
			},
		);

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});
		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});

		expect(sessionStore.getSessionPage).toHaveBeenNthCalledWith(1, "s1", "3");
		expect(sessionStore.getSessionPage).toHaveBeenNthCalledWith(2, "s1", "1");
		expect(
			result.current.activeSession?.messages.map((message) => message.id),
		).toEqual(["m0", "m1", "m2", "m3"]);
	});

	it("loadOlderMessages is no-op when there is no older cursor", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", [chatMessage("m1", "latest", 1001)]), {
				nextCursor: "1",
				hasMore: false,
				totalCount: 1,
			}),
		);
		const { result, unmount } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});
		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});
		expect(sessionStore.getSessionPage).not.toHaveBeenCalled();
		unmount();

		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s2", [chatMessage("m2", "latest", 1002)]), {
				nextCursor: null,
				hasMore: true,
				totalCount: 1,
			}),
		);
		const { result: result2 } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result2.current.selectSession("s2");
		});
		await act(async () => {
			await result2.current.loadOlderMessages("s2");
		});
		expect(sessionStore.getSessionPage).not.toHaveBeenCalled();
	});

	it("loadOlderMessages suppresses re-entry while a page is loading", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", [chatMessage("m2", "latest", 1002)]), {
				nextCursor: "2",
				hasMore: true,
				totalCount: 2,
			}),
		);
		let resolvePage: (
			value: Awaited<ReturnType<typeof sessionStore.getSessionPage>>,
		) => void = () => {};
		vi.mocked(sessionStore.getSessionPage).mockReturnValue(
			new Promise((resolve) => {
				resolvePage = resolve;
			}),
		);
		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			const first = result.current.loadOlderMessages("s1");
			const second = result.current.loadOlderMessages("s1");
			await second;
			resolvePage(
				pageResponse([chatMessage("m1", "older", 1001)], null, false),
			);
			await first;
		});

		expect(sessionStore.getSessionPage).toHaveBeenCalledTimes(1);
		expect(
			result.current.activeSession?.messages.map((message) => message.id),
		).toEqual(["m1", "m2"]);
	});

	it("loadOlderMessages stops after null page and releases loading", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", [chatMessage("m1", "latest", 1001)]), {
				nextCursor: "1",
				hasMore: true,
				totalCount: 1,
			}),
		);
		vi.mocked(sessionStore.getSessionPage).mockResolvedValueOnce(null);
		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});
		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});

		expect(sessionStore.getSessionPage).toHaveBeenCalledTimes(1);
		expect(
			result.current.activeSession?.messages.map((message) => message.id),
		).toEqual(["m1"]);
	});

	it("loadOlderMessages sets error on reject and can fetch again", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", [chatMessage("m2", "latest", 1002)]), {
				nextCursor: "2",
				hasMore: true,
				totalCount: 2,
			}),
		);
		vi.mocked(sessionStore.getSessionPage)
			.mockRejectedValueOnce(new Error("network down"))
			.mockResolvedValueOnce(
				pageResponse([chatMessage("m1", "older", 1001)], null, false),
			);
		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});
		expect(result.current.error).toContain("過去メッセージの読み込みに失敗");

		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});

		expect(sessionStore.getSessionPage).toHaveBeenCalledTimes(2);
		expect(
			result.current.activeSession?.messages.map((message) => message.id),
		).toEqual(["m1", "m2"]);
	});

	it("evictOlderMessages drops the oldest loaded page and rewinds the cursor for rehydration", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", messageRange(201, 250)), {
				nextCursor: "201",
				hasMore: true,
				totalCount: 250,
			}),
		);
		vi.mocked(sessionStore.getSessionPage).mockImplementation(
			async (_sessionId, cursor) => {
				if (cursor === "201")
					return pageResponse(messageRange(151, 200), "151", true);
				if (cursor === "151")
					return pageResponse(messageRange(101, 150), "101", true);
				if (cursor === "101")
					return pageResponse(messageRange(51, 100), "51", true);
				if (cursor === "51")
					return pageResponse(messageRange(1, 50), null, false);
				throw new Error(`unexpected cursor ${cursor}`);
			},
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			async (request) =>
				request.active
					? {
							active: {
								sessionId: "s1",
								direction: "older",
								count: 50,
								nextCursor: "51",
								hasMore: true,
								loadedPages: [
									{ requestCursor: null, count: 50 },
									{ requestCursor: "201", count: 50 },
									{ requestCursor: "151", count: 50 },
									{ requestCursor: "101", count: 50 },
								],
							},
							evictSessionIds: [],
						}
					: { active: null, evictSessionIds: [] },
		);

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});
		for (let i = 0; i < 4; i++) {
			await act(async () => {
				await result.current.loadOlderMessages("s1");
			});
		}
		expect(result.current.activeSession?.messages).toHaveLength(250);

		await act(async () => {
			await result.current.evictOlderMessages("s1", {
				oldestVisibleIndex: 50,
			});
		});

		expect(sessionStore.planAgentChatEviction).toHaveBeenCalledWith({
			active: expect.objectContaining({
				sessionId: "s1",
				messageCount: 250,
				oldestVisibleIndex: 50,
			}),
		});
		expect(result.current.activeSession?.messages).toHaveLength(200);
		expect(result.current.activeSession?.messages[0]?.id).toBe("m51");

		await act(async () => {
			await result.current.loadOlderMessages("s1");
		});

		expect(sessionStore.getSessionPage).toHaveBeenLastCalledWith("s1", "51");
		expect(result.current.activeSession?.messages).toHaveLength(250);
		expect(result.current.activeSession?.messages[0]?.id).toBe("m1");
		const restoredMessages = result.current.activeSession?.messages ?? [];
		expect(restoredMessages[restoredMessages.length - 1]?.id).toBe("m250");
	});

	it("evictOlderMessages serializes eviction planning per session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", messageRange(201, 250)), {
				nextCursor: "201",
				hasMore: true,
				totalCount: 250,
			}),
		);
		vi.mocked(sessionStore.getSessionPage).mockImplementation(
			async (_sessionId, cursor) => {
				if (cursor === "201")
					return pageResponse(messageRange(151, 200), "151", true);
				if (cursor === "151")
					return pageResponse(messageRange(101, 150), "101", true);
				if (cursor === "101")
					return pageResponse(messageRange(51, 100), "51", true);
				if (cursor === "51")
					return pageResponse(messageRange(1, 50), null, false);
				throw new Error(`unexpected cursor ${cursor}`);
			},
		);

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});
		for (let i = 0; i < 4; i++) {
			await act(async () => {
				await result.current.loadOlderMessages("s1");
			});
		}
		expect(result.current.activeSession?.messages).toHaveLength(250);

		let resolvePlan: (value: never) => void = () => {};
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			(request) =>
				request.active
					? (new Promise((resolve) => {
							resolvePlan = resolve;
						}) as never)
					: Promise.resolve({ active: null, evictSessionIds: [] }),
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockClear();

		let firstEviction = Promise.resolve();
		await act(async () => {
			firstEviction = result.current.evictOlderMessages("s1", {
				oldestVisibleIndex: 50,
			});
			await result.current.evictOlderMessages("s1", {
				oldestVisibleIndex: 50,
			});
		});

		expect(sessionStore.planAgentChatEviction).toHaveBeenCalledTimes(1);

		await act(async () => {
			resolvePlan({
				active: {
					sessionId: "s1",
					direction: "older",
					count: 50,
					nextCursor: "51",
					hasMore: true,
					loadedPages: [
						{ requestCursor: null, count: 50 },
						{ requestCursor: "201", count: 50 },
						{ requestCursor: "151", count: 50 },
						{ requestCursor: "101", count: 50 },
					],
				},
				evictSessionIds: [],
			} as never);
			await firstEviction;
		});

		expect(result.current.activeSession?.messages).toHaveLength(200);
		expect(result.current.activeSession?.messages[0]?.id).toBe("m51");
	});

	it("evictOlderMessages discards a stale plan after page state changes", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", messageRange(201, 250)), {
				nextCursor: "201",
				hasMore: true,
				totalCount: 250,
			}),
		);
		vi.mocked(sessionStore.getSessionPage).mockImplementation(
			async (_sessionId, cursor) => {
				if (cursor === "201")
					return pageResponse(messageRange(151, 200), "151", true);
				if (cursor === "151")
					return pageResponse(messageRange(101, 150), "101", true);
				if (cursor === "101")
					return pageResponse(messageRange(51, 100), "51", true);
				if (cursor === "51")
					return pageResponse(messageRange(1, 50), null, false);
				throw new Error(`unexpected cursor ${cursor}`);
			},
		);

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});
		for (let i = 0; i < 4; i++) {
			await act(async () => {
				await result.current.loadOlderMessages("s1");
			});
		}
		expect(result.current.activeSession?.messages).toHaveLength(250);

		let resolvePlan: (value: never) => void = () => {};
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			(request) =>
				request.active
					? (new Promise((resolve) => {
							resolvePlan = resolve;
						}) as never)
					: Promise.resolve({ active: null, evictSessionIds: [] }),
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockClear();

		let eviction = Promise.resolve();
		await act(async () => {
			eviction = result.current.evictOlderMessages("s1", {
				oldestVisibleIndex: 50,
			});
		});

		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", messageRange(201, 250)), {
				nextCursor: "201",
				hasMore: true,
				totalCount: 250,
			}),
		);
		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			resolvePlan({
				active: {
					sessionId: "s1",
					direction: "older",
					count: 50,
					nextCursor: "51",
					hasMore: true,
					loadedPages: [
						{ requestCursor: null, count: 50 },
						{ requestCursor: "201", count: 50 },
						{ requestCursor: "151", count: 50 },
						{ requestCursor: "101", count: 50 },
					],
				},
				evictSessionIds: [],
			} as never);
			await eviction;
		});

		expect(result.current.activeSession?.messages).toHaveLength(50);
		expect(result.current.activeSession?.messages[0]?.id).toBe("m201");
	});

	it("evictOlderMessages leaves messages unchanged when Rust returns no active plan", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: chatSession("s1", messageRange(201, 250)),
			turnPhase: "streaming",
			selectedModel: null,
			availableModels: [],
			initialPage: {
				nextCursor: "201",
				hasMore: true,
				totalCount: 250,
			},
		} as never);
		vi.mocked(sessionStore.getSessionPage).mockImplementation(
			async (_sessionId, cursor) => {
				if (cursor === "201")
					return pageResponse(messageRange(151, 200), "151", true);
				if (cursor === "151")
					return pageResponse(messageRange(101, 150), "101", true);
				if (cursor === "101")
					return pageResponse(messageRange(51, 100), "51", true);
				if (cursor === "51")
					return pageResponse(messageRange(1, 50), null, false);
				throw new Error(`unexpected cursor ${cursor}`);
			},
		);

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await result.current.selectSession("s1");
		});
		for (let i = 0; i < 4; i++) {
			await act(async () => {
				await result.current.loadOlderMessages("s1");
			});
		}

		await act(async () => {
			await result.current.evictOlderMessages("s1", {
				oldestVisibleIndex: 50,
			});
		});

		expect(sessionStore.planAgentChatEviction).toHaveBeenCalledWith({
			active: expect.objectContaining({
				sessionId: "s1",
				turnPhase: "streaming",
			}),
		});
		expect(result.current.activeSession?.messages).toHaveLength(250);
	});

	it("does not surface eviction planning failures and retries on the next trigger", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
		try {
			vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
				sessionResponse(chatSession("s1", messageRange(201, 250)), {
					nextCursor: "201",
					hasMore: true,
					totalCount: 250,
				}),
			);
			vi.mocked(sessionStore.getSessionPage).mockImplementation(
				async (_sessionId, cursor) => {
					if (cursor === "201")
						return pageResponse(messageRange(151, 200), "151", true);
					if (cursor === "151")
						return pageResponse(messageRange(101, 150), "101", true);
					if (cursor === "101")
						return pageResponse(messageRange(51, 100), "51", true);
					if (cursor === "51")
						return pageResponse(messageRange(1, 50), null, false);
					throw new Error(`unexpected cursor ${cursor}`);
				},
			);
			const { result } = renderHook(() => useAgentChat("/repo"));
			await act(async () => {
				await result.current.selectSession("s1");
			});
			for (let i = 0; i < 4; i++) {
				await act(async () => {
					await result.current.loadOlderMessages("s1");
				});
			}
			vi.mocked(sessionStore.planAgentChatEviction).mockClear();
			vi.mocked(sessionStore.planAgentChatEviction)
				.mockRejectedValueOnce(new Error("planner unavailable"))
				.mockResolvedValueOnce({ active: null, evictSessionIds: [] });

			await act(async () => {
				await result.current.evictOlderMessages("s1", {
					oldestVisibleIndex: 50,
				});
			});

			expect(result.current.error).toBeNull();
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining("メッセージ退避計画の取得に失敗"),
			);

			await act(async () => {
				await result.current.evictOlderMessages("s1", {
					oldestVisibleIndex: 50,
				});
			});

			expect(sessionStore.planAgentChatEviction).toHaveBeenCalledTimes(2);
		} finally {
			warnSpy.mockRestore();
		}
	});

	it("keeps loaded page accounting aligned after new and duplicate SDK messages so eviction can proceed", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce(
			sessionResponse(chatSession("s1", messageRange(1, 201)), {
				nextCursor: null,
				hasMore: false,
				totalCount: 201,
			}),
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			async (request) => {
				const messageCount = request.active?.messageCount ?? 0;
				return request.active
					? {
							active: {
								sessionId: "s1",
								direction: "older",
								count: 50,
								nextCursor: "51",
								hasMore: true,
								loadedPages: [
									{ requestCursor: null, count: messageCount - 50 },
								],
							},
							evictSessionIds: [],
						}
					: { active: null, evictSessionIds: [] };
			},
		);
		const dateSpy = vi.spyOn(Date, "now").mockReturnValue(12345);
		try {
			const { result } = renderHook(() => useAgentChat("/repo"));
			await act(async () => {
				await result.current.selectSession("s1");
			});
			await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));
			const emitSystemMessage = () =>
				listenCallbacks.get("agent-sdk-message")?.({
					payload: {
						type: "system",
						chat_session_id: "s1",
						message: "background notice",
					},
				});

			await act(async () => {
				emitSystemMessage();
			});
			await waitFor(() =>
				expect(result.current.activeSession?.messages).toHaveLength(202),
			);
			await act(async () => {
				emitSystemMessage();
			});
			await waitFor(() =>
				expect(result.current.activeSession?.messages).toHaveLength(202),
			);
			vi.mocked(sessionStore.planAgentChatEviction).mockClear();

			await act(async () => {
				await result.current.evictOlderMessages("s1", {
					oldestVisibleIndex: 50,
				});
			});

			expect(sessionStore.planAgentChatEviction).toHaveBeenCalledWith({
				active: expect.objectContaining({
					sessionId: "s1",
					messageCount: 202,
					loadedPages: [{ requestCursor: null, count: 202 }],
				}),
			});
			expect(result.current.activeSession?.messages).toHaveLength(152);
		} finally {
			dateSpy.mockRestore();
		}
	});

	it("does not replan inactive eviction when only an existing hydrated session message count changes", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: sessionResponse(
				chatSession("s1", [chatMessage("m1", "existing", 1001)]),
				{
					nextCursor: null,
					hasMore: false,
					totalCount: 1,
				},
			),
		} as never);
		const dateSpy = vi.spyOn(Date, "now").mockReturnValue(23456);
		try {
			const { result } = renderHook(() => useAgentChat("/repo"));
			await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));
			vi.mocked(sessionStore.planAgentChatEviction).mockClear();

			await act(async () => {
				listenCallbacks.get("agent-sdk-message")?.({
					payload: {
						type: "system",
						chat_session_id: "s1",
						message: "new visible message",
					},
				});
			});
			await waitFor(() =>
				expect(result.current.activeSession?.messages).toHaveLength(2),
			);
			await act(async () => {
				await Promise.resolve();
			});

			expect(sessionStore.planAgentChatEviction).not.toHaveBeenCalled();
		} finally {
			dateSpy.mockRestore();
		}
	});

	it("replans inactive eviction when a session transitions from empty to hydrated", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		const dateSpy = vi.spyOn(Date, "now").mockReturnValue(34567);
		try {
			const { result } = renderHook(() => useAgentChat("/repo"));
			await waitFor(() => expect(result.current.activeSession?.id).toBe("s1"));
			vi.mocked(sessionStore.planAgentChatEviction).mockClear();

			await act(async () => {
				listenCallbacks.get("agent-sdk-message")?.({
					payload: {
						type: "system",
						chat_session_id: "s1",
						message: "first visible message",
					},
				});
			});

			await waitFor(() => {
				expect(sessionStore.planAgentChatEviction).toHaveBeenCalledWith({
					sessions: expect.arrayContaining([
						expect.objectContaining({
							sessionId: "s1",
							messageCount: 1,
						}),
					]),
				});
			});
		} finally {
			dateSpy.mockRestore();
		}
	});

	it("evicts inactive session bodies above the hydrate cap and rehydrates on selection", async () => {
		const { renderHook, act, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);
		vi.mocked(sessionStore.getSession).mockImplementation(async (sessionId) =>
			sessionResponse(chatSession(String(sessionId), messageRange(1, 50)), {
				nextCursor: null,
				hasMore: false,
				totalCount: 50,
			}),
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			async (request) => {
				const sessions = request.sessions ?? [];
				const hydrated = sessions.filter((session) => session.messageCount > 0);
				const candidates = hydrated
					.filter((session) => !session.protected && !session.loading)
					.sort(
						(left, right) =>
							left.evictionRank - right.evictionRank ||
							left.sessionId.localeCompare(right.sessionId),
					);
				return {
					active: null,
					evictSessionIds:
						hydrated.length > 3 && candidates[0]
							? [candidates[0].sessionId]
							: [],
				};
			},
		);

		const sessionIds = ["s1", "s2", "s3", "s4"];
		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		for (const sessionId of sessionIds) {
			await act(async () => {
				await result.current.selectSession(sessionId);
			});
		}

		await waitFor(() => {
			const hydratedCount = sessionIds.filter(
				(sessionId) =>
					(result.current.getSessionById(sessionId)?.messages.length ?? 0) > 0,
			).length;
			expect(hydratedCount).toBeLessThanOrEqual(3);
		});
		const inactiveRequests = vi
			.mocked(sessionStore.planAgentChatEviction)
			.mock.calls.map(([request]) => request)
			.filter((request) => request.sessions);
		const requestWithAllSessions = [...inactiveRequests]
			.reverse()
			.find((request) =>
				sessionIds.every((sessionId) =>
					request.sessions?.some((session) => session.sessionId === sessionId),
				),
			);
		expect(requestWithAllSessions).toBeDefined();
		const ranks = Object.fromEntries(
			(requestWithAllSessions?.sessions ?? []).map((session) => [
				session.sessionId,
				session.evictionRank,
			]),
		) as Record<string, number>;
		expect(ranks.s1).toBeLessThan(ranks.s2);
		expect(ranks.s2).toBeLessThan(ranks.s3);
		expect(ranks.s3).toBeLessThan(ranks.s4);
		expect(result.current.getSessionById("s1")?.messages).toEqual([]);

		await act(async () => {
			await result.current.selectSession("s1");
		});

		expect(result.current.activeSession?.id).toBe("s1");
		expect(result.current.activeSession?.messages).toHaveLength(50);
	});

	it("does not apply a stale inactive eviction plan to a session that becomes active", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);
		vi.mocked(sessionStore.getSession).mockImplementation(async (sessionId) =>
			sessionResponse(chatSession(String(sessionId), messageRange(1, 50)), {
				nextCursor: null,
				hasMore: false,
				totalCount: 50,
			}),
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockResolvedValue({
			active: null,
			evictSessionIds: [],
		});

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		for (const sessionId of ["s1", "s2", "s3", "s4"]) {
			await act(async () => {
				await result.current.selectSession(sessionId);
			});
		}

		let resolvePlan: (value: never) => void = () => {};
		let pendingIssued = false;
		vi.mocked(sessionStore.planAgentChatEviction).mockImplementation(
			(request) => {
				if (!pendingIssued && request.sessions && request.sessions.length > 0) {
					pendingIssued = true;
					return new Promise((resolve) => {
						resolvePlan = resolve;
					}) as never;
				}
				return Promise.resolve({ active: null, evictSessionIds: [] });
			},
		);
		vi.mocked(sessionStore.planAgentChatEviction).mockClear();

		let cleanup: () => void = () => {};
		await act(async () => {
			cleanup = result.current.registerViewableSession("transient-session");
		});
		await act(async () => {
			cleanup();
		});
		expect(sessionStore.planAgentChatEviction).toHaveBeenCalledTimes(1);

		await act(async () => {
			await result.current.selectSession("s1");
		});

		await act(async () => {
			resolvePlan({
				active: null,
				evictSessionIds: ["s1"],
			} as never);
			await Promise.resolve();
		});

		expect(result.current.activeSession?.id).toBe("s1");
		expect(result.current.activeSession?.messages).toHaveLength(50);
		expect(result.current.getSessionById("s1")?.messages).toHaveLength(50);
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
			availableModels: [{ value: "claude-4" }],
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(result.current.selectedModel).toBe("claude-4");
		expect(result.current.availableModels).toEqual([{ value: "claude-4" }]);
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
			permissionMode: "edit",
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
			result.current.setPermissionMode(
				result.current.activeSession?.id ?? null,
				"ask",
			);
		});

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			null,
			"/repo",
			"hello",
			"ask",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("setPermissionMode immediately invokes set_agent_permission_mode for active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setPermissionMode(
				result.current.activeSession?.id ?? null,
				"full" as never,
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_permission_mode", {
			chatSessionId: "s1",
			permissionMode: "full",
		});
	});

	it("setPlanMode immediately invokes set_agent_plan_mode for active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setPlanMode(
				result.current.activeSession?.id ?? null,
				true,
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_plan_mode", {
			chatSessionId: "s1",
			planMode: true,
		});
	});

	it("sendMessage uses the addressed session permission and plan mode", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		act(() => {
			const cleanupA = result.current.registerViewableSession("session-a");
			const cleanupB = result.current.registerViewableSession("session-b");
			result.current.setPermissionMode("session-a", "ask");
			result.current.setPlanMode("session-a", true);
			result.current.setPermissionMode("session-b", "full");
			result.current.setPlanMode("session-b", false);
			cleanupA();
			cleanupB();
		});

		await act(async () => {
			await result.current.sendMessage("session-a", "hello a");
			await result.current.sendMessage("session-b", "hello b");
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenNthCalledWith(
			1,
			"session-a",
			"/repo",
			"hello a",
			"ask",
			true,
			null,
			undefined,
			undefined,
		);
		expect(sessionStore.sendAgentMessage).toHaveBeenNthCalledWith(
			2,
			"session-b",
			"/repo",
			"hello b",
			"full",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("workflow approval chat uses the addressed session permission and plan mode", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() =>
			useAgentChat("/repo", "approval-session", "run-1"),
		);

		act(() => {
			const cleanup =
				result.current.registerViewableSession("approval-session");
			result.current.setPermissionMode("approval-session", "full");
			result.current.setPlanMode("approval-session", true);
			cleanup();
		});

		await act(async () => {
			await result.current.sendMessage("approval-session", "approve");
		});

		expect(sessionStore.sendWorkflowApprovalChatMessage).toHaveBeenCalledWith(
			"run-1",
			"approve",
			"full",
			true,
			undefined,
			undefined,
		);
	});

	it("setModel invokes set_agent_model with chatSessionId and modelId for active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setModel(
				result.current.activeSession?.id ?? "",
				"claude-4",
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: "claude-4",
		});
	});

	it("setModel always sends a non-null modelId (no unset/null path)", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session first
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.setModel(
				result.current.activeSession?.id ?? "",
				"claude-opus-4-8",
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: "claude-opus-4-8",
		});
		const setModelCalls = mockInvoke.mock.calls.filter(
			(call) => call[0] === "set_agent_model",
		);
		for (const call of setModelCalls) {
			expect((call[1] as { modelId: unknown }).modelId).not.toBeNull();
		}
	});

	it("sendMessage after setModel invokes sendAgentMessage for the active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Create active session via first message
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"first",
			);
		});
		mockInvoke.mockClear();
		vi.mocked(sessionStore.sendAgentMessage).mockClear();

		// Select model
		act(() => {
			result.current.setModel(
				result.current.activeSession?.id ?? "",
				"claude-4",
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "s1",
			modelId: "claude-4",
		});

		// Send second message — model sync is handled by Rust's start_agent_turn
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"second",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"s1",
			"/repo",
			"second",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("respondPermission for ExitPlanMode sends { behavior: allow } without updatedInput", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		act(() => {
			result.current.respondPermission(
				result.current.activeSession?.id ?? "",
				"req-exitplan-001",
				true,
			);
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		mockInvoke.mockClear();

		const updatedInput = {
			questions: [
				{ question: "Pick one", header: "Q", options: [], multiSelect: false },
			],
			answers: { "Pick one": "A" },
		};

		act(() => {
			result.current.respondPermission(
				result.current.activeSession?.id ?? "",
				"req-003",
				true,
				updatedInput,
			);
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
			permissionMode: "edit",
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);

		let createdSessionId: string | null = null;
		await act(async () => {
			createdSessionId = await result.current.createNewSession();
		});

		expect(sessionStore.createSession).toHaveBeenCalledWith(
			"/repo",
			"edit",
			null,
		);
		expect(result.current.activeSession).toEqual(newSession);
		expect(createdSessionId).toBe("new-s");
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_agent_session",
			expect.anything(),
		);
		// R4-02: New session starts with default model (null)
		expect(result.current.selectedModel).toBeNull();
	});

	it("createNewSession loads model metadata for the new empty session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

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
			permissionMode: "edit",
			backendId: "codex",
		};
		const models = [{ value: "sonnet" }];
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: newSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: models,
		} as never);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(sessionStore.getSession).toHaveBeenCalledWith("new-s");
		expect(result.current.availableModels).toEqual(models);
		expect(result.current.selectedModel).toBeNull();
	});

	it("createNewSession allows selecting a model before the backend process starts", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

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
			permissionMode: "edit",
			backendId: "claude",
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: newSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [{ value: "sonnet" }],
		} as never);

		await act(async () => {
			await result.current.createNewSession();
		});
		mockInvoke.mockClear();

		await act(async () => {
			result.current.setModel(result.current.activeSession?.id ?? "", "sonnet");
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
			chatSessionId: "new-s",
			modelId: "sonnet",
		});
		expect(result.current.selectedModel).toBe("sonnet");
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_agent_session",
			expect.anything(),
		);
	});

	it("sendMessage after selecting a model in NewSession uses the existing session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

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
			permissionMode: "edit",
			backendId: "claude",
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: newSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [{ value: "sonnet" }],
		} as never);

		await act(async () => {
			await result.current.createNewSession();
		});
		act(() => {
			result.current.setModel(result.current.activeSession?.id ?? "", "sonnet");
		});
		vi.mocked(sessionStore.sendAgentMessage).mockClear();

		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(sessionStore.sendAgentMessage).toHaveBeenCalledWith(
			"new-s",
			"/repo",
			"hello",
			"edit",
			false,
			null,
			undefined,
			undefined,
		);
	});

	it("createNewSession passes selectedBackendId to createSession", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: null,
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Wait for mount effect
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		// Select a backend
		act(() => {
			result.current.setBackend(
				result.current.activeSession?.id ?? null,
				"claude",
			);
		});

		const newSession = {
			id: "new-s",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 2000,
			updatedAt: 2000,
			permissionMode: "edit",
			backendId: "claude",
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(sessionStore.createSession).toHaveBeenCalledWith(
			"/repo",
			"edit",
			"claude",
		);
	});

	it("does not change selected backend while an existing session is active", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: {
				session: {
					id: "s-codex",
					worktreePath: "/repo",
					messages: [
						{
							id: "m1",
							role: "human",
							parts: [{ type: "text", content: "hello" }],
							timestamp: 1000,
						},
					],
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
					permissionMode: "edit",
					backendId: "codex",
				},
				turnPhase: "idle",
				selectedModel: null,
				availableModels: [],
			},
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		act(() => {
			result.current.setBackend(
				result.current.activeSession?.id ?? null,
				"claude",
			);
		});

		expect(result.current.selectedBackendId).toBe("codex");
		expect(sessionStore.setSessionBackend).not.toHaveBeenCalled();
	});

	it("updates selected backend for an empty active session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [],
			activeSession: {
				session: {
					id: "empty-claude",
					worktreePath: "/repo",
					messages: [],
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
					permissionMode: "edit",
					backendId: "claude",
				},
				turnPhase: "idle",
				selectedModel: null,
				availableModels: [],
			},
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		vi.mocked(sessionStore.setSessionBackend).mockResolvedValueOnce({
			session: {
				id: "empty-claude",
				worktreePath: "/repo",
				messages: [],
				state: "active",
				createdAt: 1000,
				updatedAt: 1100,
				permissionMode: "edit",
				backendId: "codex",
			},
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [{ value: "codex-mini" }],
		} as never);

		await act(async () => {
			result.current.setBackend(
				result.current.activeSession?.id ?? null,
				"codex",
			);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(sessionStore.setSessionBackend).toHaveBeenCalledWith(
			"empty-claude",
			"codex",
		);
		expect(result.current.selectedBackendId).toBe("codex");
		expect(result.current.availableModels).toEqual([{ value: "codex-mini" }]);

		const newSession = {
			id: "new-claude",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 2000,
			updatedAt: 2000,
			permissionMode: "edit",
			backendId: "claude",
		};
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce(
			newSession as never,
		);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(sessionStore.createSession).toHaveBeenCalledWith(
			"/repo",
			"edit",
			"claude",
		);
	});

	it("closeSession on non-active session keeps activeSession unchanged", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Send a message to create s1 as active session
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
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
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"close_agent_session",
			expect.anything(),
		);
		expect(result.current.activeSession?.id).toBe(activeSession?.id);
	});

	it("closeSession on active session selects adjacent session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		// Send message to create s1 as active
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
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
			permissionMode: "edit",
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
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"close_agent_session",
			expect.anything(),
		);
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

	it("archiveSession archives a closed session and refreshes closed sessions", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		vi.mocked(sessionStore.listClosedSessions).mockResolvedValueOnce([]);

		await act(async () => {
			await result.current.archiveSession("s-closed");
		});

		expect(sessionStore.archiveSession).toHaveBeenCalledWith("s-closed");
		expect(sessionStore.listClosedSessions).toHaveBeenCalledWith("/repo");
	});

	it("archiveOpenSession archives the active session and refreshes lists", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const activeSession = {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "idle",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit",
		};
		vi.mocked(sessionStore.initAgentSessions).mockResolvedValueOnce({
			sessions: [{ id: "s1", worktreePath: "/repo", updatedAt: 1000 }],
			activeSession: {
				session: activeSession,
				turnPhase: "idle",
				selectedModel: null,
				availableModels: [],
			},
		} as never);
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([]);
		vi.mocked(sessionStore.listClosedSessions).mockResolvedValueOnce([]);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		await act(async () => {
			await result.current.archiveOpenSession("s1");
		});

		expect(sessionStore.archiveOpenSession).toHaveBeenCalledWith("s1");
		expect(result.current.activeSession).toBeNull();
		expect(sessionStore.listSessions).toHaveBeenCalledWith("/repo");
		expect(sessionStore.listClosedSessions).toHaveBeenCalledWith("/repo");
	});

	it("forkSession creates and activates a forked session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const forkedSession = {
			id: "s-forked",
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
			createdAt: 2000,
			updatedAt: 2000,
			permissionMode: "edit",
		};
		vi.mocked(sessionStore.forkSession).mockResolvedValueOnce(
			forkedSession as never,
		);
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: forkedSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.forkSession("s1");
		});

		expect(sessionStore.forkSession).toHaveBeenCalledWith("s1");
		expect(result.current.activeSession?.id).toBe("s-forked");
	});

	it("setSessionTitle persists title and refreshes sessions", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		vi.mocked(sessionStore.setSessionTitle).mockResolvedValueOnce({
			id: "s1",
			worktreePath: "/repo",
			state: "idle",
			createdAt: 1000,
			updatedAt: 1000,
			firstMessage: "Custom title",
			messageCount: 1,
			permissionMode: "edit",
		} as never);
		vi.mocked(sessionStore.listSessions).mockResolvedValueOnce([
			{
				id: "s1",
				worktreePath: "/repo",
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				firstMessage: "Custom title",
				messageCount: 1,
				permissionMode: "edit",
			},
		] as never);

		let label = "";
		await act(async () => {
			label = await result.current.setSessionTitle("s1", "Custom title");
		});

		expect(sessionStore.setSessionTitle).toHaveBeenCalledWith(
			"s1",
			"Custom title",
		);
		expect(label).toBe("Custom title");
		expect(sessionStore.listSessions).toHaveBeenCalledWith("/repo");
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
			permissionMode: "edit",
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
			permissionMode: "edit",
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
				permissionMode: "edit",
			}),
		);
	});

	it("restoreSession does not start an agent process when Rust restored a workflow step tab", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const restoredSession = {
			id: "workflow-step-closed",
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
			permissionMode: "edit",
			agentSessionId: "agent-session-1",
		};
		vi.mocked(sessionStore.restoreSession).mockResolvedValueOnce({
			restoredWorkflowStep: true,
		});
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: restoredSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);
		mockInvoke.mockClear();

		await act(async () => {
			await result.current.restoreSession("workflow-step-closed");
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_agent_session",
			expect.anything(),
		);
		expect(result.current.activeSession?.id).toBe("workflow-step-closed");
	});

	it("restoreSession passes workflow step execution context from closed summaries", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		vi.mocked(sessionStore.listClosedSessions).mockResolvedValueOnce([
			{
				id: "workflow-step-closed",
				worktreePath: "/repo",
				state: "closed",
				createdAt: 1,
				updatedAt: 1,
				firstMessage: "",
				messageCount: 1,
				permissionMode: "edit",
				workflowStepSession: true,
			},
		] as never);
		await act(async () => {
			await result.current.refreshClosedSessions();
		});

		vi.mocked(sessionStore.restoreSession).mockResolvedValueOnce({
			restoredWorkflowStep: true,
		});
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "workflow-step-closed",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 500,
				updatedAt: 500,
				permissionMode: "edit",
				agentSessionId: "agent-session-1",
			},
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);

		await act(async () => {
			await result.current.restoreSession("workflow-step-closed");
		});

		expect(sessionStore.restoreSession).toHaveBeenCalledWith(
			"workflow-step-closed",
		);
	});

	it("restoreSession does not start agent process for an unstarted session", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		const restoredSession = {
			id: "s-empty",
			worktreePath: "/repo",
			messages: [],
			state: "idle",
			createdAt: 500,
			updatedAt: 500,
			permissionMode: "edit",
		};
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: restoredSession,
			turnPhase: "idle",
			selectedModel: null,
			availableModels: [],
		} as never);
		mockInvoke.mockClear();

		await act(async () => {
			await result.current.restoreSession("s-empty");
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_agent_session",
			expect.anything(),
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
					permissionMode: "edit",
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

		vi.mocked(sessionStore.listAgentBackends).mockResolvedValueOnce({
			backends: [
				{
					id: "claude",
					name: "Claude",
					available: true,
					availableModels: [{ value: "claude-4" }],
				},
			],
			defaultId: "claude",
		});
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
				availableModels: [{ value: "claude-4" }],
			},
		} as never);

		const { result } = renderHook(() => useAgentChat("/repo"));

		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(result.current.selectedModel).toBe("claude-4");
		expect(result.current.availableModels).toEqual([{ value: "claude-4" }]);
	});

	it("restores permissionMode and planMode from Rust response when worktree changes", async () => {
		const { renderHook, waitFor } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");
		const sessionStore = await import("./useSessionStore");

		vi.mocked(sessionStore.initAgentSessions)
			.mockResolvedValueOnce({
				sessions: [
					{
						id: "s1",
						worktreePath: "/repo-a",
						updatedAt: 1000,
						state: "active",
						firstMessage: "hi",
						messageCount: 1,
						createdAt: 1000,
						permissionMode: "ask",
						planMode: true,
					},
				],
				activeSession: {
					session: {
						id: "s1",
						worktreePath: "/repo-a",
						messages: [],
						state: "active",
						createdAt: 1000,
						updatedAt: 1000,
						permissionMode: "ask",
						planMode: true,
					},
					turnPhase: "idle",
					selectedModel: null,
					availableModels: [],
				},
				permissionMode: "ask",
				planMode: true,
			} as never)
			.mockResolvedValueOnce({
				sessions: [],
				activeSession: null,
				permissionMode: "edit",
				planMode: false,
			} as never);

		const { result, rerender } = renderHook(
			({ worktreePath }) => useAgentChat(worktreePath),
			{ initialProps: { worktreePath: "/repo-a" } },
		);

		await waitFor(() => {
			expect(result.current.permissionMode).toBe("ask");
			expect(result.current.planMode).toBe(true);
		});

		rerender({ worktreePath: "/repo-b" });

		await waitFor(() => {
			expect(result.current.permissionMode).toBe("edit");
			expect(result.current.planMode).toBe(false);
		});
		expect(sessionStore.initAgentSessions).toHaveBeenCalledWith("/repo-b");
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

		// Mock getSession returning a session with "ask" permissionMode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "s2",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "ask",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("s2");
		});

		expect(result.current.permissionMode).toBe("ask");
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

		// Switch to Session A with "ask" mode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-a",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "ask",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-a");
		});

		expect(result.current.permissionMode).toBe("ask");

		// Switch to Session B with "full" mode
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-b",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "full",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-b");
		});

		expect(result.current.permissionMode).toBe("full");

		// Switch back to Session A — mode should still be "ask"
		vi.mocked(sessionStore.getSession).mockResolvedValueOnce({
			session: {
				id: "session-a",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "ask",
			},
			turnPhase: "idle",
		} as never);

		await act(async () => {
			await result.current.selectSession("session-a");
		});

		expect(result.current.permissionMode).toBe("ask");
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

		// Change mode to ask via event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		// First send a message to create session
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});
		expect(result.current.permissionMode).toBe("ask");

		// Create new session (returns default "edit")
		vi.mocked(sessionStore.createSession).mockResolvedValueOnce({
			id: "new-s",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 2000,
			updatedAt: 2000,
			permissionMode: "edit",
		} as never);

		await act(async () => {
			await result.current.createNewSession();
		});

		expect(result.current.permissionMode).toBe("edit");
	});

	it("permissionMode is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		// Set mode to "plan" via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});
		expect(result.current.permissionMode).toBe("ask");

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

		// permissionMode should still be "ask" after turn completion
		expect(result.current.permissionMode).toBe("ask");
	});

	it("permissionMode full is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		// Set mode to "full" via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "full",
				},
			});
		});
		expect(result.current.permissionMode).toBe("full");

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

		// permissionMode should still be "full" after turn completion
		expect(result.current.permissionMode).toBe("full");
	});

	it("permissionMode ask is preserved after turn completion", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		// Set mode to "ask" via Rust event
		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});
		expect(result.current.permissionMode).toBe("ask");

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

		// permissionMode should still be "ask" after turn completion
		expect(result.current.permissionMode).toBe("ask");
	});
});

describe("permissionMode sync from agent-permission-mode-changed event", () => {
	it("syncs permissionMode when agent-permission-mode-changed event fires with ask", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends permission mode change for the active session
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});

		expect(result.current.permissionMode).toBe("ask");
	});

	it("restores resolved mode when Rust sends agent-permission-mode-changed after ExitPlanMode", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		expect(result.current.permissionMode).toBe("edit");

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends ask mode
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});
		expect(result.current.permissionMode).toBe("ask");

		// Rust resolves and sends the restored edit mode
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "edit",
				},
			});
		});
		expect(result.current.permissionMode).toBe("edit");
	});

	it("restores full when Rust sends agent-permission-mode-changed after ask override", async () => {
		const { renderHook, act } = await import("@testing-library/react");
		const { useAgentChat } = await import("./useAgentChat");

		const { result } = renderHook(() => useAgentChat("/repo"));
		await act(async () => {});

		// Send a message to create an active session (s1)
		await act(async () => {
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
		});

		// User selects full
		act(() => {
			result.current.setPermissionMode(
				result.current.activeSession?.id ?? null,
				"full",
			);
		});
		expect(result.current.permissionMode).toBe("full");

		const permCb = listenCallbacks.get("agent-permission-mode-changed");
		expect(permCb).toBeDefined();

		// Rust sends ask mode
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "ask",
				},
			});
		});
		expect(result.current.permissionMode).toBe("ask");

		// Rust resolves and sends full back
		act(() => {
			permCb?.({
				payload: {
					chat_session_id: "s1",
					permission_mode: "full",
				},
			});
		});
		expect(result.current.permissionMode).toBe("full");
	});
});

describe("useSessionStore", () => {
	it("should export all session operations", async () => {
		const mod = await import("./useSessionStore");
		expect(mod.listSessions).toBeDefined();
		expect(mod.getSession).toBeDefined();
		expect(mod.createSession).toBeDefined();
		expect(mod.addMessage).toBeDefined();
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
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
					permissionMode: "edit",
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
			await result.current.sendMessage(
				result.current.activeSession?.id ?? null,
				"hello",
			);
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
					permissionMode: "edit",
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
				permissionMode: "edit",
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
				permissionMode: "edit",
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
				permissionMode: "edit",
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
