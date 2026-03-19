import { render, screen } from "@testing-library/react";
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
	useAgentChatMock.mockReturnValue({
		sessions: [],
		activeSession: null,
		isStreaming: false,
		error: null,
		pendingPermission: null,
		sendMessage: vi.fn(),
		interrupt: vi.fn(),
		selectSession: vi.fn(),
		refreshSessions: vi.fn(),
		clearActiveSession: vi.fn(),
		...overrides,
	});
}

describe("AgentChatPanel", () => {
	it("renders empty state when no active session", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("agent-chat-panel")).toBeDefined();
		expect(
			screen.getByText(
				"Start a conversation or select a session from the sidebar.",
			),
		).toBeDefined();
	});

	it("renders session list", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("session-list")).toBeDefined();
	});

	it("renders message input", () => {
		mockUseAgentChat();
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("message-input")).toBeDefined();
	});
});

describe("AgentChatPanel plan mode", () => {
	it("displays plan mode indicator when permissionMode is plan", () => {
		mockUseAgentChat({
			permissionMode: "plan",
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{
						id: "m2",
						role: "agent",
						content: "Planning...",
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("plan-mode-indicator")).toBeDefined();
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
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{ id: "m2", role: "agent", content: "editing...", timestamp: 1001 },
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
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{ id: "m2", role: "agent", content: "working...", timestamp: 1001 },
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

	it("reflects permissionMode plan in ModeSelector by showing Plan as active", () => {
		mockUseAgentChat({
			permissionMode: "plan",
			setPermissionMode: vi.fn(),
			respondPermission: vi.fn(),
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);

		const planButton = screen.getByText("Plan");
		expect(planButton).toHaveAttribute("data-active", "true");
	});
});

describe("AgentChatPanel ThinkingIndicator", () => {
	it("shows waiting indicator when streaming with empty agent content and no thinking", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{ id: "m2", role: "agent", content: "", timestamp: 1001 },
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("waiting-indicator")).toBeDefined();
		expect(screen.getByText("Waiting...")).toBeDefined();
		// Empty content StreamMessage should NOT be shown
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
	});

	it("shows ThinkingIndicator with content during thinking phase", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{
						id: "m2",
						role: "agent",
						content: "",
						thinking: "Let me think about this...",
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("thinking-indicator")).toBeDefined();
		expect(screen.getByTestId("thinking-toggle")).toBeDefined();
		// Empty content StreamMessage should NOT be shown during thinking phase
		expect(screen.queryByTestId("stream-message-agent")).toBeNull();
	});

	it("shows both ThinkingIndicator and StreamMessage when text arrives after thinking", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{
						id: "m2",
						role: "agent",
						content: "I am responding...",
						thinking: "Let me think...",
						timestamp: 1001,
					},
				],
				state: "active",
				createdAt: 1000,
				updatedAt: 1000,
			},
		});
		render(<AgentChatPanel worktreePath="/repo" />);
		expect(screen.getByTestId("thinking-indicator")).toBeDefined();
		expect(screen.getByTestId("stream-message-agent")).toBeDefined();
	});

	it("shows StreamMessage but not ThinkingIndicator when streaming with no thinking", () => {
		mockUseAgentChat({
			isStreaming: true,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{
						id: "m2",
						role: "agent",
						content: "I am responding...",
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
		expect(screen.queryByTestId("thinking-indicator")).toBeNull();
	});

	it("does not show ThinkingIndicator when streaming is finished and no thinking", () => {
		mockUseAgentChat({
			isStreaming: false,
			activeSession: {
				id: "s1",
				worktreePath: "/repo",
				messages: [
					{ id: "m1", role: "human", content: "hello", timestamp: 1000 },
					{
						id: "m2",
						role: "agent",
						content: "Done.",
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
		expect(screen.queryByTestId("thinking-indicator")).toBeNull();
	});
});
