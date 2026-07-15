import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type {
	CenterSelection,
	WorkspaceWorkflowNodeDetail,
} from "@/types/workspace-tree";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => invokeMock(...args),
}));

const useWorkspaceWorkflowNodeDetailMock = vi.fn();
const submitWorkspaceWorkflowNodeActionMock = vi.fn();

vi.mock("@/hooks/useWorkspaceWorkflowNodeDetail", () => ({
	useWorkspaceWorkflowNodeDetail: (input: unknown) =>
		useWorkspaceWorkflowNodeDetailMock(input),
	submitWorkspaceWorkflowNodeAction: (input: unknown) =>
		submitWorkspaceWorkflowNodeActionMock(input),
}));

const worktreeSessionStatusesMock =
	vi.fn<(worktreePath: string | null) => Map<string, SessionStatus>>();

vi.mock("@/hooks/useWorktreeSessionStatuses", () => ({
	useWorktreeSessionStatuses: (worktreePath: string | null) =>
		worktreeSessionStatusesMock(worktreePath),
}));

function sessionStatus(
	agentState: SessionStatus["agent_state"],
): SessionStatus {
	return {
		chat_session_id: "",
		worktree_id: "/repo",
		worktree_path: "/repo",
		pty_id: null,
		agent_state: agentState,
		turn_phase: "idle",
		session_state: "active",
		pending_permission: false,
		last_activity_at: 0,
	};
}

vi.mock("@/components/panels/AgentChatPanel", () => ({
	BoundSessionChat: ({
		sessionId,
		worktreePath,
	}: {
		sessionId: string | null;
		worktreePath: string;
	}) => (
		<div data-testid={`bound-session-chat-${sessionId}`}>
			{worktreePath}:{sessionId}
			<div data-testid="message-input" />
		</div>
	),
}));

const { WorkflowView } = await import("./WorkflowView");

let resizeObserverMocks: ResizeObserverMock[] = [];

class ResizeObserverMock {
	private readonly callback: ResizeObserverCallback;
	private readonly elements: Element[] = [];

	constructor(callback: ResizeObserverCallback) {
		this.callback = callback;
		resizeObserverMocks.push(this);
	}

	observe(element: Element) {
		this.elements.push(element);
	}

	disconnect() {
		this.elements.length = 0;
	}

	trigger() {
		const entries = this.elements.map(
			(target) => ({ target }) as ResizeObserverEntry,
		);
		this.callback(entries, this as unknown as ResizeObserver);
	}
}

const selection: CenterSelection = {
	kind: "workflowNode",
	worktreePath: "/repo",
	executionId: "execution-1",
	nodeExecutionId: "node-review-1",
	nodeName: "review",
};

function nodeDetail(
	overrides: Partial<WorkspaceWorkflowNodeDetail> = {},
): WorkspaceWorkflowNodeDetail {
	return {
		kind: "node",
		nodeExecutionId: "node-review-1",
		executionId: "execution-1",
		worktreePath: "/repo",
		title: "review",
		nodeName: "review",
		status: "running",
		executionStatus: "running",
		canStop: true,
		canResume: false,
		canAbort: true,
		nodeKind: "fanout",
		updatedAt: 1000,
		attempt: 1,
		sessions: [
			{
				kind: "session",
				id: "session-a",
				worktreePath: "/repo",
				title: "Pane A",
				state: "active",
				updatedAt: 1000,
				workflowNodeSession: true,
				nodeExecutionId: "node-review-a-1",
				nodeName: "review-a",
				attempt: 1,
				agentState: "running",
			},
			{
				kind: "session",
				id: "session-b",
				worktreePath: "/repo",
				title: "Pane B",
				state: "active",
				updatedAt: 1001,
				workflowNodeSession: true,
				nodeExecutionId: "node-review-b-1",
				nodeName: "review-b",
				attempt: 1,
				agentState: "waiting",
			},
		],
		...overrides,
	};
}

function nodeDetailState(overrides: Partial<WorkspaceWorkflowNodeDetail> = {}) {
	return { detail: nodeDetail(overrides), loading: false, error: null };
}

function setElementSize(
	element: HTMLElement,
	{ width, height }: { width: number; height: number },
) {
	Object.defineProperty(element, "clientWidth", {
		configurable: true,
		value: width,
	});
	Object.defineProperty(element, "clientHeight", {
		configurable: true,
		value: height,
	});
}

function triggerWorkflowGridResize(width: number, height: number) {
	const grid = screen.getByTestId("workflow-node-grid");
	const container = grid.parentElement as HTMLElement;
	setElementSize(container, { width, height });
	act(() => {
		for (const observer of resizeObserverMocks) {
			observer.trigger();
		}
	});
	return { container, grid };
}

function gridSessions(count: number) {
	return Array.from({ length: count }, (_, index) => ({
		...nodeDetail().sessions[0],
		id: `session-${index + 1}`,
		title: `Pane ${index + 1}`,
		nodeName: `review-${index + 1}`,
		updatedAt: 1_000 + index,
	}));
}

