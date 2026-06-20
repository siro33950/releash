import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// jsdom does not implement scrollIntoView
Element.prototype.scrollIntoView = vi.fn();

const clipboardWriteText = vi.fn().mockResolvedValue(undefined);
Object.defineProperty(navigator, "clipboard", {
	configurable: true,
	value: {
		writeText: clipboardWriteText,
	},
});

beforeEach(() => {
	clipboardWriteText.mockClear();
});

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

vi.mock("@tanstack/react-virtual", () => ({
	useVirtualizer: ({
		count,
		estimateSize,
		getItemKey,
	}: {
		count: number;
		estimateSize: (index: number) => number;
		getItemKey?: (index: number) => string | number;
	}) => ({
		getVirtualItems: () =>
			Array.from({ length: count }, (_, i) => {
				const size = estimateSize(i);
				return {
					index: i,
					key: getItemKey?.(i) ?? i,
					start: i * size,
					size,
					end: (i + 1) * size,
				};
			}),
		getTotalSize: () => {
			let total = 0;
			for (let i = 0; i < count; i++) {
				total += estimateSize(i);
			}
			return total;
		},
		measureElement: () => {},
		scrollToIndex: () => {},
	}),
}));

const defaultAgentShortcuts = [
	{
		id: "command_menu",
		label: "Command menu",
		shortcut: "Cmd K",
		alternateShortcut: "Cmd Shift P",
		defaultShortcut: "Cmd K",
	},
	{
		id: "new_thread",
		label: "New thread",
		shortcut: "Cmd N",
		defaultShortcut: "Cmd N",
	},
	{
		id: "search_threads",
		label: "Search threads",
		shortcut: "Cmd G",
		defaultShortcut: "Cmd G",
	},
	{
		id: "find_in_thread",
		label: "Find in thread",
		shortcut: "Cmd F",
		defaultShortcut: "Cmd F",
	},
	{
		id: "copy_latest_response",
		label: "Copy latest response",
		shortcut: "Ctrl O",
		defaultShortcut: "Ctrl O",
	},
	{
		id: "toggle_raw_scrollback",
		label: "Toggle raw scrollback",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "previous_thread",
		label: "Previous thread",
		shortcut: "Cmd Shift [",
		defaultShortcut: "Cmd Shift [",
	},
	{
		id: "next_thread",
		label: "Next thread",
		shortcut: "Cmd Shift ]",
		defaultShortcut: "Cmd Shift ]",
	},
];

const mockInvoke = vi.fn(
	(command: string, ..._args: unknown[]): Promise<unknown> => {
		if (command === "get_agent_shortcut_settings") {
			return Promise.resolve(defaultAgentShortcuts);
		}
		if (command === "is_agent_command_enabled") {
			return Promise.resolve(true);
		}
		return Promise.resolve([]);
	},
);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (command: string, args?: unknown) => mockInvoke(command, args),
}));

beforeEach(() => {
	mockInvoke.mockClear();
	mockInvoke.mockImplementation((command: string) => {
		if (command === "get_agent_shortcut_settings") {
			return Promise.resolve(defaultAgentShortcuts);
		}
		if (command === "is_agent_command_enabled") {
			return Promise.resolve(true);
		}
		return Promise.resolve([]);
	});
});

type ListenCallback = (event: { payload: unknown }) => void;
const listenCallbacks = new Map<string, ListenCallback>();
const mockListen = vi.fn((eventName: string, callback: ListenCallback) => {
	listenCallbacks.set(eventName, callback);
	return Promise.resolve(() => {
		listenCallbacks.delete(eventName);
	});
});
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) =>
		mockListen(args[0] as string, args[1] as ListenCallback),
}));

const useAgentChatMock = vi.fn();
// spec issues-1023: AgentChatPanel は useAgentChatContext を経由するため
// Context module を直接 mock する。AgentChatProvider 自体は通さない。
vi.mock("@/contexts/AgentChatContext", () => ({
	useAgentChatContext: () => useAgentChatMock(),
	AgentChatProvider: ({ children }: { children: React.ReactNode }) => children,
}));

const useWorkflowStateMock = vi.fn().mockReturnValue({ workflowState: null });
vi.mock("@/hooks/useWorkflowState", () => ({
	useWorkflowState: (...args: unknown[]) => useWorkflowStateMock(...args),
}));

vi.mock("@/hooks/useWorkflowConfig", () => ({
	useWorkflowConfig: () => ({ workflows: [] }),
}));

// Must import after mocks
const { AgentChatPanel } = await import("./AgentChatPanel");

type DropCallback = (paths: string[]) => void;
const agentDropCallbacks = new Map<string, DropCallback>();
const mockRegisterDropZone = vi.fn(
	(zone: string, _element: HTMLElement | null, onDrop?: DropCallback) => {
		if (_element && onDrop) {
			agentDropCallbacks.set(zone, onDrop);
		} else if (!_element) {
			agentDropCallbacks.delete(zone);
		}
	},
);

