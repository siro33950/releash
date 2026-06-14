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
		id: "copy_response_options",
		label: "Copy response...",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "export_transcript",
		label: "Export transcript",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "create_agents_md",
		label: "Create AGENTS.md",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "toggle_raw_scrollback",
		label: "Toggle raw scrollback",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "debug_config",
		label: "Debug config",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "doctor",
		label: "Doctor",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_account_usage",
		label: "Codex account usage",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_goal",
		label: "Codex goal",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_compact_context",
		label: "Compact context",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_clean_background_terminals",
		label: "Clean Codex background terminals",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_shell_command",
		label: "Run Codex shell command",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_review_uncommitted_changes",
		label: "Codex review uncommitted changes",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_review_base_branch",
		label: "Codex review base branch",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_review_commit",
		label: "Codex review commit",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_review_custom",
		label: "Codex review custom",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_thread_history",
		label: "Codex thread history",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_thread_transcript",
		label: "Codex thread transcript",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_permission_profiles",
		label: "Codex permission profiles",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_hooks",
		label: "Codex hooks",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_realtime_voices",
		label: "Codex realtime voices",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_realtime_text_start",
		label: "Start Codex realtime text",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_realtime_text_append",
		label: "Append Codex realtime text",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_realtime_stop",
		label: "Stop Codex realtime",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_mcp_status",
		label: "Codex MCP status",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_runtime_config",
		label: "Codex runtime config",
		shortcut: "",
		defaultShortcut: "",
	},
	{
		id: "codex_runtime_capabilities",
		label: "Codex runtime capabilities",
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
		availableModels: [],
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
		getSessionCodexGoal: vi.fn().mockReturnValue(null),
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
					backendId: null,
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
					backendId: null,
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

	it("does not run Codex-only shortcuts for non-Codex sessions", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(
					defaultAgentShortcuts.map((shortcut) =>
						shortcut.id === "codex_compact_context"
							? { ...shortcut, shortcut: "Ctrl Shift C" }
							: shortcut,
					),
				);
			}
			if (command === "is_agent_command_enabled") {
				return Promise.resolve(false);
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "claude",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1000,
				permissionMode: "edit",
				backendId: "claude",
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

		fireEvent.keyDown(window, { key: "c", ctrlKey: true, shiftKey: true });

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("is_agent_command_enabled", {
				request: {
					commandId: "codex_compact_context",
					request: {
						hasActiveSession: true,
						sessionCount: 0,
						backendId: "claude",
					},
				},
			}),
		);
		expect(mockInvoke).not.toHaveBeenCalledWith("compact_agent_context", {
			chatSessionId: "s1",
		});
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

	it("runs leading bang input through live shell mode and sends command output prompt", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockInvoke.mockImplementation((command: string) => {
			if (command === "prepare_agent_shell_input") {
				return Promise.resolve({
					command: "printf hello",
					displayCommand: "printf hello",
					label: "agent-shell:printf hello",
					timeoutSecs: 120,
					background: false,
				});
			}
			if (command === "spawn_oneshot_pty") {
				return Promise.resolve({
					pty_id: 77,
					session_key: "shell-77",
					worktree_path: "/repo",
					label: "agent-shell:printf hello",
					status: "running",
					exit_code: null,
					started_at: 1000,
					completed_at: null,
				});
			}
			if (command === "build_agent_shell_command_context_prompt") {
				return Promise.resolve({
					title: "Shell: completed",
					detail: "hello",
					prompt: "Shell context prompt with hello",
					exitCode: 0,
					timedOut: false,
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

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "! printf hello" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(await screen.findByText("Shell: running")).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith("prepare_agent_shell_input", {
			content: "! printf hello",
		});
		expect(mockInvoke).toHaveBeenCalledWith("spawn_oneshot_pty", {
			worktreePath: "/repo",
			command: "printf hello",
			label: "agent-shell:printf hello",
			timeoutSecs: 120,
		});

		await waitFor(() => expect(listenCallbacks.has("pty-output")).toBe(true));
		act(() => {
			listenCallbacks.get("pty-output")?.({
				payload: { pty_id: 77, data: "hello" },
			});
		});
		expect(await screen.findByText("hello")).toBeInTheDocument();

		act(() => {
			listenCallbacks.get("oneshot-pty-status-changed")?.({
				payload: {
					pty_id: 77,
					session_key: "shell-77",
					worktree_path: "/repo",
					label: "agent-shell:printf hello",
					status: "completed",
					exit_code: 0,
					started_at: 1000,
					completed_at: 1001,
				},
			});
		});
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"build_agent_shell_command_context_prompt",
				{
					command: "printf hello",
					output: "hello",
					exitCode: 0,
					timedOut: false,
					truncated: false,
				},
			),
		);
		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"Shell context prompt with hello",
				undefined,
				undefined,
			),
		);
	});

	it("passes regular text through after Rust shell classification returns none", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockInvoke.mockImplementation((command: string) => {
			if (command === "prepare_agent_shell_input") {
				return Promise.resolve(null);
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

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "regular prompt" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("prepare_agent_shell_input", {
				content: "regular prompt",
			}),
		);
		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"regular prompt",
				undefined,
				undefined,
			),
		);
	});

	it("runs trailing ampersand shell input through Rust-prepared background mode", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		mockInvoke.mockImplementation((command: string) => {
			if (command === "prepare_agent_shell_input") {
				return Promise.resolve({
					command: "pnpm test",
					displayCommand: "pnpm test",
					label: "agent-shell-bg:pnpm test",
					background: true,
				});
			}
			if (command === "spawn_oneshot_pty") {
				return Promise.resolve({
					pty_id: 78,
					session_key: "shell-78",
					worktree_path: "/repo",
					label: "agent-shell-bg:pnpm test",
					status: "running",
					exit_code: null,
					started_at: 1000,
					completed_at: null,
				});
			}
			if (command === "build_agent_shell_command_context_prompt") {
				return Promise.resolve({
					title: "Shell: completed",
					detail: "test log line",
					prompt: "Background shell context prompt",
					exitCode: 0,
					timedOut: false,
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

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "! pnpm test &" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(
			await screen.findByText("Shell background: running"),
		).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith("prepare_agent_shell_input", {
			content: "! pnpm test &",
		});
		expect(mockInvoke).toHaveBeenCalledWith("spawn_oneshot_pty", {
			worktreePath: "/repo",
			command: "pnpm test",
			label: "agent-shell-bg:pnpm test",
			timeoutSecs: null,
		});

		await waitFor(() => expect(listenCallbacks.has("pty-output")).toBe(true));
		act(() => {
			listenCallbacks.get("pty-output")?.({
				payload: { pty_id: 78, data: "test log line" },
			});
			listenCallbacks.get("oneshot-pty-status-changed")?.({
				payload: {
					pty_id: 78,
					session_key: "shell-78",
					worktree_path: "/repo",
					label: "agent-shell-bg:pnpm test",
					status: "completed",
					exit_code: 0,
					started_at: 1000,
					completed_at: 1001,
				},
			});
		});

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"build_agent_shell_command_context_prompt",
				{
					command: "pnpm test",
					output: "test log line",
					exitCode: 0,
					timedOut: false,
					truncated: false,
				},
			),
		);
		await waitFor(() =>
			expect(sendMessage).toHaveBeenCalledWith(
				"s1",
				"Background shell context prompt",
				undefined,
				undefined,
			),
		);
	});

	it("stops an owned background shell command from the shell panel", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "prepare_agent_shell_input") {
				return Promise.resolve({
					command: "pnpm test --watch",
					displayCommand: "pnpm test --watch",
					label: "agent-shell-bg:pnpm test --watch",
					background: true,
				});
			}
			if (command === "spawn_oneshot_pty") {
				return Promise.resolve({
					pty_id: 79,
					session_key: "shell-79",
					worktree_path: "/repo",
					label: "agent-shell-bg:pnpm test --watch",
					status: "running",
					exit_code: null,
					started_at: 1000,
					completed_at: null,
				});
			}
			if (command === "cancel_oneshot_pty") {
				return Promise.resolve(undefined);
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
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "! pnpm test --watch &" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(
			await screen.findByText("Shell background: running"),
		).toBeInTheDocument();
		fireEvent.click(screen.getByLabelText("Cancel shell command"));

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("cancel_oneshot_pty", {
				ptyId: 79,
			}),
		);
	});

	it("queues leading bang shell input while the agent is streaming", async () => {
		const sendMessage = vi.fn().mockResolvedValue(undefined);
		const activeSession = {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1000,
			updatedAt: 1001,
			permissionMode: "edit",
		};
		mockInvoke.mockImplementation((command: string) => {
			if (command === "prepare_agent_shell_input") {
				return Promise.resolve({
					command: "printf queued",
					displayCommand: "printf queued",
					label: "agent-shell:printf queued",
					timeoutSecs: 120,
					background: false,
				});
			}
			if (command === "spawn_oneshot_pty") {
				return Promise.resolve({
					pty_id: 79,
					session_key: "shell-79",
					worktree_path: "/repo",
					label: "agent-shell:printf queued",
					status: "running",
					exit_code: null,
					started_at: 1000,
					completed_at: null,
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			activeSession,
			isStreaming: true,
			sendMessage,
		});
		const view = render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.change(textarea, { target: { value: "! printf queued" } });
		fireEvent.keyDown(textarea, { key: "Enter", metaKey: true });

		expect(await screen.findByText("Queued shell 1")).toBeInTheDocument();
		expect(screen.getAllByText("printf queued").length).toBeGreaterThan(0);
		expect(mockInvoke).toHaveBeenCalledWith("prepare_agent_shell_input", {
			content: "! printf queued",
		});
		expect(mockInvoke).not.toHaveBeenCalledWith("spawn_oneshot_pty", {
			worktreePath: "/repo",
			command: "printf queued",
			label: "agent-shell:printf queued",
			timeoutSecs: 120,
		});

		mockUseAgentChat({
			activeSession: { ...activeSession, state: "idle" },
			isStreaming: false,
			sendMessage,
		});
		view.rerender(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("spawn_oneshot_pty", {
				worktreePath: "/repo",
				command: "printf queued",
				label: "agent-shell:printf queued",
				timeoutSecs: 120,
			}),
		);
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

	it("opens transcript export from the command palette", async () => {
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "export_transcript",
						label: "Export transcript",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "build_agent_export_transcript") {
				expect(args).toEqual({
					chatSessionId: "s1",
					raw: undefined,
				});
				return Promise.resolve({
					title: "Transcript ready",
					detail: "Prepared 1 message for clipboard export.",
					content: "# Releash Agent Transcript\n\nAgent:\nHello\n",
					path: null,
					suggestedPath: "transcripts/releash-agent-s1.md",
					messageCount: 1,
				});
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
						parts: [{ type: "text", content: "Hello" }],
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

		mockInvoke.mockClear();
		await waitFor(() => {
			fireEvent.keyDown(window, { key: "k", metaKey: true });
			expect(mockInvoke).toHaveBeenCalledWith(
				"present_agent_command_palette",
				expect.any(Object),
			);
		});
		fireEvent.click(await screen.findByText("Export transcript"));

		expect(await screen.findByText("Transcript ready")).toBeInTheDocument();
		const pathInput = screen.getByLabelText(
			"Transcript export write path",
		) as HTMLInputElement;
		expect(pathInput.value).toBe("transcripts/releash-agent-s1.md");
		await waitFor(() => expect(document.activeElement).toBe(pathInput));
	});

	it("opens transcript export from the agent actions menu", async () => {
		const user = userEvent.setup();
		let exportCallCount = 0;
		mockInvoke.mockImplementation((command, args) => {
			if (command === "build_agent_export_transcript") {
				exportCallCount += 1;
				if (exportCallCount === 1) {
					expect(args).toEqual({
						chatSessionId: "s1",
						raw: undefined,
					});
					return Promise.resolve({
						title: "Transcript ready",
						detail: "Prepared 2 messages for clipboard export.",
						content: "# Releash Agent Transcript\n\nHuman:\nHello\n",
						path: null,
						suggestedPath: "transcripts/releash-agent-s1.md",
						messageCount: 2,
					});
				}
				expect(args).toEqual({
					chatSessionId: "s1",
					raw: "transcripts/releash-agent-s1.md",
				});
				return Promise.resolve({
					title: "Transcript exported",
					detail:
						"Wrote 2 message transcript to /repo/transcripts/releash-agent-s1.md",
					content: "# Releash Agent Transcript\n\nHuman:\nHello\n",
					path: "/repo/transcripts/releash-agent-s1.md",
					suggestedPath: "transcripts/releash-agent-s1.md",
					messageCount: 2,
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
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Export transcript"));

		expect(await screen.findByText("Transcript ready")).toBeInTheDocument();
		expect(await screen.findByText("Copy transcript")).toBeInTheDocument();
		expect(await screen.findByText("Write transcript")).toBeInTheDocument();
		const pathInput = screen.getByLabelText(
			"Transcript export write path",
		) as HTMLInputElement;
		expect(pathInput.value).toBe("transcripts/releash-agent-s1.md");
		await waitFor(() => expect(document.activeElement).toBe(pathInput));
		expect(clipboardWriteText).not.toHaveBeenCalled();

		await user.click(screen.getByText("Write transcript"));

		expect(await screen.findByText("Transcript exported")).toBeInTheDocument();
		expect(exportCallCount).toBe(2);
	});

	it("opens copy response options from the command palette", async () => {
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "copy_response_options",
						label: "Copy response...",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "build_agent_copy_response") {
				expect(args).toEqual({
					chatSessionId: "s1",
					raw: undefined,
					excludeMessageId: undefined,
				});
				return Promise.resolve({
					title: "Copied latest response",
					detail: "Copied the latest completed agent response.",
					content: "Use this code:\n```ts\nconst ok = true;\n```",
					ordinal: 1,
					messageId: "m-agent",
					suggestedPath: "snippets/releash-response-m-agent.md",
					codeBlocks: [
						{
							index: 1,
							language: "ts",
							label: "Block 1 (ts, 1 lines)",
							content: "const ok = true;",
							lineCount: 1,
							suggestedPath: "snippets/releash-response-m-agent-block-1.ts",
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
		fireEvent.click(await screen.findByText("Copy response..."));

		expect(
			await screen.findByText("Copied latest response"),
		).toBeInTheDocument();
		expect(await screen.findByText("Whole response")).toBeInTheDocument();
		expect(
			await screen.findByText("Block 1 (ts, 1 lines)"),
		).toBeInTheDocument();
		const pathInput = screen.getByLabelText(
			"Copy selection write path",
		) as HTMLInputElement;
		expect(pathInput.value).toBe("snippets/releash-response-m-agent.md");
		await waitFor(() => expect(document.activeElement).toBe(pathInput));
	});

	it("opens copy response options from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "build_agent_copy_response") {
				expect(args).toEqual({
					chatSessionId: "s1",
					raw: undefined,
					excludeMessageId: undefined,
				});
				return Promise.resolve({
					title: "Copied latest response",
					detail: "Copied the latest completed agent response.",
					content: "Use this code:\n```ts\nconst ok = true;\n```",
					ordinal: 1,
					messageId: "m-agent",
					suggestedPath: "snippets/releash-response-m-agent.md",
					codeBlocks: [
						{
							index: 1,
							language: "ts",
							label: "Block 1 (ts, 1 lines)",
							content: "const ok = true;\n",
							lineCount: 1,
							suggestedPath: "snippets/releash-response-m-agent-block-1.ts",
						},
					],
				});
			}
			if (command === "write_agent_copy_selection_to_file") {
				expect(args).toEqual({
					worktreePath: "/repo",
					rawPath: "snippets/releash-response-m-agent-block-1.ts",
					content: "const ok = true;\n",
				});
				return Promise.resolve({
					title: "Copy selection written",
					detail:
						"Wrote 17 bytes to /repo/snippets/releash-response-m-agent-block-1.ts",
					path: "/repo/snippets/releash-response-m-agent-block-1.ts",
					byteCount: 17,
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
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Copy response..."));

		expect(
			await screen.findByText("Copied latest response"),
		).toBeInTheDocument();
		expect(await screen.findByText("Whole response")).toBeInTheDocument();
		const pathInput = screen.getByLabelText(
			"Copy selection write path",
		) as HTMLInputElement;
		expect(pathInput.value).toBe("snippets/releash-response-m-agent.md");
		await waitFor(() => expect(document.activeElement).toBe(pathInput));
		await user.click(await screen.findByText("Block 1 (ts, 1 lines)"));

		expect(
			await screen.findByText("Copied Block 1 (ts, 1 lines)."),
		).toBeInTheDocument();

		await user.click(screen.getAllByText("Write")[1]);

		expect(
			await screen.findByText("Copy selection written"),
		).toBeInTheDocument();
	});

	it("creates AGENTS.md from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "create_agents_md_scaffold") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve({
					path: "/repo/AGENTS.md",
					created: true,
					content: "# Agent Instructions\n",
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
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Create AGENTS.md"));

		expect(await screen.findByText("AGENTS.md created")).toBeInTheDocument();
		expect(
			await screen.findByText("Created starter guidance at /repo/AGENTS.md"),
		).toBeInTheDocument();
	});

	it("starts Codex runtime context compaction from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "compact_agent_context") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Context compaction started",
					detail: "Requested runtime compaction for codex.",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Compact context"));

		expect(
			await screen.findByText("Context compaction started"),
		).toBeInTheDocument();
		expect(
			await screen.findByText("Requested runtime compaction for codex."),
		).toBeInTheDocument();
	});

	it("cleans Codex background terminals from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "clean_codex_background_terminals") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex background terminals cleaned",
					detail:
						"Requested runtime cleanup for this thread's background terminals.",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(
			await screen.findByText("Clean Codex background terminals"),
		);

		expect(
			await screen.findByText("Codex background terminals cleaned"),
		).toBeInTheDocument();
		expect(
			await screen.findByText(
				"Requested runtime cleanup for this thread's background terminals.",
			),
		).toBeInTheDocument();
	});

	it("runs Codex runtime shell command from the current composer draft", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "run_codex_shell_command") {
				expect(args).toEqual({
					chatSessionId: "s1",
					content: "! git status --short",
				});
				return Promise.resolve({
					title: "Codex shell command sent",
					detail:
						"Runtime shell command sent for this thread: git status --short",
					command: "git status --short",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.type(
			screen.getByPlaceholderText("Send a message..."),
			"! git status --short",
		);
		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Run Codex shell command"));

		expect(
			await screen.findByText("Codex shell command sent"),
		).toBeInTheDocument();
		expect(
			await screen.findByText(
				"Runtime shell command sent for this thread: git status --short",
			),
		).toBeInTheDocument();
	});

	it("controls Codex realtime text from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "start_codex_realtime_text_session") {
				expect(args).toEqual({
					chatSessionId: "s1",
					content: "listen for parser notes",
				});
				return Promise.resolve({
					title: "Codex realtime text started",
					detail:
						"Started a runtime realtime text session with the current composer draft as prompt.",
				});
			}
			if (command === "append_codex_realtime_text") {
				expect(args).toEqual({
					chatSessionId: "s1",
					content: "next realtime turn",
				});
				return Promise.resolve({
					title: "Codex realtime text sent",
					detail:
						"Appended the current composer draft to the runtime realtime session.",
				});
			}
			if (command === "stop_codex_realtime_session") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex realtime stopped",
					detail: "Requested runtime realtime stop for this thread.",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const composer = screen.getByPlaceholderText("Send a message...");
		await user.type(composer, "listen for parser notes");
		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Start Codex realtime text"));
		expect(
			await screen.findByText("Codex realtime text started"),
		).toBeInTheDocument();

		await user.type(composer, "next realtime turn");
		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Append Codex realtime text"));
		expect(
			await screen.findByText("Codex realtime text sent"),
		).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Stop Codex realtime"));
		expect(
			await screen.findByText("Codex realtime stopped"),
		).toBeInTheDocument();
	});

	it("opens Codex account usage from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command) => {
			if (command === "read_codex_account_status") {
				return Promise.resolve({
					title: "Codex account usage",
					detail:
						"Account\nRequires OpenAI auth: no\nSigned in account: chatgpt\nEmail: dev@example.com\nPlan: pro\n\nToken usage\nLifetime tokens: 12,345\n\nRate limits\n- codex\n  Primary: 50% used",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex account usage"));

		expect(await screen.findByText("Codex account usage")).toBeInTheDocument();
		expect(
			await screen.findByText(/Signed in account: chatgpt/),
		).toBeInTheDocument();
		expect(
			await screen.findByText(/Lifetime tokens: 12,345/),
		).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith(
			"read_codex_account_status",
			undefined,
		);
	});

	it("opens Codex account usage from the command palette", async () => {
		mockInvoke.mockImplementation((command) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "codex_account_usage",
						label: "Codex account usage",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "read_codex_account_status") {
				return Promise.resolve({
					title: "Codex account usage",
					detail:
						"Signed in account: chatgpt user@example.com\nLifetime tokens: 12,345",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
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
			expect(mockInvoke).toHaveBeenCalledWith("present_agent_command_palette", {
				request: {
					hasActiveSession: true,
					sessionCount: 0,
					backendId: "codex",
				},
			});
		});
		fireEvent.click(await screen.findByText("Codex account usage"));

		expect(await screen.findByText("Codex account usage")).toBeInTheDocument();
		expect(
			await screen.findByText(/Signed in account: chatgpt/),
		).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith(
			"read_codex_account_status",
			undefined,
		);
	});

	it("opens Codex hooks from the command palette", async () => {
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "codex_hooks",
						label: "Codex hooks",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "read_codex_hooks_report") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve({
					title: "Codex hooks",
					detail: "Hooks: 2 total\npreToolUse: trusted",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
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
			expect(mockInvoke).toHaveBeenCalledWith("present_agent_command_palette", {
				request: {
					hasActiveSession: true,
					sessionCount: 0,
					backendId: "codex",
				},
			});
		});
		fireEvent.click(await screen.findByText("Codex hooks"));

		expect(await screen.findByText("Codex hooks")).toBeInTheDocument();
		expect(await screen.findByText(/Hooks: 2 total/)).toBeInTheDocument();
		expect(mockInvoke).toHaveBeenCalledWith("read_codex_hooks_report", {
			worktreePath: "/repo",
		});
	});

	it("opens Codex goal from the command palette", async () => {
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "codex_goal",
						label: "Codex goal",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "read_codex_thread_goal") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex goal",
					detail:
						"Status: active\nTokens: 1200 / 50000\nElapsed: 30s\nFinish parity",
					goal: {
						objective: "Finish parity",
						status: "active",
						tokenBudget: 50000,
						tokensUsed: 1200,
						timeUsedSeconds: 30,
					},
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
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
			expect(mockInvoke).toHaveBeenCalledWith("present_agent_command_palette", {
				request: {
					hasActiveSession: true,
					sessionCount: 0,
					backendId: "codex",
				},
			});
		});
		fireEvent.click(await screen.findByText("Codex goal"));

		const objective = await screen.findByLabelText("Codex goal objective");
		expect(objective).toHaveValue("Finish parity");
		expect(mockInvoke).toHaveBeenCalledWith("read_codex_thread_goal", {
			chatSessionId: "s1",
		});
	});

	it("runs Codex runtime thread actions from the command palette", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_agent_shortcut_settings") {
				return Promise.resolve(defaultAgentShortcuts);
			}
			if (command === "present_agent_command_palette") {
				return Promise.resolve([
					{
						id: "codex_compact_context",
						label: "Compact context",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
					{
						id: "codex_clean_background_terminals",
						label: "Clean Codex background terminals",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
					{
						id: "codex_shell_command",
						label: "Run Codex shell command",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
					{
						id: "codex_review_uncommitted_changes",
						label: "Codex review uncommitted changes",
						shortcut: "",
						alternateShortcut: null,
						enabled: true,
					},
				]);
			}
			if (command === "compact_agent_context") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Context compaction started",
					detail: "Requested runtime compaction for codex.",
				});
			}
			if (command === "clean_codex_background_terminals") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex background terminals cleaned",
					detail:
						"Requested runtime cleanup for this thread's background terminals.",
				});
			}
			if (command === "run_codex_shell_command") {
				expect(args).toEqual({
					chatSessionId: "s1",
					content: "! git status --short",
				});
				return Promise.resolve({
					title: "Codex shell command sent",
					detail:
						"Runtime shell command sent for this thread: git status --short",
					command: "git status --short",
				});
			}
			if (command === "start_codex_uncommitted_changes_review") {
				expect(args).toEqual({
					chatSessionId: "s1",
					targetType: "uncommittedChanges",
					targetValue: undefined,
				});
				return Promise.resolve({
					title: "Codex review started",
					detail: "Runtime review started for uncommitted changes.",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
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

		await user.type(
			screen.getByPlaceholderText("Send a message..."),
			"! git status --short",
		);

		fireEvent.keyDown(window, { key: "k", metaKey: true });
		await user.click(await screen.findByText("Compact context"));
		expect(
			await screen.findByText("Context compaction started"),
		).toBeInTheDocument();

		fireEvent.keyDown(window, { key: "k", metaKey: true });
		await user.click(
			await screen.findByText("Clean Codex background terminals"),
		);
		expect(
			await screen.findByText("Codex background terminals cleaned"),
		).toBeInTheDocument();

		fireEvent.keyDown(window, { key: "k", metaKey: true });
		await user.click(await screen.findByText("Run Codex shell command"));
		expect(
			await screen.findByText("Codex shell command sent"),
		).toBeInTheDocument();

		fireEvent.keyDown(window, { key: "k", metaKey: true });
		await user.click(
			await screen.findByText("Codex review uncommitted changes"),
		);
		expect(await screen.findByText("Codex review started")).toBeInTheDocument();
	});

	it("manages a Codex thread goal from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "read_codex_thread_goal") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex goal",
					detail:
						"Status: active\nTokens: 1200 / 50000\nElapsed: 30s\nFinish parity",
					goal: {
						objective: "Finish parity",
						status: "active",
						tokenBudget: 50000,
						tokensUsed: 1200,
						timeUsedSeconds: 30,
					},
				});
			}
			if (command === "set_codex_thread_goal") {
				expect(args).toEqual({
					chatSessionId: "s1",
					objective: "Finish parity and tests",
					status: "paused",
					tokenBudget: 50000,
				});
				return Promise.resolve({
					title: "Codex goal updated",
					detail:
						"Status: paused\nTokens: 1200 / 50000\nElapsed: 31s\nFinish parity and tests",
					goal: {
						objective: "Finish parity and tests",
						status: "paused",
						tokenBudget: 50000,
						tokensUsed: 1200,
						timeUsedSeconds: 31,
					},
				});
			}
			if (command === "clear_codex_thread_goal") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex goal cleared",
					detail: "No Codex goal is set for this thread.",
					goal: null,
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex goal"));

		const objective = await screen.findByLabelText("Codex goal objective");
		expect(objective).toHaveValue("Finish parity");
		await user.clear(objective);
		await user.type(objective, "Finish parity and tests");
		await user.click(await screen.findByText("Pause"));

		expect(await screen.findByText("Codex goal updated")).toBeInTheDocument();
		expect(await screen.findByText(/Status: paused/)).toBeInTheDocument();

		await user.click(await screen.findByText("Clear"));

		expect(await screen.findByText("Codex goal cleared")).toBeInTheDocument();
	});

	it("renders Codex goal progress row and updates runtime goal status", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "set_codex_thread_goal") {
				expect(args).toEqual({
					chatSessionId: "s1",
					objective: "Finish parity",
					status: "paused",
					tokenBudget: 50000,
				});
				return Promise.resolve({
					title: "Codex goal updated",
					detail:
						"Status: paused\nTokens: 1200 / 50000\nElapsed: 31s\nFinish parity",
					goal: {
						objective: "Finish parity",
						status: "paused",
						tokenBudget: 50000,
						tokensUsed: 1200,
						timeUsedSeconds: 31,
					},
				});
			}
			if (command === "clear_codex_thread_goal") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex goal cleared",
					detail: "No Codex goal is set for this thread.",
					goal: null,
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			getSessionCodexGoal: vi.fn().mockReturnValue({
				objective: "Finish parity",
				status: "active",
				tokenBudget: 50000,
				tokensUsed: 1200,
				timeUsedSeconds: 30,
			}),
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(await screen.findByText("Goal")).toBeInTheDocument();
		expect(screen.getByText("Finish parity")).toBeInTheDocument();
		expect(screen.getByText(/Tokens 1200/)).toBeInTheDocument();

		await user.click(screen.getByText("Pause"));

		expect(await screen.findByText("Codex goal updated")).toBeInTheDocument();

		await user.click(screen.getByText("Clear"));

		expect(await screen.findByText("Codex goal cleared")).toBeInTheDocument();
	});

	it("starts Codex review and opens runtime inventory actions from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "start_codex_uncommitted_changes_review") {
				if (
					args &&
					typeof args === "object" &&
					"targetType" in args &&
					args.targetType === "baseBranch"
				) {
					expect(args).toEqual({
						chatSessionId: "s1",
						targetType: "baseBranch",
						targetValue: "main",
					});
					return Promise.resolve({
						title: "Codex review started",
						detail: "Runtime review started for base branch main.",
					});
				}
				expect(args).toEqual({
					chatSessionId: "s1",
					targetType: "uncommittedChanges",
					targetValue: undefined,
				});
				return Promise.resolve({
					title: "Codex review started",
					detail: "Runtime review started for uncommitted changes.",
				});
			}
			if (command === "read_codex_hooks_report") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve({
					title: "Codex hooks",
					detail:
						"Hooks: 2 total, 1 enabled, 1 disabled\n- preToolUse command [project, trusted, enabled]",
				});
			}
			if (command === "read_codex_realtime_voices_report") {
				expect(args).toBeUndefined();
				return Promise.resolve({
					title: "Codex realtime voices",
					detail:
						"Codex realtime voices are runtime-provided and experimental.\nDefault v1: alloy\nDefault v2: marin\nv1 voices: alloy, echo\nv2 voices: marin, verse\nTransport methods: thread/realtime/start, thread/realtime/appendAudio, thread/realtime/appendText, thread/realtime/stop\nDesktop audio capture and WebRTC session lifecycle are not connected yet.",
				});
			}
			if (command === "read_codex_thread_history_report") {
				expect(args).toEqual({ worktreePath: "/repo", query: null });
				return Promise.resolve({
					title: "Codex thread history",
					detail:
						"Codex runtime threads: 1 latest thread(s) in /repo\n- Fix parser bug [idle, appServer, updated 1780000000]\n  thr_123",
				});
			}
			if (command === "read_codex_thread_transcript_report") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex thread transcript",
					detail:
						"Thread: Fix parser bug\nID: thr_123\nTurns: 1, persisted item(s): 2\n\nTurn 1 [completed] turn_1\n- User: Fix parser\n- Agent: Done",
				});
			}
			if (command === "read_codex_permission_profiles") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve([
					{ id: ":workspace", description: "Workspace write" },
					{ id: "readonly", description: null },
				]);
			}
			if (command === "set_codex_permission_profile") {
				expect(args).toEqual({
					chatSessionId: "s1",
					permissionProfileId: ":workspace",
				});
				return Promise.resolve(null);
			}
			if (command === "read_codex_mcp_status_report") {
				expect(args).toEqual({ chatSessionId: "s1" });
				return Promise.resolve({
					title: "Codex MCP status",
					detail:
						"MCP servers: 1, tools: 3, resources: 0, templates: 0\n- docs: auth=oAuth, tools=3",
				});
			}
			if (command === "read_codex_runtime_config_report") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve({
					title: "Codex runtime config",
					detail:
						"Effective Codex config for /repo\nModel: gpt-5-codex\nApproval policy: on-request\n\nLayers: 2\n- user /home/dev/.codex/config.toml v1\n\nRequirements\nNo requirements configured.",
				});
			}
			if (command === "read_codex_runtime_capabilities_report") {
				expect(args).toEqual({ chatSessionId: "s1", worktreePath: "/repo" });
				return Promise.resolve({
					title: "Codex runtime capabilities",
					detail:
						"Model provider capabilities\n- Web search: yes\n\nApps/connectors: 1 total, 1 enabled, 1 accessible\n\nPlugins: 1 total, 1 installed, 1 enabled, 1 marketplace(s), 0 featured, 0 load error(s)",
				});
			}
			return Promise.resolve([]);
		});
		mockUseAgentChat({
			selectedBackendId: "codex",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "codex",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(
			await screen.findByText("Codex review uncommitted changes"),
		);

		expect(await screen.findByText("Codex review started")).toBeInTheDocument();
		expect(
			await screen.findByText(/Runtime review started/),
		).toBeInTheDocument();

		const promptSpy = vi.spyOn(window, "prompt").mockReturnValue("main");
		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex review base branch"));
		expect(promptSpy).toHaveBeenCalledWith("Base branch");
		expect(await screen.findByText(/base branch main/)).toBeInTheDocument();
		promptSpy.mockRestore();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex hooks"));

		expect(await screen.findByText("Codex hooks")).toBeInTheDocument();
		expect(await screen.findByText(/Hooks: 2 total/)).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex realtime voices"));

		expect(
			await screen.findByText("Codex realtime voices"),
		).toBeInTheDocument();
		expect(await screen.findByText(/Default v2: marin/)).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex thread history"));

		expect(await screen.findByText("Codex thread history")).toBeInTheDocument();
		expect(
			await screen.findByText(/Codex runtime threads: 1/),
		).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex thread transcript"));

		expect(
			await screen.findByText("Codex thread transcript"),
		).toBeInTheDocument();
		expect(await screen.findByText(/Turns: 1/)).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex permission profiles"));

		expect(
			await screen.findByText("Codex permission profiles: 2"),
		).toBeInTheDocument();
		await user.click(await screen.findByText(":workspace"));
		expect(
			await screen.findByText("Codex permission profile applied"),
		).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex MCP status"));

		expect(await screen.findByText("Codex MCP status")).toBeInTheDocument();
		expect(await screen.findByText(/MCP servers: 1/)).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex runtime config"));

		expect(await screen.findByText("Codex runtime config")).toBeInTheDocument();
		expect(
			await screen.findByText(/Effective Codex config for \/repo/),
		).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Codex runtime capabilities"));

		expect(
			await screen.findByText("Codex runtime capabilities"),
		).toBeInTheDocument();
		expect(await screen.findByText(/Apps\/connectors: 1/)).toBeInTheDocument();
	});

	it("hides Codex runtime context compaction for non-Codex sessions", async () => {
		const user = userEvent.setup();
		mockUseAgentChat({
			selectedBackendId: "claude",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [],
				state: "idle",
				createdAt: 1000,
				updatedAt: 1001,
				permissionMode: "edit",
				backendId: "claude",
			},
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));

		expect(screen.queryByText("Compact context")).not.toBeInTheDocument();
		expect(
			screen.queryByText("Codex review uncommitted changes"),
		).not.toBeInTheDocument();
		expect(screen.queryByText("Codex goal")).not.toBeInTheDocument();
		expect(screen.queryByText("Codex account usage")).not.toBeInTheDocument();
		expect(screen.queryByText("Codex thread history")).not.toBeInTheDocument();
		expect(
			screen.queryByText("Codex thread transcript"),
		).not.toBeInTheDocument();
		expect(screen.queryByText("Codex hooks")).not.toBeInTheDocument();
		expect(screen.queryByText("Codex realtime voices")).not.toBeInTheDocument();
		expect(
			screen.queryByText("Codex permission profiles"),
		).not.toBeInTheDocument();
		expect(screen.queryByText("Codex MCP status")).not.toBeInTheDocument();
		expect(screen.queryByText("Codex runtime config")).not.toBeInTheDocument();
		expect(
			screen.queryByText("Codex runtime capabilities"),
		).not.toBeInTheDocument();
	});

	it("opens config diagnostics from the agent actions menu", async () => {
		const user = userEvent.setup();
		mockInvoke.mockImplementation((command, args) => {
			if (command === "build_agent_debug_config_report") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve("Config layers\n- Releash config: present");
			}
			if (command === "build_agent_doctor_report") {
				expect(args).toEqual({ worktreePath: "/repo" });
				return Promise.resolve({
					title: "Doctor: all checks passed",
					detail: "OK Worktree: /repo\nOK Git repository: present",
					okCount: 2,
					warningCount: 0,
					errorCount: 0,
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
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Debug config"));

		expect(await screen.findByText("Debug config")).toBeInTheDocument();
		expect(
			await screen.findByText(/Releash config: present/),
		).toBeInTheDocument();

		await user.click(screen.getByLabelText("Agent actions"));
		await user.click(await screen.findByText("Doctor"));

		expect(
			await screen.findByText("Doctor: all checks passed"),
		).toBeInTheDocument();
		expect(
			await screen.findByText(/Git repository: present/),
		).toBeInTheDocument();
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

	it("enables Agent selector in the input for an empty active session", () => {
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
			backends: [
				{ id: "claude", name: "Claude", available: true, availableModels: [] },
				{ id: "codex", name: "Codex", available: true, availableModels: [] },
			],
			selectedBackendId: "claude",
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(screen.getByTestId("backend-selector-trigger")).toBeEnabled();
	});

	it("disables Agent selector after the active session has messages", () => {
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
			backends: [
				{ id: "claude", name: "Claude", available: true, availableModels: [] },
				{ id: "codex", name: "Codex", available: true, availableModels: [] },
			],
			selectedBackendId: "claude",
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		expect(screen.getByTestId("backend-selector-trigger")).toBeDisabled();
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

	it("collapses and expands thinking content", () => {
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
		expect(screen.getByText("private reasoning")).toBeInTheDocument();
		fireEvent.click(toggle);
		expect(screen.queryByText("private reasoning")).toBeNull();
		fireEvent.click(toggle);
		expect(screen.getByText("private reasoning")).toBeInTheDocument();
	});

	it("toggles all thinking content from the transcript control", () => {
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
								notificationType: "hook",
								status: "error",
								label: "pre-commit (PromptSubmit)",
								hookId: "hook-1",
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
		const el = screen.getByText(/pre-commit/);
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

		fireEvent.keyDown(window, { key: "g", metaKey: true });

		const search = await screen.findByLabelText("Search sessions");
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
