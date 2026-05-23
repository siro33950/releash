import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/types/session";
import type { WorkflowState } from "@/types/workflow";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

// jsdom does not implement scrollIntoView (ChatSessionView 内の自動スクロール用)
Element.prototype.scrollIntoView = vi.fn();

const mockInvoke = vi.fn().mockResolvedValue([]);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

const useWorkflowStateMock = vi.fn();
vi.mock("@/hooks/useWorkflowState", () => ({
	useWorkflowState: (path: string | undefined) => useWorkflowStateMock(path),
}));

const useWorkflowStepDetailMock = vi.fn();
vi.mock("@/hooks/useWorkflowStepDetail", () => ({
	useWorkflowStepDetail: (input: {
		worktreePath: string | null;
		runId: string | null;
		nodeName: string | null;
		runIndex?: number;
	}) => useWorkflowStepDetailMock(input),
}));

// spec issues-1023: WorkflowSidebarPanel は useAgentChatContext から
// viewedStepSession を引いて ChatSessionView を描画する。
const useAgentChatContextMock = vi.fn();
vi.mock("@/contexts/AgentChatContext", () => ({
	useAgentChatContext: () => useAgentChatContextMock(),
	AgentChatProvider: ({ children }: { children: React.ReactNode }) => children,
}));

const { WorkflowSidebarPanel } = await import("./WorkflowSidebarPanel");

function workflowStateFixture(
	overrides: Partial<WorkflowState> = {},
): WorkflowState {
	return {
		executionId: "exec-1",
		workflowName: "wf",
		state: { type: "running" },
		currentStepIndex: 0,
		currentStepName: "step-1",
		currentSessionId: "chat-session-1",
		totalSteps: 1,
		stepHistory: [],
		stepExecutionCounts: { "step-1": 1 },
		stepOutputs: {},
		workflowDefinition: {
			name: "wf",
			description: "",
			builtin: false,
			nodes: [{ name: "step-1", type: "agent", instruction: "i", rules: [] }],
		},
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		stepStates: { "step-1": "running" },
		startedAt: 1000,
		updatedAt: 2000,
		...overrides,
	};
}

function chatSessionFixture(overrides: Partial<ChatSession> = {}): ChatSession {
	return {
		id: "chat-session-1",
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit",
		workflowStepSession: true,
		...overrides,
	};
}

function mockAgentChatContext(overrides: Record<string, unknown> = {}) {
	useAgentChatContextMock.mockReturnValue({
		sessions: [],
		orderedSessions: [],
		closedSessions: [],
		activeSession: null,
		viewedStepSession: null,
		isStreaming: false,
		activityStatus: null,
		error: null,
		permissionMode: "edit",
		sessionAgentStates: new Map(),
		sendMessage: vi.fn().mockResolvedValue(undefined),
		interrupt: vi.fn(),
		selectSession: vi.fn().mockResolvedValue(undefined),
		refreshSessions: vi.fn().mockResolvedValue([]),
		refreshClosedSessions: vi.fn().mockResolvedValue(undefined),
		closeSession: vi.fn().mockResolvedValue(undefined),
		restoreSession: vi.fn().mockResolvedValue(undefined),
		createNewSession: vi.fn().mockResolvedValue(undefined),
		reorderSessions: vi.fn(),
		setPermissionMode: vi.fn(),
		respondPermission: vi.fn(),
		availableModels: [],
		selectedModel: null,
		setModel: vi.fn(),
		backends: [],
		selectedBackendId: null,
		setBackend: vi.fn(),
		loadStepSession: vi.fn().mockResolvedValue(undefined),
		clearStepSession: vi.fn(),
		viewedStepSessionStreaming: false,
		viewedStepSessionActivityStatus: null,
		getSessionTurnPhase: vi.fn().mockReturnValue("idle"),
		getSessionSelectedModel: vi.fn().mockReturnValue(null),
		...overrides,
	});
}