function mockUseAgentChat(overrides: Record<string, unknown> = {}) {
	const sessions = (overrides.sessions ?? []) as Array<{ id: string }>;
	const orderedSessions = overrides.orderedSessions ?? sessions;
	const activeSession = (overrides.activeSession ?? null) as {
		id: string;
	} | null;
	const sessionsById = activeSession
		? { [activeSession.id]: activeSession }
		: {};
	// 旧 API の `isStreaming` フラグを BoundSessionChat 内部の派生
	// `getSessionTurnPhase` に橋渡しする。テストが `isStreaming: true` を渡したら、
	// active session の turnPhase を "streaming" として返す mock を立てる。
	const isStreaming = overrides.isStreaming === true;
	const explicitTurnPhase = overrides.getSessionTurnPhase as
		| ((id: string) => string)
		| undefined;
	const getSessionTurnPhase = explicitTurnPhase
		? vi.fn(explicitTurnPhase)
		: vi.fn((id: string) =>
				activeSession && id === activeSession.id && isStreaming
					? "streaming"
					: "idle",
			);
	useAgentChatMock.mockReturnValue({
		sessions,
		orderedSessions,
		closedSessions: [],
		activeSession,
		isStreaming,
		activityStatus: null,
		error: null,
		pendingPermission: null,
		sessionAgentStates: new Map(),
		sendMessage: vi.fn(),
		interrupt: vi.fn(),
		selectSession: vi.fn(),
		refreshSessions: vi.fn(),
		refreshClosedSessions: vi.fn(),
		closeSession: vi.fn(),
		archiveSession: vi.fn(),
		archiveOpenSession: vi.fn(),
		restoreSession: vi.fn(),
		forkSession: vi.fn(),
		setSessionTitle: vi.fn(),
		createNewSession: vi.fn(),
		reorderSessions: vi.fn(),
		setPermissionMode: vi.fn(),
		respondPermission: vi.fn(),
		permissionMode: "edit",
		planMode: false,
		setPlanMode: vi.fn(),
		availableModels: [],
		availableModelsByBackend: {},
		selectedModel: null,
		setModel: vi.fn(),
		backends: [],
		selectedBackendId: null,
		setBackend: vi.fn(),
		loadSession: vi.fn().mockResolvedValue(null),
		getSessionById: vi.fn(
			(id: string | null | undefined) =>
				(id != null && sessionsById[id]) || null,
		),
		registerViewableSession: vi.fn().mockReturnValue(() => {}),
		getSessionTurnPhase,
		getSessionSelectedModel: vi.fn().mockReturnValue(null),
		getSessionPendingQueue: vi.fn().mockReturnValue([]),
		getSessionLatestTokenUsage: vi.fn().mockReturnValue(null),
		getSessionRuntimeSlashCommands: vi.fn().mockReturnValue([]),
		getSessionInterrupting: vi.fn().mockReturnValue(false),
		...overrides,
	});
}

