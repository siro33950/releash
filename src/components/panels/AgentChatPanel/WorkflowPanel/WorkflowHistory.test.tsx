import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@xyflow/react", () => ({
	ReactFlow: ({
		children,
		onNodeClick,
	}: {
		children?: React.ReactNode;
		onNodeClick?: (event: unknown, node: { id: string }) => void;
	}) => (
		// biome-ignore lint/a11y/useKeyWithClickEvents: test mock
		// biome-ignore lint/a11y/noStaticElementInteractions: test mock
		<div
			data-testid="react-flow"
			onClick={() => onNodeClick?.(null, { id: "step-1" })}
		>
			{children}
		</div>
	),
	Handle: () => <div />,
	Position: { Top: "top", Bottom: "bottom" },
}));

const { WorkflowHistory } = await import("./WorkflowHistory");

const makeMockState = (
	overrides: Partial<WorkflowState> = {},
): WorkflowState => ({
	executionId: "exec-001",
	workflowName: "test-wf",
	state: { type: "completed" },
	currentStepIndex: 0,
	currentStepName: "step-1",
	totalSteps: 1,
	stepHistory: [
		{
			stepName: "step-1",
			result: "done",
			completedAt: 2000,
			tokenUsage: { inputTokens: 100, outputTokens: 50 },
			sessionId: "session-1",
		},
	],
	stepExecutionCounts: { "step-1": 1 },
	workflowDefinition: {
		name: "test-wf",
		description: "",
		builtin: false,
		steps: [{ name: "step-1", mode: "auto", prompt: "p", rules: [] }],
	},
	totalTokenUsage: { inputTokens: 100, outputTokens: 50 },
	stepStates: { "step-1": "completed" },
	startedAt: 1000,
	updatedAt: 2000,
	...overrides,
});

describe("WorkflowHistory", () => {
	it("shows empty message when no execution history", async () => {
		mockInvoke.mockResolvedValue([]);
		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(
				screen.getByText("過去の実行履歴はありません"),
			).toBeInTheDocument();
		});
	});

	it("shows execution IDs when history exists", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001", "exec-002"]);
			}
			return Promise.resolve(null);
		});
		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
			expect(screen.getByText("exec-002")).toBeInTheDocument();
		});
	});

	it("loads execution log and state when an execution is selected", async () => {
		const mockState = makeMockState();

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001"]);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([
					{
						event: "workflow_started",
						execution_id: "exec-001",
						workflow_name: "test-wf",
						timestamp: 1000,
					},
				]);
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(mockState);
			}
			return Promise.resolve(null);
		});

		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		fireEvent.click(screen.getByText("exec-001"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_workflow_execution_log", {
				executionId: "exec-001",
			});
			expect(mockInvoke).toHaveBeenCalledWith("get_workflow_execution_state", {
				executionId: "exec-001",
			});
		});
	});

	it("displays event log entries after selection", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001"]);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([
					{
						event: "workflow_started",
						execution_id: "exec-001",
						workflow_name: "test-wf",
						timestamp: 1000,
					},
					{
						event: "step_started",
						execution_id: "exec-001",
						workflow_name: "test-wf",
						step_name: "step-1",
						execution_count: 1,
						timestamp: 1001,
					},
				]);
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(null);
			}
			return Promise.resolve(null);
		});

		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		fireEvent.click(screen.getByText("exec-001"));

		await waitFor(() => {
			expect(screen.getByText("workflow_started")).toBeInTheDocument();
			expect(screen.getByText("step_started")).toBeInTheDocument();
			expect(screen.getByText("(step-1)")).toBeInTheDocument();
		});
	});

	it("displays total token usage when non-zero", async () => {
		const mockState = makeMockState({
			totalTokenUsage: { inputTokens: 500, outputTokens: 200 },
		});

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001"]);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(mockState);
			}
			return Promise.resolve(null);
		});

		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		fireEvent.click(screen.getByText("exec-001"));

		await waitFor(() => {
			expect(screen.getByText(/合計:/)).toBeInTheDocument();
			expect(screen.getByText(/700/)).toBeInTheDocument();
		});
	});

	it("highlights selected execution", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001", "exec-002"]);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			return Promise.resolve(null);
		});

		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		fireEvent.click(screen.getByText("exec-001"));

		await waitFor(() => {
			const btn = screen.getByText("exec-001");
			expect(btn.className).toContain("bg-muted");
		});
	});

	it("shows step detail when a step node is clicked in history graph", async () => {
		const mockState = makeMockState();

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001"]);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(mockState);
			}
			return Promise.resolve(null);
		});

		render(<WorkflowHistory worktreePath="/repo" />);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		fireEvent.click(screen.getByText("exec-001"));

		await waitFor(() => {
			expect(screen.getByTestId("react-flow")).toBeInTheDocument();
		});

		// Click on a step node in the graph (mock triggers onNodeClick with id "step-1")
		fireEvent.click(screen.getByTestId("react-flow"));

		await waitFor(() => {
			// StepDetail should show the step name header and a Close button
			expect(screen.getByText("step-1")).toBeInTheDocument();
			expect(screen.getByText("Close")).toBeInTheDocument();
			// StepDetail should show the step history entry result
			expect(screen.getByText("done")).toBeInTheDocument();
		});
	});

	it("re-fetches execution list when workflow reaches terminal state", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_executions") {
				return Promise.resolve(["exec-001"]);
			}
			return Promise.resolve(null);
		});

		const { rerender } = render(
			<WorkflowHistory
				worktreePath="/repo"
				workflowState={{ state: { type: "running" } } as WorkflowState}
			/>,
		);
		await waitFor(() => {
			expect(screen.getByText("exec-001")).toBeInTheDocument();
		});

		const callCountBefore = mockInvoke.mock.calls.filter(
			(c: unknown[]) => c[0] === "list_workflow_executions",
		).length;

		// Transition to completed
		rerender(
			<WorkflowHistory
				worktreePath="/repo"
				workflowState={{ state: { type: "completed" } } as WorkflowState}
			/>,
		);

		await waitFor(() => {
			const callCountAfter = mockInvoke.mock.calls.filter(
				(c: unknown[]) => c[0] === "list_workflow_executions",
			).length;
			expect(callCountAfter).toBeGreaterThan(callCountBefore);
		});
	});
});