describe("WorkflowSidebarPanel", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValue([]);
		useWorkflowStateMock.mockReset();
		useWorkflowStepDetailMock.mockReset();
		useWorkflowStepDetailMock.mockReturnValue({
			detail: {
				stepName: "step-1",
				nodeType: "agent",
				runIndex: 1,
				state: "running",
				sessionId: "chat-session-1",
				input: {
					instruction: "do something",
				},
				startedAtMs: 1_700_000_000_000,
			},
			isLoading: false,
			error: null,
		});
		useAgentChatContextMock.mockReset();
		mockAgentChatContext();
	});

	it("mounts even when workflow run is absent (history-only / empty state)", () => {
		useWorkflowStateMock.mockReturnValue({ workflowState: null });
		render(<WorkflowSidebarPanel worktreePath="/repo" />);
		expect(screen.getByTestId("workflow-sidebar-panel")).toBeInTheDocument();
	});

	it("loads the step session and renders ChatSessionView when an agent step is selected", async () => {
		useWorkflowStateMock.mockReturnValue({
			workflowState: workflowStateFixture(),
		});
		const loadStepSession = vi.fn().mockResolvedValue(undefined);
		mockAgentChatContext({
			loadStepSession,
			viewedStepSession: chatSessionFixture({
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "agent line" }],
						timestamp: 0,
					},
				],
			}),
		});

		render(<WorkflowSidebarPanel worktreePath="/repo" />);

		// 初期表示では detail / chat pane は出ない
		expect(screen.queryByTestId("workflow-step-detail")).toBeNull();
		expect(screen.queryByTestId("message-input")).toBeNull();

		// Eye toggle で step session を選択する経路（WorkflowTrace 側の SessionToggleButton 経由）
		const openButtons = await screen.findAllByLabelText("Open tab");
		fireEvent.click(openButtons[0]);

		await waitFor(() => {
			expect(loadStepSession).toHaveBeenCalledWith("chat-session-1");
		});
		await waitFor(() => {
			expect(screen.getByTestId("workflow-step-detail")).toBeInTheDocument();
		});
		// ChatSessionView 内の MessageInput（composer）が表示される
		expect(screen.getByTestId("message-input")).toBeInTheDocument();
		// step session の本文が描画される
		expect(screen.getByText("agent line")).toBeInTheDocument();
	});

	it("renders step detail fields and chat composer for an agent step", async () => {
		// spec issues-1023 L75-79: step detail は入出力・遷移結果・所要時間を表示する。
		// L81-84: agent step の対話も Workflow panel 内で読み書きできる（composer 付き）。
		useWorkflowStateMock.mockReturnValue({
			workflowState: workflowStateFixture(),
		});
		useWorkflowStepDetailMock.mockReturnValue({
			detail: {
				stepName: "step-1",
				nodeType: "agent",
				runIndex: 1,
				state: "completed",
				sessionId: "chat-session-1",
				result: "LGTM",
				structuredOutput: { verdict: "LGTM" },
				startedAtMs: Date.UTC(2024, 0, 2, 3, 4, 5),
				completedAtMs: Date.UTC(2024, 0, 2, 3, 4, 6),
				durationMs: 1000,
				input: {
					instruction: "do the work",
					previousStepName: "plan",
					previousStepStructuredOutput: { summary: "diff is ok" },
				},
			},
			isLoading: false,
			error: null,
		});
		mockAgentChatContext({
			viewedStepSession: chatSessionFixture({
				messages: [
					{
						id: "m1",
						role: "human",
						parts: [{ type: "text", content: "review please" }],
						timestamp: 0,
					},
					{
						id: "m2",
						role: "agent",
						parts: [{ type: "text", content: "looks good" }],
						timestamp: 1,
					},
				],
			}),
		});

		render(<WorkflowSidebarPanel worktreePath="/repo" />);

		const openButtons = await screen.findAllByLabelText("Open tab");
		fireEvent.click(openButtons[0]);

		const detailPane = await screen.findByTestId("workflow-step-detail");
		// 遷移結果・所要時間・structured output が表示される
		expect(detailPane).toHaveTextContent("LGTM");
		expect(detailPane).toHaveTextContent("Duration");
		expect(detailPane).toHaveTextContent("1.0 s");
		expect(detailPane).toHaveTextContent("verdict");
		expect(
			screen.getByTestId("workflow-step-detail-input"),
		).toBeInTheDocument();
		expect(detailPane).toHaveTextContent("do the work");
		expect(detailPane).toHaveTextContent("plan");
		expect(detailPane).toHaveTextContent("diff is ok");

		// chat 側で human / agent メッセージ + composer が出る
		expect(screen.getByText("review please")).toBeInTheDocument();
		expect(screen.getByText("looks good")).toBeInTheDocument();
		expect(screen.getByTestId("message-input")).toBeInTheDocument();
	});

	it("renders NewWorkflowButton kicker so the user can start a workflow", () => {
		useWorkflowStateMock.mockReturnValue({
			workflowState: workflowStateFixture(),
		});
		render(<WorkflowSidebarPanel worktreePath="/repo" />);
		// 起動経路を残すため kicker を表示する。
		expect(screen.queryByLabelText("New workflow")).not.toBeNull();
	});
});