describe("AgentChatPanel", () => {
	it("renders empty state when no active session", () => {
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("agent-chat-panel")).toBeDefined();
	});

	it("has data-tauri-drag-region attribute for window dragging", () => {
		mockUseAgentChat();
		const { container } = render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const dragRegion = container.querySelector("[data-tauri-drag-region]");
		expect(dragRegion).toBeInTheDocument();
	});

	it("shows a context restore warning for failed active sessions", () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				contextCarry: "failed",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(
			screen.getByText(/Conversation context was not restored/),
		).toBeInTheDocument();
	});

	it("opens the Rust-backed command palette with Cmd+K", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "new_thread",
						label: "New thread",
						shortcut: "Cmd N",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		await waitFor(() => {
			fireEvent.keyDown(window, { key: "k", metaKey: true });
			expect(mockInvoke).toHaveBeenCalledWith("present_agent_command_palette", {
				request: {
					hasActiveSession: false,
					sessionCount: 0,
				},
			});
		});
		expect(await screen.findByText("New thread")).toBeInTheDocument();
	});

	it("opens the command palette with the native Cmd+Shift+P alternate shortcut", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "new_thread",
						label: "New thread",
						shortcut: "Cmd N",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		await waitFor(() => {
			fireEvent.keyDown(window, { key: "p", metaKey: true, shiftKey: true });
			expect(mockInvoke).toHaveBeenCalledWith("present_agent_command_palette", {
				request: {
					hasActiveSession: false,
					sessionCount: 0,
				},
			});
		});
		expect(await screen.findByText("New thread")).toBeInTheDocument();
	});

	it("opens current thread search from the command palette Find action", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "find_in_thread",
						label: "Find in thread",
						shortcut: "Cmd F",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "Searchable answer" }],
						timestamp: 1000,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		mockInvoke.mockClear();
		await waitFor(() => {
			fireEvent.keyDown(window, { key: "k", metaKey: true });
			expect(mockInvoke).toHaveBeenCalledWith(
				"present_agent_command_palette",
				expect.any(Object),
			);
		});
		fireEvent.click(await screen.findByText("Find in thread"));

		const input = await screen.findByPlaceholderText("Find in current thread");
		expect(document.activeElement).toBe(input);
	});

	it("creates a new thread with Cmd+N", async () => {
		const createNewSession = vi.fn();
		mockUseAgentChat({ createNewSession });
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		fireEvent.keyDown(window, { key: "n", metaKey: true });

		await waitFor(() => expect(createNewSession).toHaveBeenCalledTimes(1));
	});

	it("uses customized shortcuts from Rust settings", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(
					defaultAgentShortcuts.map((shortcut) =>
						shortcut.id === "new_thread"
							? { ...shortcut, shortcut: "Ctrl Shift N" }
							: shortcut,
					),
				);
			}
			if (command === "is_agent_command_enabled") {
				return Promise.resolve(true);
			}
			return Promise.resolve([]);
		});
		const createNewSession = vi.fn();
		mockUseAgentChat({ createNewSession });
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		fireEvent.keyDown(window, { key: "n", metaKey: true });
		fireEvent.keyDown(window, { key: "n", ctrlKey: true, shiftKey: true });

		await waitFor(() => expect(createNewSession).toHaveBeenCalledTimes(1));
	});

	it("navigates threads with Cmd+Shift brackets", async () => {
		const selectSession = vi.fn();
		const sessionOne = {
			id: "s1",
			worktreePath: "/repo",
			firstMessage: "One",
			messages: [],
			state: "idle" as const,
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit" as const,
		};
		const sessionTwo = {
			...sessionOne,
			id: "s2",
			firstMessage: "Two",
		};
		mockUseAgentChat({
			sessions: [sessionOne, sessionTwo],
			orderedSessions: [sessionOne, sessionTwo],
			activeSession: sessionOne,
			selectSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		fireEvent.keyDown(window, { key: "]", metaKey: true, shiftKey: true });
		fireEvent.keyDown(window, { key: "[", metaKey: true, shiftKey: true });

		await waitFor(() => {
			expect(selectSession).toHaveBeenNthCalledWith(1, "s2");
			expect(selectSession).toHaveBeenNthCalledWith(2, "s2");
		});
	});

	it("renders message input", () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("message-input")).toBeDefined();
	});

	it("copies the latest completed agent response", async () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "First answer" }],
						timestamp: 1000,
					},
					{
						id: "m2",
						role: "agent",
						parts: [{ type: "text", content: "Latest answer" }],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.click(screen.getByLabelText("Copy latest agent response"));

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith("Latest answer"),
		);
	});

	it("copies the previous completed agent response while streaming", async () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "Completed answer" }],
						timestamp: 1000,
					},
					{
						id: "m2",
						role: "agent",
						parts: [{ type: "text", content: "Partial answer" }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.click(screen.getByLabelText("Copy latest agent response"));

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith("Completed answer"),
		);
	});

	it("copies the latest completed agent response with Ctrl+O", async () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "Copy via shortcut" }],
						timestamp: 1000,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.keyDown(window, { key: "o", ctrlKey: true });

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith("Copy via shortcut"),
		);
	});

	it("finds and navigates matches in the current thread", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "search_agent_thread_messages") {
				return Promise.resolve([
					{ messageId: "m1", matchIndex: 0 },
					{ messageId: "m2", matchIndex: 0 },
					{ messageId: "m2", matchIndex: 1 },
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "human",
						parts: [{ type: "text", content: "Please inspect the agent" }],
						timestamp: 1000,
					},
					{
						id: "m2",
						role: "agent",
						parts: [{ type: "text", content: "The agent found an agent bug" }],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.click(screen.getByLabelText("Find in current thread"));
		const input = screen.getByPlaceholderText("Find in current thread");
		fireEvent.change(input, { target: { value: "agent" } });

		await waitFor(() => expect(screen.getByText("1/3")).toBeInTheDocument());
		expect(mockInvoke).toHaveBeenCalledWith("search_agent_thread_messages", {
			request: {
				messages: expect.any(Array),
				query: "agent",
			},
		});
		fireEvent.click(screen.getByLabelText("Next search match"));
		expect(screen.getByText("2/3")).toBeInTheDocument();
		fireEvent.click(screen.getByLabelText("Previous search match"));
		expect(screen.getByText("1/3")).toBeInTheDocument();
	});

	it("refocuses and selects current thread search when Cmd+F is pressed again", async () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "The agent found an agent bug" }],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.keyDown(window, { key: "f", metaKey: true });
		const input = screen.getByPlaceholderText(
			"Find in current thread",
		) as HTMLInputElement;
		fireEvent.change(input, { target: { value: "agent" } });
		input.blur();
		expect(document.activeElement).not.toBe(input);

		fireEvent.keyDown(window, { key: "f", metaKey: true });

		await waitFor(() => expect(document.activeElement).toBe(input));
		expect(input.selectionStart).toBe(0);
		expect(input.selectionEnd).toBe("agent".length);
	});

	it("shows Rust task list report with Ctrl+T", async () => {
		const sendMessage = vi.fn();
		mockInvoke.mockImplementation((command: string) => {
			if (command === "build_agent_task_list_report") {
				return Promise.resolve({
					title: "Tasks: 1 active / 1 finished",
					detail:
						"completed - Explore parser (Explore)\nrunning background - Running tests",
					activeCount: 1,
					completedCount: 1,
					totalCount: 2,
					items: [
						{
							toolUseId: "task-1",
							label: "Explore parser (Explore)",
							status: "completed",
							background: false,
						},
						{
							toolUseId: "task-2",
							label: "Running tests",
							status: "running",
							background: true,
						},
					],
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			sendMessage,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.keyDown(window, { key: "t", ctrlKey: true });

		expect(
			await screen.findByText("Tasks: 1 active / 1 finished"),
		).toBeInTheDocument();
		expect(
			screen.getByText(/running background - Running tests/),
		).toBeInTheDocument();
		expect(screen.getByText("completed")).toBeInTheDocument();
		expect(screen.getByText("running background")).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith("build_agent_task_list_report", {
			chatSessionId: "s1",
		});

		mockInvoke.mockClear();
		fireEvent.click(screen.getByLabelText("Dismiss command result"));
		fireEvent.keyDown(window, { key: "t", ctrlKey: true });

		expect(
			await screen.findByText("Tasks: 1 active / 1 finished"),
		).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith("build_agent_task_list_report", {
			chatSessionId: "s1",
		});
		expect(sendMessage).not.toHaveBeenCalled();
	});

	it("refreshes the Rust task list report while it is open", async () => {
		let taskReportCallCount = 0;
		mockInvoke.mockImplementation((command: string) => {
			if (command === "build_agent_task_list_report") {
				taskReportCallCount += 1;
				return Promise.resolve(
					taskReportCallCount === 1
						? {
								title: "Tasks: 1 active / 0 finished",
								detail: "running background - Running tests",
								activeCount: 1,
								completedCount: 0,
								totalCount: 1,
								items: [
									{
										toolUseId: "task-1",
										label: "Running tests",
										status: "running",
										background: true,
									},
								],
							}
						: {
								title: "Tasks: 0 active / 1 finished",
								detail: "completed background - Running tests",
								activeCount: 0,
								completedCount: 1,
								totalCount: 1,
								items: [
									{
										toolUseId: "task-1",
										label: "Running tests",
										status: "completed",
										background: true,
									},
								],
							},
				);
			}
			return Promise.resolve([]);
		});
		const activeSession = {
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "agent",
					content: "",
					parts: [
						{
							type: "tool_use",
							tool: "Bash",
							id: "task-1",
							input: {
								command: "pnpm test",
								run_in_background: true,
							},
						},
					],
					timestamp: 1000,
				},
			],
			state: "idle",
			createdAt: 1000,
			updatedAt: 1001,
			permissionMode: "edit",
		};
		mockUseAgentChat({ activeSession });
		const { rerender } = render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.keyDown(window, { key: "t", ctrlKey: true });

		expect(
			await screen.findByText("Tasks: 1 active / 0 finished"),
		).toBeInTheDocument();
		expect(taskReportCallCount).toBe(1);

		mockUseAgentChat({
			activeSession: {
				...activeSession,
				messages: [
					{
						...activeSession.messages[0],
						parts: [
							...activeSession.messages[0].parts,
							{
								type: "task_status",
								taskToolUseId: "task-1",
								status: "completed",
								description: "done",
							},
						],
					},
				],
				updatedAt: 1002,
			},
		});
		rerender(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(
			await screen.findByText("Tasks: 0 active / 1 finished"),
		).toBeInTheDocument();
		expect(taskReportCallCount).toBe(2);
	});

	it("loads Rust prompt suggestion and accepts it with Tab", async () => {
		const sendMessage = vi.fn();
		mockInvoke.mockImplementation((command: string) => {
			if (command === "build_agent_prompt_suggestion") {
				return Promise.resolve({
					text: "Review the current repository state and suggest the next step.",
					source: "empty_session",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			sendMessage,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = (await screen.findByPlaceholderText(
			"Review the current repository state and suggest the next step.",
		)) as HTMLTextAreaElement;
		fireEvent.keyDown(textarea, { key: "Tab" });

		expect(textarea.value).toBe(
			"Review the current repository state and suggest the next step.",
		);
		expect(mockInvoke).toHaveBeenCalledWith("build_agent_prompt_suggestion", {
			chatSessionId: "s1",
		});
	});

	it("passes runtime-owned slash commands through to the selected backend", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			sendMessage,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "/compact native UX audit" },
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"/compact native UX audit",
				undefined,
				undefined,
			),
		);
		expect(screen.queryByText("Compaction started")).not.toBeInTheDocument();
	});

	it("passes active and open editor context through normal sends", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			sendMessage,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				activeEditorPath="/repo/src/main.rs"
				openEditorPaths={["/repo/src/main.rs", "/repo/src/lib.rs"]}
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "explain this area" },
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"explain this area",
				undefined,
				undefined,
				{
					editorContext: {
						activeEditorPath: "/repo/src/main.rs",
						openEditorPaths: ["/repo/src/main.rs", "/repo/src/lib.rs"],
					},
				},
			),
		);
	});

	it("passes selected editor line range through normal sends", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
			sendMessage,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				activeEditorPath="/repo/src/main.rs"
				openEditorPaths={["/repo/src/main.rs"]}
				activeEditorSelection={{
					filePath: "/repo/src/main.rs",
					startLine: 12,
					endLine: 14,
				}}
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, {
			target: { value: "explain this selection" },
		});
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"explain this selection",
				undefined,
				undefined,
				{
					editorContext: {
						activeEditorPath: "/repo/src/main.rs",
						openEditorPaths: ["/repo/src/main.rs"],
						selection: {
							filePath: "/repo/src/main.rs",
							startLine: 12,
							endLine: 14,
						},
					},
				},
			),
		);
	});

	it("toggles raw scrollback with the toolbar button", async () => {
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "**bold text**" }],
						createdAt: 1000,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.queryByTestId("agent-raw-message")).not.toBeInTheDocument();

		fireEvent.click(screen.getByLabelText("Enable raw scrollback"));

		const raw = await screen.findByTestId("agent-raw-message");
		expect(raw.textContent).toContain("**bold text**");
	});

	it("toggles raw scrollback from the command palette", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "toggle_raw_scrollback",
						label: "Toggle raw scrollback",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "**raw from palette**" }],
						createdAt: 1000,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);
		expect(screen.queryByTestId("agent-raw-message")).not.toBeInTheDocument();

		mockInvoke.mockClear();
		await waitFor(() => {
			fireEvent.keyDown(window, { key: "k", metaKey: true });
			expect(mockInvoke).toHaveBeenCalledWith(
				"present_agent_command_palette",
				expect.any(Object),
			);
		});
		fireEvent.click(await screen.findByText("Toggle raw scrollback"));

		const raw = await screen.findByTestId("agent-raw-message");
		expect(raw.textContent).toContain("**raw from palette**");
	});
});

