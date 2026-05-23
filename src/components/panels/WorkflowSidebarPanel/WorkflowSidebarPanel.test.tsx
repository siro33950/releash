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
	// テスト側が `viewedStepSession` という名前で渡してきた session を `sessionsById`
	// 経由で参照可能にする互換 layer（旧 API 経路の test を最小書き換えで残すため）。
	const explicitSessions =
		(overrides.sessionsById as Record<string, { id: string }>) ?? {};
	const legacyStepSession = (overrides.viewedStepSession ?? null) as {
		id: string;
	} | null;
	const sessionsById: Record<string, unknown> = { ...explicitSessions };
	if (legacyStepSession) {
		sessionsById[legacyStepSession.id] = legacyStepSession;
	}
	// `loadStepSession` という旧 API 名で test が spy を渡してきた場合は loadSession に
	// 振り替える。loadSession の戻り値仕様差（ChatSession | null）も吸収する。
	const loadSession =
		(overrides.loadSession as ((id: string) => Promise<unknown>) | undefined) ??
		(overrides.loadStepSession as
			| ((id: string) => Promise<unknown>)
			| undefined) ??
		vi.fn().mockResolvedValue(null);
	useAgentChatContextMock.mockReturnValue({
		sessions: [],
		orderedSessions: [],
		closedSessions: [],
		activeSession: null,
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
		loadSession,
		getSessionById: vi.fn(
			(id: string | null | undefined) =>
				(id != null && sessionsById[id]) || null,
		),
		registerViewableSession: vi.fn().mockReturnValue(() => {}),
		getSessionTurnPhase: vi.fn().mockReturnValue("idle"),
		getSessionSelectedModel: vi.fn().mockReturnValue(null),
		...overrides,
		// sessionsById / viewedStepSession の上書きは互換 layer 内で吸収済みのため除く
		viewedStepSession: undefined,
		sessionsById: undefined,
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

	it("renders ChatSessionView when an approval step that owns a session is selected", async () => {
		// approval step は被承認 agent step の current_session_id を引き継ぎ、
		// approval chat 経由で対話できる（engine: validate_approval_chat_instruction /
		// send_workflow_approval_chat_message）。WorkflowSidebarPanel は nodeType を
		// 見ず session 有無だけで ChatUI を出すこと。
		useWorkflowStateMock.mockReturnValue({
			workflowState: workflowStateFixture({
				state: { type: "waiting_approval" },
				currentSessionId: "approval-session-1",
				workflowDefinition: {
					name: "wf",
					description: "",
					builtin: false,
					nodes: [
						{ name: "step-1", type: "approval", instruction: "i", rules: [] },
					],
				},
				stepStates: { "step-1": "waiting_approval" },
			}),
		});
		useWorkflowStepDetailMock.mockReturnValue({
			detail: {
				stepName: "step-1",
				nodeType: "approval",
				runIndex: 1,
				state: "waiting_approval",
				sessionId: "approval-session-1",
				input: { instruction: "approve please" },
				startedAtMs: 1_700_000_000_000,
			},
			isLoading: false,
			error: null,
		});
		const loadStepSession = vi.fn().mockResolvedValue(undefined);
		mockAgentChatContext({
			loadStepSession,
			viewedStepSession: chatSessionFixture({
				id: "approval-session-1",
				messages: [
					{
						id: "m1",
						role: "agent",
						parts: [{ type: "text", content: "ready for approval" }],
						timestamp: 0,
					},
				],
			}),
		});

		render(<WorkflowSidebarPanel worktreePath="/repo" />);

		const openButtons = await screen.findAllByLabelText("Open tab");
		fireEvent.click(openButtons[0]);

		await waitFor(() => {
			expect(loadStepSession).toHaveBeenCalledWith("approval-session-1");
		});
		await waitFor(() => {
			expect(screen.getByTestId("workflow-step-detail")).toBeInTheDocument();
		});
		// ChatUI（composer + step session 本文）が approval でも描画される
		expect(screen.getByTestId("message-input")).toBeInTheDocument();
		expect(screen.getByText("ready for approval")).toBeInTheDocument();
	});

	it("renders NewWorkflowButton kicker so the user can start a workflow", () => {
		useWorkflowStateMock.mockReturnValue({
			workflowState: workflowStateFixture(),
		});
		render(<WorkflowSidebarPanel worktreePath="/repo" />);
		// 起動経路を残すため kicker を表示する。
		expect(screen.queryByLabelText("New workflow")).not.toBeNull();
	});

	describe("tab bar", () => {
		function multiStepWorkflowFixture(): WorkflowState {
			return workflowStateFixture({
				currentStepIndex: 2,
				currentStepName: "step-3",
				currentSessionId: "chat-session-3",
				totalSteps: 3,
				stepExecutionCounts: { "step-1": 1, "step-2": 1, "step-3": 1 },
				stepStates: {
					"step-1": "completed",
					"step-2": "completed",
					"step-3": "running",
				},
				stepHistory: [
					{
						stepName: "step-1",
						completedAt: 1500,
						result: "ok",
						sessionId: "chat-session-1",
						runIndex: 1,
					},
					{
						stepName: "step-2",
						completedAt: 1600,
						result: "ok",
						sessionId: "chat-session-2",
						runIndex: 1,
					},
				],
				workflowDefinition: {
					name: "wf",
					description: "",
					builtin: false,
					nodes: [
						{ name: "step-1", type: "agent", instruction: "a", rules: [] },
						{ name: "step-2", type: "agent", instruction: "b", rules: [] },
						{ name: "step-3", type: "agent", instruction: "c", rules: [] },
					],
				},
			});
		}

		it("opens a tab on timeline click and loads the corresponding step session", async () => {
			useWorkflowStateMock.mockReturnValue({
				workflowState: multiStepWorkflowFixture(),
			});
			const loadStepSession = vi.fn().mockResolvedValue(undefined);
			mockAgentChatContext({
				loadStepSession,
				viewedStepSession: chatSessionFixture({ id: "chat-session-1" }),
			});

			render(<WorkflowSidebarPanel worktreePath="/repo" />);

			// 初期: タブバーは出ない。
			expect(screen.queryByTestId("workflow-step-tab-list")).toBeNull();

			const openButtons = await screen.findAllByLabelText("Open tab");
			fireEvent.click(openButtons[0]);

			// タブバーが現れ、1 件のタブが表示される。
			const tabList = await screen.findByTestId("workflow-step-tab-list");
			expect(tabList).toBeInTheDocument();
			await waitFor(() => {
				expect(loadStepSession).toHaveBeenCalledWith(
					expect.stringMatching(/^chat-session-/),
				);
			});
		});

		it("accumulates multiple tabs when different steps are selected", async () => {
			useWorkflowStateMock.mockReturnValue({
				workflowState: multiStepWorkflowFixture(),
			});
			mockAgentChatContext({
				viewedStepSession: chatSessionFixture({ id: "chat-session-1" }),
			});

			render(<WorkflowSidebarPanel worktreePath="/repo" />);

			const openButtons = await screen.findAllByLabelText("Open tab");
			expect(openButtons.length).toBeGreaterThanOrEqual(2);
			fireEvent.click(openButtons[0]);
			fireEvent.click(openButtons[1]);

			// 2 つのタブが両方残っていること（"Close <label>" aria-label を 2 つ持つ）。
			const closeButtons = await screen.findAllByLabelText(/^Close tab /);
			expect(closeButtons.length).toBeGreaterThanOrEqual(2);
		});

		it("switches active tab when clicking another tab in the tab bar", async () => {
			useWorkflowStateMock.mockReturnValue({
				workflowState: multiStepWorkflowFixture(),
			});
			mockAgentChatContext({
				viewedStepSession: chatSessionFixture({ id: "chat-session-1" }),
			});

			render(<WorkflowSidebarPanel worktreePath="/repo" />);

			const openButtons = await screen.findAllByLabelText("Open tab");
			fireEvent.click(openButtons[0]);
			fireEvent.click(openButtons[1]);

			// 2 番目のタブが active state になっている。
			const tabList = await screen.findByTestId("workflow-step-tab-list");
			const getTabs = () =>
				tabList.querySelectorAll('[data-slot="tabs-trigger"]');
			await waitFor(() => {
				expect(getTabs()[1].getAttribute("data-state")).toBe("active");
			});

			// 最初のタブのラベルをクリック → そちらが active になる。
			// Radix Tabs は pointerdown / mousedown 経由でも値変更を発火する。
			const firstTab = getTabs()[0] as HTMLElement;
			fireEvent.pointerDown(firstTab, { button: 0 });
			fireEvent.mouseDown(firstTab, { button: 0 });
			fireEvent.click(firstTab);
			await waitFor(() => {
				expect(getTabs()[0].getAttribute("data-state")).toBe("active");
				expect(getTabs()[1].getAttribute("data-state")).toBe("inactive");
			});
		});

		it("closes a tab when its X button is clicked and returns to placeholder when all tabs are closed", async () => {
			useWorkflowStateMock.mockReturnValue({
				workflowState: multiStepWorkflowFixture(),
			});
			const clearStepSession = vi.fn();
			mockAgentChatContext({
				clearStepSession,
				viewedStepSession: chatSessionFixture({ id: "chat-session-1" }),
			});

			render(<WorkflowSidebarPanel worktreePath="/repo" />);

			const openButtons = await screen.findAllByLabelText("Open tab");
			fireEvent.click(openButtons[0]);

			// 1 タブ開いてから × をクリック。
			const closeBtn = await screen.findByLabelText(/^Close tab /);
			fireEvent.click(closeBtn);

			// タブバーは消え、初期プレースホルダに戻る。
			await waitFor(() => {
				expect(screen.queryByTestId("workflow-step-tab-list")).toBeNull();
			});
			expect(
				screen.getByText(/Select a node in the workflow/i),
			).toBeInTheDocument();
		});

		it("clears all tabs when the workflow run identity changes", async () => {
			useWorkflowStateMock.mockReturnValue({
				workflowState: multiStepWorkflowFixture(),
			});
			mockAgentChatContext({
				viewedStepSession: chatSessionFixture({ id: "chat-session-1" }),
			});

			const { rerender } = render(
				<WorkflowSidebarPanel worktreePath="/repo" />,
			);

			const openButtons = await screen.findAllByLabelText("Open tab");
			fireEvent.click(openButtons[0]);
			expect(
				await screen.findByTestId("workflow-step-tab-list"),
			).toBeInTheDocument();

			// run を切替（executionId が変わる）。
			useWorkflowStateMock.mockReturnValue({
				workflowState: workflowStateFixture({ executionId: "exec-2" }),
			});
			rerender(<WorkflowSidebarPanel worktreePath="/repo" />);

			await waitFor(() => {
				expect(screen.queryByTestId("workflow-step-tab-list")).toBeNull();
			});
		});
	});
});