describe("WorkflowView", () => {
	beforeEach(() => {
		invokeMock.mockReset();
		invokeMock.mockResolvedValue(undefined);
		useWorkspaceWorkflowNodeDetailMock.mockReset();
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(nodeDetailState());
		submitWorkspaceWorkflowNodeActionMock.mockReset();
		submitWorkspaceWorkflowNodeActionMock.mockResolvedValue(nodeDetail());
		worktreeSessionStatusesMock.mockReset();
		worktreeSessionStatusesMock.mockReturnValue(
			new Map([
				["session-a", sessionStatus("running")],
				["session-b", sessionStatus("waiting")],
			]),
		);
		resizeObserverMocks = [];
		vi.stubGlobal("ResizeObserver", ResizeObserverMock);
	});

	it("renders the selected workflow node as an equal grid of session panes", async () => {
		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		const { container, grid } = triggerWorkflowGridResize(700, 800);

		expect(useWorkspaceWorkflowNodeDetailMock).toHaveBeenCalledWith({
			worktreePath: "/repo",
			executionId: "execution-1",
			nodeExecutionId: "node-review-1",
		});
		expect(screen.getByText("review")).toBeInTheDocument();
		expect(screen.getByText("fanout")).toBeInTheDocument();
		expect(container).toHaveClass("overflow-x-hidden", "overflow-y-auto");
		await waitFor(() => {
			expect(grid).toHaveStyle({
				gridTemplateColumns: "repeat(2, minmax(320px, 1fr))",
				gridTemplateRows: "repeat(1, 784px)",
			});
		});
		const tiles = screen.getAllByTestId("workflow-node-grid-tile");
		expect(tiles).toHaveLength(2);
		for (const tile of tiles) {
			expect(tile).toHaveClass("overflow-hidden");
		}
		expect(
			screen.getByTestId("bound-session-chat-session-a"),
		).toBeInTheDocument();
		expect(
			screen.getByTestId("bound-session-chat-session-b"),
		).toBeInTheDocument();
		// 各 Pane ヘッダーに Session 名と実行ステータスを表示する。
		expect(screen.getByText("Pane A")).toBeInTheDocument();
		expect(screen.getByText("Pane B")).toBeInTheDocument();
		expect(screen.getByTitle("running")).toBeInTheDocument();
		expect(screen.getByTitle("waiting")).toBeInTheDocument();
		expect(screen.queryByTestId("workflow-node-tab-list")).toBeNull();
		expect(screen.queryByTestId("workflow-node-detail")).toBeNull();
		expect(screen.queryByText("Event log")).toBeNull();
		expect(screen.queryByRole("button", { name: /Close tab/ })).toBeNull();
	});

	it("lays out four node sessions as two fixed-height rows", async () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({ sessions: gridSessions(4) }),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		const { grid } = triggerWorkflowGridResize(700, 800);

		await waitFor(() => {
			expect(grid).toHaveStyle({
				gridTemplateColumns: "repeat(2, minmax(320px, 1fr))",
				gridTemplateRows: "repeat(2, 388px)",
			});
		});
		expect(screen.getAllByTestId("workflow-node-grid-tile")).toHaveLength(4);
	});

	it("renders an empty state inside the grid for a node without sessions", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({ sessions: [] }),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(
			screen.getByText("No agent conversation for this node."),
		).toBeInTheDocument();
		expect(screen.getAllByTestId("workflow-node-grid-tile")).toHaveLength(1);
		expect(screen.queryByTestId(/^bound-session-chat-/)).toBeNull();
	});

	it("renders the node kind from the Rust DTO instead of deriving it from status or session count", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "completed",
				nodeKind: "session",
				sessions: [nodeDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.getByText("session")).toBeInTheDocument();
		expect(screen.queryByText("agent")).toBeNull();
	});

	it("shows only Approve while an approval-gated session is waiting", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [nodeDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.getByRole("button", { name: "Approve" })).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Reject" })).toBeNull();
	});

	it("does not show Approve for a waiting session without an approval gate", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "waiting",
				sessions: [nodeDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
	});

	it("submits Approve from the node header", async () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [nodeDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		fireEvent.click(screen.getByRole("button", { name: "Approve" }));

		await waitFor(() => {
			expect(submitWorkspaceWorkflowNodeActionMock).toHaveBeenCalledWith({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
				nodeName: "review",
			});
		});
	});

	it("shows the interrupted checkpoint and enables actions only from Rust DTO flags", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "aborted",
				nodeExecutionStatus: "aborted",
				executionStatus: "interrupted",
				canStop: false,
				canResume: true,
				canAbort: true,
				interruptionReason: "stop",
				resumeFromNode: "review",
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(
			screen.getByTitle("Execution status: interrupted"),
		).toHaveTextContent("execution: interrupted");
		expect(screen.getByTitle("Interrupted: stop")).toHaveTextContent("stop");
		expect(screen.getByTitle("Resume from review")).toHaveTextContent(
			"resume: review",
		);
		expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
		expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled();
		expect(screen.getByRole("button", { name: "Abort" })).toBeEnabled();
	});

	it.each([
		[
			"Stop",
			"stop_workflow",
			{ canStop: true, canResume: false, canAbort: true },
		],
		[
			"Resume",
			"resume_workflow",
			{
				executionStatus: "interrupted",
				canStop: false,
				canResume: true,
				canAbort: true,
			},
		],
		[
			"Abort",
			"abort_workflow",
			{
				executionStatus: "interrupted",
				canStop: false,
				canResume: true,
				canAbort: true,
			},
		],
	] as const)("dispatches %s through the typed Tauri command boundary", async (label, command, overrides) => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState(overrides),
		);
		const refreshEvents: Array<CustomEvent<{ worktreePath?: string }>> = [];
		const onRefresh = (event: Event) => {
			refreshEvents.push(event as CustomEvent<{ worktreePath?: string }>);
		};
		window.addEventListener("workspace-tree-refresh", onRefresh);

		try {
			render(
				<WorkflowView worktreePath="/repo" selectionRequest={selection} />,
			);
			fireEvent.click(screen.getByRole("button", { name: label }));

			await waitFor(() => {
				expect(invokeMock).toHaveBeenCalledWith(command, {
					executionId: "execution-1",
				});
			});
			expect(refreshEvents).toHaveLength(1);
			expect(refreshEvents[0].detail).toEqual({ worktreePath: "/repo" });
		} finally {
			window.removeEventListener("workspace-tree-refresh", onRefresh);
		}
	});

	it("shows NodeExecution identity, fanout coordinates, session, and artifact", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				nodeExecutionId: "ne-review-item-2",
				nodeName: "review",
				title: "review",
				nodeKind: "session",
				attempt: 2,
				sessionId: "session-item-2",
				artifact: {
					nodeName: "review",
					value: { verdict: "pass" },
					producedAt: 2_000,
				},
				fanoutParent: {
					parentNode: "parallel-review",
					parentAttempt: 1,
					itemIndex: 2,
					childIndex: 0,
				},
				sessions: [],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.getByText("ne-review-item-2")).toBeInTheDocument();
		expect(screen.getByText("session-item-2")).toBeInTheDocument();
		expect(screen.getByText("attempt 2")).toBeInTheDocument();
		expect(
			screen.getByText("parallel-review#1 · item 2 · child 0"),
		).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Artifact" }),
		).toBeInTheDocument();
		expect(screen.getByText(/"verdict": "pass"/)).toBeInTheDocument();
	});

	it("keeps the action error icon after closing the error popup", async () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue(
			nodeDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [nodeDetail().sessions[0]],
			}),
		);
		submitWorkspaceWorkflowNodeActionMock.mockRejectedValue(
			new Error("approval failed"),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		fireEvent.click(screen.getByRole("button", { name: "Approve" }));

		expect(await screen.findByText("approval failed")).toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "Close action error" }));
		await waitFor(() => {
			expect(screen.queryByText("approval failed")).toBeNull();
		});

		const errorButton = screen.getByRole("button", {
			name: "Show action error",
		});
		expect(errorButton).toBeInTheDocument();
		fireEvent.click(errorButton);
		expect(await screen.findByText("approval failed")).toBeInTheDocument();
	});

	it("shows an unavailable state when no node is selected", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue({
			detail: null,
			loading: false,
			error: null,
		});

		render(<WorkflowView worktreePath="/repo" />);

		expect(screen.getByText("Node unavailable")).toBeInTheDocument();
		expect(useWorkspaceWorkflowNodeDetailMock).toHaveBeenCalledWith({
			worktreePath: null,
			executionId: null,
			nodeExecutionId: null,
		});
	});

	it("shows the requested node loading and error states without stale detail", () => {
		useWorkspaceWorkflowNodeDetailMock.mockReturnValue({
			detail: null,
			loading: true,
			error: null,
		});

		const { rerender } = render(
			<WorkflowView worktreePath="/repo" selectionRequest={selection} />,
		);

		expect(screen.getByText("Loading Node...")).toBeInTheDocument();
		expect(screen.queryByText("review")).toBeNull();

		useWorkspaceWorkflowNodeDetailMock.mockReturnValue({
			detail: null,
			loading: false,
			error: "detail failed",
		});
		rerender(
			<WorkflowView worktreePath="/repo" selectionRequest={selection} />,
		);

		expect(screen.getByText("Node unavailable")).toBeInTheDocument();
		expect(screen.getByText("detail failed")).toBeInTheDocument();
		expect(screen.queryByText("review")).toBeNull();
	});
});