describe("AgentChatPanel session tabs", () => {
	it("renders a tab for each session with firstMessage as label", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
				{
					id: "s2",
					firstMessage: "Fix bug",
					messageCount: 5,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByText("Hello")).toBeDefined();
		expect(screen.getByText("Fix bug")).toBeDefined();
	});

	it("shows 'New session' for sessions without firstMessage", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "",
					messageCount: 0,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByText("New session")).toBeDefined();
	});

	it("does not show X button when only one session", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.queryByLabelText("Close Hello")).toBeNull();
	});

	it("shows X button when multiple sessions", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
				{
					id: "s2",
					firstMessage: "Fix bug",
					messageCount: 5,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByLabelText("Close Hello")).toBeDefined();
		expect(screen.getByLabelText("Close Fix bug")).toBeDefined();
	});

	it("calls closeSession when X button is clicked", () => {
		const closeSession = vi.fn();
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
				{
					id: "s2",
					firstMessage: "Fix bug",
					messageCount: 5,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
			closeSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Close Hello"));
		expect(closeSession).toHaveBeenCalledWith("s1");
	});

	it("calls createNewSession when + button is clicked", () => {
		const createNewSession = vi.fn();
		mockUseAgentChat({ createNewSession });
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		fireEvent.click(screen.getByLabelText("New session"));
		expect(createNewSession).toHaveBeenCalled();
	});

	it("enables model selector in the input for an empty active session", () => {
		mockUseAgentChat({
			activeSession: {
				id: "new-s",
				worktreePath: "/repo",
				messages: [],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "edit",
				backendId: "claude",
			},
			availableModels: [
				{
					id: "claude:claude-opus-4-8",
					displayName: "Opus 4.8",
					backend: "claude",
					modelId: "claude-opus-4-8",
				},
			],
			selectedModel: "claude:claude-opus-4-8",
			selectedBackendId: "claude",
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(screen.getByTestId("model-selector-trigger")).toBeEnabled();
	});

	it("falls back to the first model for the active session backend", () => {
		mockUseAgentChat({
			activeSession: {
				id: "codex-s",
				worktreePath: "/repo",
				messages: [],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "edit",
				backendId: "codex",
			},
			availableModels: [
				{
					id: "claude:claude-opus-4-8",
					displayName: "Opus 4.8",
					backend: "claude",
					modelId: "claude-opus-4-8",
				},
				{
					id: "codex:gpt-5.4",
					displayName: "GPT-5.4",
					backend: "codex",
					modelId: "gpt-5.4",
				},
			],
			availableModelsByBackend: {
				claude: [
					{
						id: "claude:claude-opus-4-8",
						displayName: "Opus 4.8",
						backend: "claude",
						modelId: "claude-opus-4-8",
					},
				],
				codex: [
					{
						id: "codex:gpt-5.4",
						displayName: "GPT-5.4",
						backend: "codex",
						modelId: "gpt-5.4",
					},
				],
			},
			selectedBackendId: "codex",
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const trigger = screen.getByTestId("model-selector-trigger");
		expect(trigger).toHaveTextContent("GPT-5.4");
		expect(trigger).not.toHaveTextContent("Opus 4.8");
	});

	it("disables cross-backend model options after the active session has messages", async () => {
		const user = userEvent.setup();
		mockUseAgentChat({
			activeSession: {
				id: "s1",
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
				backendId: "claude",
			},
			availableModels: [
				{
					id: "claude:claude-opus-4-8",
					displayName: "Opus 4.8",
					backend: "claude",
					modelId: "claude-opus-4-8",
				},
				{
					id: "codex:gpt-5.4",
					displayName: "GPT-5.4",
					backend: "codex",
					modelId: "gpt-5.4",
				},
			],
			selectedModel: "claude:claude-opus-4-8",
			selectedBackendId: "claude",
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		expect(
			screen.getByText("GPT-5.4").closest("[role='menuitem']"),
		).toHaveAttribute("data-disabled");
	});

	it("calls selectSession when tab is clicked", () => {
		const selectSession = vi.fn();
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
				{
					id: "s2",
					firstMessage: "Fix bug",
					messageCount: 5,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
			selectSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		// Radix Tabs.Trigger は onMouseDown で value を確定する仕様。
		// fireEvent.click では Radix の Trigger ハンドラが発火しないため mouseDown を使う。
		fireEvent.mouseDown(screen.getByText("Fix bug"));
		expect(selectSession).toHaveBeenCalledWith("s2");
	});
});

describe("AgentChatPanel agent state reflection", () => {
	it("shows AgentStateIcon on tab when session is running", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
			sessionAgentStates: new Map([["s1", "running"]]),
			isStreaming: true,
			activeSession: {
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
						parts: [{ type: "text", content: "working..." }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const tab = screen.getByText("Hello").closest("[role='tab']");
		expect(tab?.querySelector("[title='running']")).not.toBeNull();
	});

	it("shows AgentStateIcon with waiting state on tab", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "active",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
			sessionAgentStates: new Map([["s1", "waiting"]]),
			isStreaming: true,
			pendingPermission: {
				request_id: "req-001",
				tool_name: "Edit",
				input: {},
				tool_use_id: "toolu_001",
			},
			activeSession: {
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
						parts: [{ type: "text", content: "editing..." }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const tab = screen.getByText("Hello").closest("[role='tab']");
		expect(tab?.querySelector("[title='waiting']")).not.toBeNull();
	});

	it("shows AgentStateIcon with done state on tab when session is idle", () => {
		mockUseAgentChat({
			sessions: [
				{
					id: "s1",
					firstMessage: "Hello",
					messageCount: 3,
					worktreePath: "/repo",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
			],
			sessionAgentStates: new Map([["s1", "done"]]),
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const tab = screen.getByText("Hello").closest("[role='tab']");
		expect(tab?.querySelector("[title='done']")).not.toBeNull();
	});

	it("reflects permissionMode ask in ModeSelector trigger label", () => {
		mockUseAgentChat({
			permissionMode: "ask",
			activeSession: {
				id: "s1",
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
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Ask",
		);
	});
});

describe("AgentChatPanel shimmer placeholder", () => {
	it("shows 3-line shimmer when streaming with empty agent parts", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer).toBeDefined();
		expect(shimmer.children).toHaveLength(3);
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
	});

	it("shows 2-line shimmer during thinking phase while streaming", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{ type: "thinking", content: "Let me think about this..." },
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer).toBeDefined();
		expect(shimmer.children).toHaveLength(2);
		expect(screen.getByTestId("thinking-block")).toBeInTheDocument();
		expect(screen.getByText("Let me think about this...")).toBeInTheDocument();
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
	});

	it("shows 1-line shimmer when streaming text", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [{ type: "text", content: "I am responding..." }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer).toBeDefined();
		expect(shimmer.children).toHaveLength(1);
	});

	it("shows 2-line shimmer when last part is tool_use", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{
								type: "tool_use",
								tool: "Read",
								input: { file_path: "/src/main.ts" },
								id: "t1",
							},
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer).toBeDefined();
		expect(shimmer.children).toHaveLength(2);
		const toolEl = screen.getByTestId("activity-tool-use-0");
		expect(toolEl.querySelector(".animate-spin")).not.toBeNull();
	});

	it("shows 2-line shimmer when last part is tool_result", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{
								type: "tool_use",
								tool: "Read",
								input: { file_path: "/src/main.ts" },
								id: "t1",
							},
							{
								type: "tool_result",
								content: "file content",
								isError: false,
								toolUseId: "t1",
							},
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer).toBeDefined();
		expect(shimmer.children).toHaveLength(2);
	});

	it("hides shimmer when last part is permission", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{
								type: "permission",
								request: {
									request_id: "r1",
									tool_name: "Edit",
									input: {},
									tool_use_id: "tu1",
								},
								status: "pending",
							},
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.queryByTestId("shimmer-placeholder")).toBeNull();
	});

	it("hides shimmer when last part is error", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [{ type: "error", content: "Something went wrong" }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.queryByTestId("shimmer-placeholder")).toBeNull();
	});

	it("shows thinking and text when text arrives after thinking", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{ type: "thinking", content: "Let me think..." },
							{ type: "text", content: "I am responding..." },
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("thinking-block")).toBeInTheDocument();
		expect(screen.getByText("Let me think...")).toBeInTheDocument();
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
	});

	it("collapses and expands thinking content", async () => {
		mockUseAgentChat({
			activeSession: {
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
						parts: [{ type: "thinking", content: "private reasoning" }],
						timestamp: 1001,
					},
				],
				state: "done",
				createdAt: 1000,
				updatedAt: 1001,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const toggle = screen.getByRole("button", { name: "Thinking" });
		await waitFor(() =>
			expect(screen.queryByText("private reasoning")).toBeNull(),
		);
		fireEvent.click(toggle);
		expect(screen.getByText("private reasoning")).toBeInTheDocument();
		fireEvent.click(toggle);
		expect(screen.queryByText("private reasoning")).toBeNull();
	});

	it("toggles all thinking content from the transcript control", async () => {
		mockUseAgentChat({
			activeSession: {
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
						parts: [{ type: "thinking", content: "private reasoning" }],
						timestamp: 1001,
					},
				],
				state: "done",
				createdAt: 1000,
				updatedAt: 1001,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await waitFor(() =>
			expect(screen.queryByText("private reasoning")).toBeNull(),
		);
		fireEvent.click(screen.getByRole("button", { name: "Thinking" }));
		expect(screen.getByText("private reasoning")).toBeInTheDocument();
		fireEvent.click(screen.getByLabelText("Hide thinking"));
		expect(screen.queryByText("private reasoning")).toBeNull();
		fireEvent.click(screen.getByLabelText("Show thinking"));
		expect(screen.getByText("private reasoning")).toBeInTheDocument();
	});

	it("toggles thinking content with Tab when the transcript is focused", () => {
		mockUseAgentChat({
			activeSession: {
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
						parts: [{ type: "thinking", content: "private reasoning" }],
						timestamp: 1001,
					},
				],
				state: "done",
				createdAt: 1000,
				updatedAt: 1001,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		screen.getByLabelText("Hide thinking").focus();
		fireEvent.keyDown(window, { key: "Tab" });

		expect(screen.queryByText("private reasoning")).toBeNull();
	});

	it("hides shimmer when streaming is finished", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [{ type: "text", content: "Done." }],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
		expect(screen.queryByTestId("shimmer-placeholder")).toBeNull();
	});

	it("pairs tool_result with tool_use by toolUseId across parallel calls", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [
							{
								type: "tool_use",
								tool: "Read",
								input: { file_path: "/a.ts" },
								id: "t1",
							},
							{
								type: "tool_use",
								tool: "Read",
								input: { file_path: "/b.ts" },
								id: "t2",
							},
							{
								type: "tool_result",
								content: "content-a",
								isError: false,
								toolUseId: "t1",
							},
							{
								type: "tool_result",
								content: "content-b",
								isError: false,
								toolUseId: "t2",
							},
						],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		// Both tool_use items should be rendered, but no standalone tool_result items
		expect(screen.getByTestId("activity-tool-use-0")).toBeDefined();
		expect(screen.getByTestId("activity-tool-use-1")).toBeDefined();
		expect(screen.queryByTestId("activity-tool-result-2")).toBeNull();
		expect(screen.queryByTestId("activity-tool-result-3")).toBeNull();
	});
});

