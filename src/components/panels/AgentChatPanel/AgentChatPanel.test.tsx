import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// jsdom does not implement scrollIntoView
Element.prototype.scrollIntoView = vi.fn();

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

const mockInvoke = vi.fn().mockResolvedValue([]);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

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
vi.mock("@/hooks/useAgentChat", () => ({
	useAgentChat: (...args: unknown[]) => useAgentChatMock(...args),
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
	useAgentChatMock.mockReturnValue({
		sessions,
		orderedSessions,
		closedSessions: [],
		activeSession: null,
		isStreaming: false,
		error: null,
		pendingPermission: null,
		sessionAgentStates: new Map(),
		sendMessage: vi.fn(),
		interrupt: vi.fn(),
		selectSession: vi.fn(),
		refreshSessions: vi.fn(),
		refreshClosedSessions: vi.fn(),
		closeSession: vi.fn(),
		restoreSession: vi.fn(),
		createNewSession: vi.fn(),
		reorderSessions: vi.fn(),
		setPermissionMode: vi.fn(),
		respondPermission: vi.fn(),
		permissionMode: "acceptEdits",
		availableModels: [],
		selectedModel: null,
		setModel: vi.fn(),
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

	it("renders message input", () => {
		mockUseAgentChat();
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);
		expect(screen.getByTestId("message-input")).toBeDefined();
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
		fireEvent.click(screen.getByText("Fix bug"));
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

	it("reflects permissionMode plan in ModeSelector trigger label", () => {
		mockUseAgentChat({
			permissionMode: "plan",
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
			"Plan",
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

	it("hides thinking and shows text when text arrives after thinking", () => {
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
		expect(screen.queryByTestId("thinking-indicator")).toBeNull();
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
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
	it("cycles mode on Shift+Tab in textarea", () => {
		const setPermissionMode = vi.fn();
		mockUseAgentChat({
			permissionMode: "acceptEdits",
			setPermissionMode,
		});
		render(
			<AgentChatPanel
				worktreePath="/repo"
				registerDropZone={mockRegisterDropZone}
			/>,
		);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		// acceptEdits (index 0) → default (index 1)
		expect(setPermissionMode).toHaveBeenCalledWith("default");
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
});

describe("AgentChatPanel image drag and drop", () => {
	it("shows drop overlay on dragover with files and hides on dragleave", () => {
		mockUseAgentChat();
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
		mockUseAgentChat();
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
		mockUseAgentChat();
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
