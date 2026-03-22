import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue([]),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

const useAgentChatMock = vi.fn();
vi.mock("@/hooks/useAgentChat", () => ({
	useAgentChat: (...args: unknown[]) => useAgentChatMock(...args),
}));

// Must import after mocks
const { AgentChatPanel } = await import("./AgentChatPanel");

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
		...overrides,
	});
}

describe("AgentChatPanel", () => {
	it("renders empty state when no active session", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("agent-chat-panel")).toBeDefined();
	});

	it("renders message input", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
		fireEvent.click(screen.getByLabelText("Close Hello"));
		expect(closeSession).toHaveBeenCalledWith("s1");
	});

	it("calls createNewSession when + button is clicked", () => {
		const createNewSession = vi.fn();
		mockUseAgentChat({ createNewSession });
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
		fireEvent.click(screen.getByText("Fix bug"));
		expect(selectSession).toHaveBeenCalledWith("s2");
	});
});

describe("AgentChatPanel agent state reflection", () => {
	it("shows Waiting when isStreaming with pendingPermission", () => {
		mockUseAgentChat({
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
		render(<AgentChatPanel worktreePath="/repo" />);

		expect(screen.getByTestId("agent-state-indicator")).toHaveTextContent(
			/waiting/i,
		);
	});

	it("shows Running when isStreaming without pendingPermission", () => {
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
						parts: [{ type: "text", content: "working..." }],
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);

		expect(screen.getByTestId("agent-state-indicator")).toHaveTextContent(
			/running/i,
		);
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
		render(<AgentChatPanel worktreePath="/repo" />);

		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Plan",
		);
	});
});

describe("AgentChatPanel shimmer placeholder", () => {
	it("shows shimmer when streaming with empty agent parts", () => {
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("shimmer-placeholder")).toBeDefined();
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
	});

	it("shows shimmer during thinking phase while streaming", () => {
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("shimmer-placeholder")).toBeDefined();
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.queryByTestId("thinking-indicator")).toBeNull();
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
	});

	it("shows StreamMessage without shimmer when streaming with no thinking", () => {
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
		expect(screen.queryByTestId("shimmer-placeholder")).toBeNull();
	});

	it("hides thinking part when streaming is finished", () => {
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
		expect(screen.queryByTestId("shimmer-placeholder")).toBeNull();
	});

	it("shows shimmer when last part is tool_use waiting for result", () => {
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
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("shimmer-placeholder")).toBeDefined();
	});
});

describe("AgentChatPanel Shift+Tab mode cycle", () => {
	it("cycles mode on Shift+Tab in textarea", () => {
		const setPermissionMode = vi.fn();
		mockUseAgentChat({
			permissionMode: "acceptEdits",
			setPermissionMode,
		});
		render(<AgentChatPanel worktreePath="/repo" />);

		const textarea = screen.getByPlaceholderText("Send a message...");
		fireEvent.keyDown(textarea, { key: "Tab", shiftKey: true });
		// acceptEdits (index 0) → default (index 1)
		expect(setPermissionMode).toHaveBeenCalledWith("default");
	});
});

describe("AgentChatPanel session history", () => {
	it("renders history button", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByLabelText("Session history")).toBeDefined();
	});

	it("shows 'No closed sessions' when closedSessions is empty", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
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
		render(<AgentChatPanel worktreePath="/repo" />);
		fireEvent.click(screen.getByLabelText("Session history"));
		expect(screen.getByText("Old conversation")).toBeDefined();
		fireEvent.click(screen.getByText("Old conversation"));
		expect(restoreSession).toHaveBeenCalledWith("closed-1");
	});
});