describe("AgentChatPanel Shift+Tab mode cycle", () => {
	const emptyActiveSession = {
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "idle" as const,
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit" as const,
	};

	it("cycles between the three abstract modes on Shift+Tab in textarea", () => {
		const setPermissionMode = vi.fn();
		mockUseAgentChat({
			permissionMode: "edit",
			setPermissionMode,
			activeSession: emptyActiveSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		// ask → edit → full → ask (MODES order: ask[0], edit[1], full[2])
		// edit (index 1) → full (index 2)
		expect(setPermissionMode).toHaveBeenCalledWith("s1", "full");
	});

	it("uses the same cycle regardless of the selected backend", () => {
		const setPermissionMode = vi.fn();
		mockUseAgentChat({
			permissionMode: "ask",
			selectedBackendId: "codex",
			setPermissionMode,
			activeSession: emptyActiveSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		// ask (index 0) → edit (index 1)
		expect(setPermissionMode).toHaveBeenCalledWith("s1", "edit");
	});

	it("wraps around from full back to ask on Shift+Tab", () => {
		const setPermissionMode = vi.fn();
		mockUseAgentChat({
			permissionMode: "full",
			setPermissionMode,
			activeSession: emptyActiveSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		// full (index 2) → ask (index 0) via (currentIndex + 1) % MODES.length
		expect(setPermissionMode).toHaveBeenCalledWith("s1", "ask");
	});
});

describe("AgentChatPanel Task tool rendering", () => {
	it("renders TaskToolActivity and excludes child parts from flat display", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [
							{
								type: "tool_use",
								tool: "Task",
								input: {
									description: "Explore code",
									subagent_type: "Explore",
								},
								id: "toolu_task_001",
							},
							{
								type: "tool_use",
								tool: "Read",
								input: { file_path: "/src/main.ts" },
								id: "toolu_child_001",
								parentToolUseId: "toolu_task_001",
							},
							{
								type: "tool_result",
								content: "file content",
								isError: false,
								toolUseId: "toolu_child_001",
								parentToolUseId: "toolu_task_001",
							},
							{
								type: "task_status",
								taskToolUseId: "toolu_task_001",
								status: "completed",
								summary: "Done",
							},
							{
								type: "tool_result",
								content: "task result",
								isError: false,
								toolUseId: "toolu_task_001",
							},
						],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		// TaskToolActivity should be rendered
		expect(screen.getByTestId("activity-task-0")).toBeDefined();

		// Child parts should NOT be rendered as standalone items
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
		expect(screen.queryByTestId("activity-tool-result-2")).toBeNull();
	});

	it("shows 2-line shimmer when last part is task_status", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{
								type: "tool_use",
								tool: "Task",
								input: {
									description: "Explore code",
									subagent_type: "Explore",
								},
								id: "toolu_task_001",
							},
							{
								type: "task_status",
								taskToolUseId: "toolu_task_001",
								status: "started",
							},
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const shimmer = screen.getByTestId("shimmer-placeholder");
		expect(shimmer.children).toHaveLength(2);
	});
});

describe("SystemNotificationItem rendering", () => {
	it("shows ⏳ with animate-pulse when status is in_progress", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
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
						parts: [
							{
								type: "system_notification",
								notificationType: "compaction",
								status: "in_progress",
								label: "Compacting conversation...",
							},
						],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const el = screen.getByText(/Compacting conversation/);
		expect(el).toBeDefined();
		expect(el.textContent).toContain("⏳");
		expect(el.className).toContain("animate-pulse");
	});

	it("shows ✓ when status is completed", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [
							{
								type: "system_notification",
								notificationType: "compaction",
								status: "completed",
								label: "Conversation compacted",
							},
						],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const el = screen.getByText(/Conversation compacted/);
		expect(el).toBeDefined();
		expect(el.textContent).toContain("✓");
		expect(el.className).not.toContain("animate-pulse");
	});

	it("shows ❌ when status is error", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [
							{
								type: "system_notification",
								notificationType: "compaction",
								status: "error",
								label: "Conversation compaction failed",
							},
						],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const el = screen.getByText(/Conversation compaction failed/);
		expect(el).toBeDefined();
		expect(el.textContent).toContain("❌");
	});

	it("shows detail in parentheses when detail is provided", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
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
						parts: [
							{
								type: "system_notification",
								notificationType: "compaction",
								status: "completed",
								label: "Conversation compacted",
								detail: "trigger=auto, 50000 tokens",
							},
						],
						timestamp: 1001,
					},
				],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByText("(trigger=auto, 50000 tokens)")).toBeDefined();
	});
});

