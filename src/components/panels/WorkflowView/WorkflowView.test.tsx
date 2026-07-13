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
	WorkspaceWorkflowStepDetail,
} from "@/types/workspace-tree";

const useWorkspaceWorkflowStepDetailMock = vi.fn();
const submitWorkspaceWorkflowStepActionMock = vi.fn();

vi.mock("@/hooks/useWorkspaceWorkflowStepDetail", () => ({
	useWorkspaceWorkflowStepDetail: (input: unknown) =>
		useWorkspaceWorkflowStepDetailMock(input),
	submitWorkspaceWorkflowStepAction: (input: unknown) =>
		submitWorkspaceWorkflowStepActionMock(input),
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
	kind: "workflowStep",
	worktreePath: "/repo",
	runId: "run-1",
	stepId: "run-1:review:1",
	stepName: "review",
};

function stepDetail(
	overrides: Partial<WorkspaceWorkflowStepDetail> = {},
): WorkspaceWorkflowStepDetail {
	return {
		kind: "step",
		id: "run-1:review:1",
		runId: "run-1",
		worktreePath: "/repo",
		title: "review",
		status: "running",
		stepType: "fanout",
		updatedAt: 1000,
		runIndex: 1,
		sessions: [
			{
				kind: "session",
				id: "session-a",
				worktreePath: "/repo",
				title: "Pane A",
				state: "active",
				updatedAt: 1000,
				workflowStepSession: true,
				stepName: "review-a",
				runIndex: 1,
				agentState: "running",
			},
			{
				kind: "session",
				id: "session-b",
				worktreePath: "/repo",
				title: "Pane B",
				state: "active",
				updatedAt: 1001,
				workflowStepSession: true,
				stepName: "review-b",
				runIndex: 1,
				agentState: "waiting",
			},
		],
		...overrides,
	};
}

function stepDetailState(overrides: Partial<WorkspaceWorkflowStepDetail> = {}) {
	return { detail: stepDetail(overrides), loading: false, error: null };
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
	const grid = screen.getByTestId("workflow-step-grid");
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
		...stepDetail().sessions[0],
		id: `session-${index + 1}`,
		title: `Pane ${index + 1}`,
		stepName: `review-${index + 1}`,
		updatedAt: 1_000 + index,
	}));
}

describe("WorkflowView", () => {
	beforeEach(() => {
		useWorkspaceWorkflowStepDetailMock.mockReset();
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(stepDetailState());
		submitWorkspaceWorkflowStepActionMock.mockReset();
		submitWorkspaceWorkflowStepActionMock.mockResolvedValue(stepDetail());
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

	it("renders the selected workflow Step as an equal grid of Session panes", async () => {
		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		const { container, grid } = triggerWorkflowGridResize(700, 800);

		expect(useWorkspaceWorkflowStepDetailMock).toHaveBeenCalledWith({
			worktreePath: "/repo",
			runId: "run-1",
			stepId: "run-1:review:1",
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
		const tiles = screen.getAllByTestId("workflow-step-grid-tile");
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
		expect(screen.queryByTestId("workflow-step-tab-list")).toBeNull();
		expect(screen.queryByTestId("workflow-step-detail")).toBeNull();
		expect(screen.queryByText("Event log")).toBeNull();
		expect(screen.queryByRole("button", { name: /Close tab/ })).toBeNull();
	});

	it("lays out four Step sessions as two fixed-height rows", async () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({ sessions: gridSessions(4) }),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		const { grid } = triggerWorkflowGridResize(700, 800);

		await waitFor(() => {
			expect(grid).toHaveStyle({
				gridTemplateColumns: "repeat(2, minmax(320px, 1fr))",
				gridTemplateRows: "repeat(2, 388px)",
			});
		});
		expect(screen.getAllByTestId("workflow-step-grid-tile")).toHaveLength(4);
	});

	it("renders an empty state inside the grid for a Step without sessions", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({ sessions: [] }),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(
			screen.getByText("No agent conversation for this node."),
		).toBeInTheDocument();
		expect(screen.getAllByTestId("workflow-step-grid-tile")).toHaveLength(1);
		expect(screen.queryByTestId(/^bound-session-chat-/)).toBeNull();
	});

	it("renders the Step type from the Rust DTO instead of deriving it from status or session count", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({
				status: "completed",
				stepType: "session",
				sessions: [stepDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.getByText("session")).toBeInTheDocument();
		expect(screen.queryByText("agent")).toBeNull();
	});

	it("shows only Approve while an approval-gated session is waiting", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [stepDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.getByRole("button", { name: "Approve" })).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Reject" })).toBeNull();
	});

	it("does not show Approve for a waiting session without an approval gate", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({
				status: "waiting",
				sessions: [stepDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);

		expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
	});

	it("submits Approve from the Step header", async () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [stepDetail().sessions[0]],
			}),
		);

		render(<WorkflowView worktreePath="/repo" selectionRequest={selection} />);
		fireEvent.click(screen.getByRole("button", { name: "Approve" }));

		await waitFor(() => {
			expect(submitWorkspaceWorkflowStepActionMock).toHaveBeenCalledWith({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
				stepName: "review",
			});
		});
	});

	it("keeps the action error icon after closing the error popup", async () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue(
			stepDetailState({
				status: "waiting",
				canApprove: true,
				sessions: [stepDetail().sessions[0]],
			}),
		);
		submitWorkspaceWorkflowStepActionMock.mockRejectedValue(
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

	it("shows an unavailable state when no Step is selected", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue({
			detail: null,
			loading: false,
			error: null,
		});

		render(<WorkflowView worktreePath="/repo" />);

		expect(screen.getByText("Step unavailable")).toBeInTheDocument();
		expect(useWorkspaceWorkflowStepDetailMock).toHaveBeenCalledWith({
			worktreePath: null,
			runId: null,
			stepId: null,
		});
	});

	it("shows the requested Step loading and error states without stale detail", () => {
		useWorkspaceWorkflowStepDetailMock.mockReturnValue({
			detail: null,
			loading: true,
			error: null,
		});

		const { rerender } = render(
			<WorkflowView worktreePath="/repo" selectionRequest={selection} />,
		);

		expect(screen.getByText("Loading Step...")).toBeInTheDocument();
		expect(screen.queryByText("review")).toBeNull();

		useWorkspaceWorkflowStepDetailMock.mockReturnValue({
			detail: null,
			loading: false,
			error: "detail failed",
		});
		rerender(
			<WorkflowView worktreePath="/repo" selectionRequest={selection} />,
		);

		expect(screen.getByText("Step unavailable")).toBeInTheDocument();
		expect(screen.getByText("detail failed")).toBeInTheDocument();
		expect(screen.queryByText("review")).toBeNull();
	});
});