describe("AgentChatPanel session history", () => {
	it("renders history button", () => {
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByLabelText("Session history")).toBeDefined();
	});

	it("shows 'No closed sessions' when closedSessions is empty", () => {
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Session history"));
		expect(screen.getByText("No closed sessions")).toBeDefined();
	});

	it("shows closed sessions in popover and calls restoreSession on click", () => {
		const restoreSession = vi.fn();
		mockUseAgentChat({
			closedSessions: [
				{
					id: "closed-1",
					firstMessage: "Old conversation",
					messageCount: 5,
					worktreePath: "/repo",
					state: "closed",
					createdAt: 500,
					updatedAt: 500,
				},
			],
			restoreSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		fireEvent.click(screen.getByLabelText("Session history"));
		expect(screen.getByText("Old conversation")).toBeDefined();
		fireEvent.click(screen.getByText("Old conversation"));
		expect(restoreSession).toHaveBeenCalledWith("closed-1");
	});

	it("archives a closed session from the history popover", () => {
		const archiveSession = vi.fn();
		const restoreSession = vi.fn();
		mockUseAgentChat({
			closedSessions: [
				{
					id: "closed-1",
					firstMessage: "Old conversation",
					messageCount: 5,
					worktreePath: "/repo",
					state: "closed",
					createdAt: 500,
					updatedAt: 500,
				},
			],
			archiveSession,
			restoreSession,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.click(screen.getByLabelText("Session history"));
		fireEvent.click(screen.getByLabelText("Archive Old conversation"));

		expect(archiveSession).toHaveBeenCalledWith("closed-1");
		expect(restoreSession).not.toHaveBeenCalled();
	});

	it("searches across sessions from history popover and restores a result", async () => {
		const restoreSession = vi.fn();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "search_agent_sessions") {
				expect(args).toEqual({
					worktreePath: "/repo",
					query: "parser",
					includeWorkflow: false,
					limit: 20,
				});
				return Promise.resolve([
					{
						session: {
							id: "s2",
							worktreePath: "/repo",
							state: "closed",
							createdAt: 1000,
							updatedAt: 1002,
							firstMessage: "Fix parser bug",
							messageCount: 4,
							permissionMode: "edit",
						},
						matchedMessageId: "m2",
						matchedRole: "agent",
						snippet: "The parser bug is fixed",
					},
				]);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({ restoreSession });
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		fireEvent.click(screen.getByLabelText("Session history"));
		fireEvent.change(screen.getByLabelText("Search sessions"), {
			target: { value: "parser" },
		});

		expect(await screen.findByText("Fix parser bug")).toBeInTheDocument();
		expect(screen.getByText("The parser bug is fixed")).toBeInTheDocument();

		fireEvent.click(screen.getByText("Fix parser bug"));
		expect(restoreSession).toHaveBeenCalledWith("s2");
	});

	it("focuses session search when opened from Cmd+G", async () => {
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_agent_shortcut_settings",
				undefined,
			),
		);

		const search = await waitFor(async () => {
			fireEvent.keyDown(window, { key: "g", metaKey: true });
			return await screen.findByLabelText("Search sessions");
		});
		await waitFor(() => expect(search).toHaveFocus());
	});
});

describe("AgentChatPanel image drag and drop", () => {
	const emptyActiveSession = {
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "idle" as const,
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit" as const,
	};

	it("shows drop overlay on dragover with files and hides on dragleave", () => {
		mockUseAgentChat({ activeSession: emptyActiveSession });
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const dropZone = screen
			.getByTestId("agent-chat-panel")
			.querySelector("[class*='relative']") as HTMLElement;
		expect(dropZone).not.toBeNull();

		// Initially no overlay
		expect(screen.queryByText("Drop image to attach")).toBeNull();

		// Drag over with Files
		fireEvent.dragOver(dropZone, {
			dataTransfer: { types: ["Files"], dropEffect: "" },
		});
		expect(screen.getByText("Drop image to attach")).toBeDefined();

		// Drag leave (relatedTarget outside currentTarget)
		fireEvent.dragLeave(dropZone, {
			relatedTarget: document.body,
		});
		expect(screen.queryByText("Drop image to attach")).toBeNull();
	});

	it("calls prepare_image_attachments_from_paths on native file drop", async () => {
		mockUseAgentChat({ activeSession: emptyActiveSession });
		mockInvoke.mockImplementation(async (cmd: string) => {
			if (cmd === "prepare_image_attachments_from_paths") {
				return [{ data: "aGVsbG8=", mediaType: "image/png" }];
			}
			return [];
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		// registerDropZone should have been called with "agent" zone
		const agentDropCallback = agentDropCallbacks.get("agent");
		expect(agentDropCallback).toBeDefined();

		await act(async () => {
			await agentDropCallback?.(["/tmp/test.png"]);
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"prepare_image_attachments_from_paths",
				{ paths: ["/tmp/test.png"] },
			);
		});

		await waitFor(() => {
			expect(screen.getByTestId("image-preview-list")).toBeDefined();
			expect(screen.getAllByTestId("image-preview-item")).toHaveLength(1);
		});
	});

	it("does not show preview when dropped files are not images", async () => {
		mockUseAgentChat({ activeSession: emptyActiveSession });
		mockInvoke.mockImplementation(async (cmd: string) => {
			if (cmd === "prepare_image_attachments_from_paths") {
				return [];
			}
			return [];
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const agentDropCallback = agentDropCallbacks.get("agent");
		expect(agentDropCallback).toBeDefined();

		await act(async () => {
			await agentDropCallback?.(["/tmp/readme.txt"]);
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"prepare_image_attachments_from_paths",
				{ paths: ["/tmp/readme.txt"] },
			);
		});

		expect(screen.queryByTestId("image-preview-list")).toBeNull();
	});
});

describe("AgentChatPanel workflow panel visibility", () => {
	// spec issues-1023: workflow state 変化を契機にした refreshSessions の発火は
	// AgentChatProvider 側責務へ移った（context provider が useEffect で監視する）。
	// AgentChatPanel 自身は context から渡された session 一覧を表示するだけなので、
	// この特定の挙動は本 panel テストの対象外。Provider 側でカバーする。

	// spec issues-1023: AgentChatPanel は WorkflowPanel をホストしない
	// （右パネル側 Workflow モードに切り出された）。本パネルでの責務は free chat
	// session の tab bar 表示と、workflow step session を tab bar から除外することのみ。

	it("excludes workflow step sessions from the chat tab bar", () => {
		useWorkflowStateMock.mockReturnValue({ workflowState: null });
		mockUseAgentChat({
			sessions: [
				{
					id: "free-1",
					firstMessage: "Free chat",
					messageCount: 1,
					worktreePath: "/test",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
				},
				{
					id: "workflow-step-1",
					firstMessage: "Workflow step",
					messageCount: 1,
					worktreePath: "/test",
					state: "idle",
					createdAt: 1000,
					updatedAt: 1000,
					workflowStepSession: true,
				},
			],
			activeSession: {
				id: "free-1",
				worktreePath: "/test",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "edit",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/test"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		const tabList = screen.getByTestId("session-tab-list");
		expect(within(tabList).getByText("Free chat")).toBeInTheDocument();
		expect(within(tabList).queryByText("Workflow step")).toBeNull();
	});
});
