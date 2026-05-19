import {
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";
import { installScrollMetricsMock } from "./testUtils";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

const mockInvoke = vi.fn().mockResolvedValue([]);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

const { WorkflowPanel } = await import("./WorkflowPanel");

/// run_id 一覧の test helper。`list_workflow_runs_for_worktree` が返す
/// `WorkflowRunSummary[]` の最小形を生成する。
function makeRunSummaries(
	runIds: string[],
	worktreePath: string,
): Array<Record<string, unknown>> {
	return runIds.map((runId) => ({
		runId,
		workflowName: "wf",
		status: "completed",
		worktreePath,
		triggerSource: "desktop_ui",
		startedAt: 0,
		updatedAt: 0,
	}));
}

function makeWorkflowState(
	overrides: Partial<WorkflowState> = {},
): WorkflowState {
	return {
		executionId: "exec-001",
		workflowName: "test-workflow",
		state: { type: "running" },
		currentStepIndex: 0,
		currentStepName: "step-1",
		totalSteps: 2,
		stepHistory: [],
		stepExecutionCounts: {},
		stepOutputs: {},
		workflowDefinition: {
			name: "test-workflow",
			description: "test",
			builtin: false,
			nodes: [
				{ name: "step-1", type: "agent", instruction: "implement", rules: [] },
				{ name: "step-2", type: "approval", instruction: "review", rules: [] },
			],
		},
		totalTokenUsage: { inputTokens: 100, outputTokens: 200 },
		stepStates: { "step-1": "running", "step-2": "pending" },
		startedAt: 1000,
		updatedAt: 2000,
		...overrides,
	};
}

describe("WorkflowPanel", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValue([]);
	});

	it("displays workflow name and status badge", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		expect(screen.getByText("test-workflow")).toBeInTheDocument();
		expect(screen.getAllByText("running").length).toBeGreaterThan(0);
	});

	it("displays total token usage", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		expect(screen.getByText("300 tokens")).toBeInTheDocument();
	});

	it("keeps the workflow status outside the trace scroll region", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		const status = screen.getByTestId("workflow-status-summary");
		const trace = screen.getByTestId("workflow-trace-scroll");
		expect(status).toBeInTheDocument();
		expect(trace).not.toContainElement(status);
	});

	it("resets auto-follow and scrolls to the bottom after switching to a past execution tab", async () => {
		const scrollMock = installScrollMetricsMock(HTMLElement.prototype, {
			scrollHeight: 500,
			scrollTop: 500,
			clientHeight: 100,
		});
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_runs_for_worktree") {
				return Promise.resolve(
					makeRunSummaries(["exec-current", "exec-old"], "/repo"),
				);
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(
					makeWorkflowState({
						executionId: "exec-old",
						workflowName: "old-workflow",
						state: { type: "completed" },
						currentStepName: "step-2",
						stepHistory: [
							{ stepName: "step-1", completedAt: 1500, result: "done" },
						],
					}),
				);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			return Promise.resolve(undefined);
		});

		try {
			render(
				<WorkflowPanel
					workflowState={makeWorkflowState({ executionId: "exec-current" })}
					worktreePath="/repo"
				/>,
			);
			const currentTrace = screen.getByTestId("workflow-trace-scroll");
			scrollMock.setMetrics({ scrollTop: 250 });
			fireEvent.scroll(currentTrace);

			scrollMock.setMetrics({ scrollHeight: 900 });
			fireEvent.click(
				screen.getByRole("button", { name: "Execution history" }),
			);
			fireEvent.click(await screen.findByText("exec-old"));

			await waitFor(() => {
				expect(screen.getByText("old-workflow")).toBeInTheDocument();
				expect(scrollMock.scrollTop).toBe(900);
			});
		} finally {
			scrollMock.restore();
		}
	});

	it("resets auto-follow and scrolls to the bottom after worktreePath changes", () => {
		const scrollMock = installScrollMetricsMock(HTMLElement.prototype, {
			scrollHeight: 500,
			scrollTop: 500,
			clientHeight: 100,
		});

		try {
			const { rerender } = render(
				<WorkflowPanel
					workflowState={makeWorkflowState()}
					worktreePath="/repo-a"
				/>,
			);
			const trace = screen.getByTestId("workflow-trace-scroll");
			scrollMock.setMetrics({ scrollTop: 250 });
			fireEvent.scroll(trace);

			scrollMock.setMetrics({ scrollHeight: 900 });
			rerender(
				<WorkflowPanel
					workflowState={makeWorkflowState()}
					worktreePath="/repo-b"
				/>,
			);

			expect(scrollMock.scrollTop).toBe(900);
		} finally {
			scrollMock.restore();
		}
	});

	it("shows Stop button when running", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({ state: { type: "running" } })}
				worktreePath="/repo"
			/>,
		);
		expect(
			screen.getByRole("button", { name: "Stop workflow" }),
		).toBeInTheDocument();
		expect(
			within(screen.getByTestId("trace-item-step-1-1")).queryByRole("button", {
				name: "Stop workflow",
			}),
		).not.toBeInTheDocument();
	});

	it("shows Stop button when waiting_approval", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
				})}
				worktreePath="/repo"
			/>,
		);
		expect(
			screen.getByRole("button", { name: "Stop workflow" }),
		).toBeInTheDocument();
	});

	it("hides Stop button when completed", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({ state: { type: "completed" } })}
				worktreePath="/repo"
			/>,
		);
		expect(
			screen.queryByRole("button", { name: "Stop workflow" }),
		).not.toBeInTheDocument();
	});

	it("hides Stop button when failed", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "failed", reason: "error" },
				})}
				worktreePath="/repo"
			/>,
		);
		expect(
			screen.queryByRole("button", { name: "Stop workflow" }),
		).not.toBeInTheDocument();
	});

	it("invokes abort_workflow when Stop is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Stop workflow" }));
		expect(mockInvoke).toHaveBeenCalledWith("abort_workflow", {
			runId: "exec-001",
		});
	});

	it("shows abort command errors in the top action area", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "abort_workflow") {
				return Promise.reject("abort failed");
			}
			return Promise.resolve([]);
		});

		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Stop workflow" }));

		expect(await screen.findByRole("alert")).toHaveTextContent("abort failed");
	});

	it("clears abort command errors when workflow identity changes", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "abort_workflow") {
				return Promise.reject("abort failed");
			}
			return Promise.resolve([]);
		});

		const { rerender } = render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Stop workflow" }));
		expect(await screen.findByRole("alert")).toHaveTextContent("abort failed");

		rerender(
			<WorkflowPanel
				workflowState={makeWorkflowState({ executionId: "exec-002" })}
				worktreePath="/repo"
			/>,
		);

		await waitFor(() => {
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		});
	});

	it("shows current execution as a tab with workflow name", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState()}
				worktreePath="/repo"
			/>,
		);
		const tabs = screen.getAllByRole("tab");
		expect(tabs.length).toBeGreaterThanOrEqual(1);
		expect(tabs[0]).toHaveTextContent("test-workflow");
	});

	it("shows empty state when no workflow and no history", () => {
		render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
		expect(screen.getByText("No workflow running")).toBeInTheDocument();
	});

	it("displays correct status badge color class for each state", () => {
		const { container: c1 } = render(
			<WorkflowPanel
				workflowState={makeWorkflowState({ state: { type: "completed" } })}
				worktreePath="/repo"
			/>,
		);
		const badge = c1.querySelector(".bg-green-500\\/20");
		expect(badge).toBeInTheDocument();
	});

	it("shows Approve and Reject buttons when waiting_approval", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		const currentRow = screen.getByTestId("trace-item-step-1-1");
		expect(
			within(currentRow).getByRole("button", { name: "Approve step" }),
		).toBeInTheDocument();
		expect(
			within(currentRow).getByRole("button", { name: "Reject step" }),
		).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Stop workflow" })
				.parentElement?.querySelector('[aria-label="Approve step"]'),
		).not.toBeInTheDocument();
	});

	it("hides Reject button when waiting_approval cannot reject", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: false },
				})}
				worktreePath="/repo"
			/>,
		);
		const currentRow = screen.getByTestId("trace-item-step-1-1");
		expect(
			within(currentRow).getByRole("button", { name: "Approve step" }),
		).toBeInTheDocument();
		expect(
			within(currentRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("invokes approve_workflow_step with approve when Approve is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Approve step" }));
		expect(mockInvoke).toHaveBeenCalledWith("approve_workflow_step", {
			runId: "exec-001",
			decision: "approve",
			stepName: "step-1",
		});
	});

	it("shows approval command errors in the UI", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "approve_workflow_step") {
				return Promise.reject(
					"invalid_state: Workflow is not waiting for approval",
				);
			}
			return Promise.resolve([]);
		});

		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
				})}
				worktreePath="/repo"
			/>,
		);
		const currentRow = screen.getByTestId("trace-item-step-1-1");
		fireEvent.click(
			within(currentRow).getByRole("button", { name: "Approve step" }),
		);

		expect(await within(currentRow).findByRole("alert")).toHaveTextContent(
			"Workflow approval is no longer available for the current step.",
		);
	});

	it("shows reject comment form when Reject is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		expect(screen.getByLabelText("Reject comment")).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Submit reject" }),
		).toBeDisabled();
	});

	it("invokes approve_workflow_step with reject comment when submitted and clears row actions after transition", async () => {
		const workflowDefinition = {
			name: "test-workflow",
			description: "test",
			builtin: false,
			nodes: [
				{
					name: "review",
					type: "approval" as const,
					instruction: "review",
					rules: [{ match: "reject", next: "fix" }],
				},
				{ name: "fix", type: "agent" as const, instruction: "fix", rules: [] },
			],
		} satisfies WorkflowState["workflowDefinition"];
		const { rerender } = render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 0,
					totalSteps: 2,
					approvalOperations: { canReject: true },
					stepStates: {
						review: "waiting_approval",
						fix: "pending",
					},
					workflowDefinition,
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		fireEvent.change(screen.getByLabelText("Reject comment"), {
			target: { value: "Please fix the bug" },
		});
		expect(
			screen.getByRole("button", { name: "Submit reject" }),
		).not.toBeDisabled();
		fireEvent.click(screen.getByRole("button", { name: "Submit reject" }));
		expect(mockInvoke).toHaveBeenCalledWith("approve_workflow_step", {
			runId: "exec-001",
			decision: { reject: { comment: "Please fix the bug" } },
			stepName: "review",
		});
		await waitFor(() => {
			expect(screen.queryByLabelText("Reject comment")).not.toBeInTheDocument();
		});

		rerender(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "running" },
					currentStepName: "fix",
					currentStepIndex: 1,
					totalSteps: 2,
					stepStates: {
						review: "completed",
						fix: "running",
					},
					stepHistory: [
						{
							stepName: "review",
							completedAt: 1001,
							result: "reject",
						},
					],
					workflowDefinition,
				})}
				worktreePath="/repo"
			/>,
		);

		const reviewRow = screen.getByTestId("trace-item-review-1");
		expect(within(reviewRow).getByText("reject")).toBeInTheDocument();
		expect(screen.getByText("Running fix")).toBeInTheDocument();
		expect(
			within(reviewRow).queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			within(reviewRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("hides reject form after successful submit", async () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		fireEvent.change(screen.getByLabelText("Reject comment"), {
			target: { value: "Please fix" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Submit reject" }));
		await waitFor(() => {
			expect(screen.queryByLabelText("Reject comment")).not.toBeInTheDocument();
		});
	});

	it("does not allow submitting reject with empty comment", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		fireEvent.change(screen.getByLabelText("Reject comment"), {
			target: { value: "   " },
		});
		expect(
			screen.getByRole("button", { name: "Submit reject" }),
		).toBeDisabled();
	});

	it("cancels reject mode when Cancel is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		expect(screen.getByLabelText("Reject comment")).toBeInTheDocument();
		fireEvent.click(screen.getByText("Cancel"));
		expect(screen.queryByLabelText("Reject comment")).not.toBeInTheDocument();
	});

	it("hides reject form when state changes from waiting_approval to running", () => {
		const { rerender } = render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					approvalOperations: { canReject: true },
				})}
				worktreePath="/repo"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		expect(screen.getByLabelText("Reject comment")).toBeInTheDocument();
		rerender(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "running" },
				})}
				worktreePath="/repo"
			/>,
		);
		expect(screen.queryByLabelText("Reject comment")).not.toBeInTheDocument();
	});

	describe("NewWorkflowButton", () => {
		beforeEach(() => {
			mockInvoke.mockReset();
			mockInvoke.mockImplementation((cmd: string) => {
				if (cmd === "list_workflows") {
					return Promise.resolve([
						{
							name: "quick-fix",
							description: "Quick fix workflow",
							builtin: true,
						},
						{
							name: "plan-implement-review",
							description: "Full workflow",
							builtin: true,
						},
					]);
				}
				if (cmd === "list_workflow_runs_for_worktree") {
					return Promise.resolve([]);
				}
				return Promise.resolve(undefined);
			});
		});

		it("shows workflow list when + button is clicked", async () => {
			render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
				expect(screen.getByText("plan-implement-review")).toBeInTheDocument();
			});
		});

		it("shows task input after selecting a workflow", async () => {
			render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
			});
			fireEvent.click(screen.getByText("quick-fix"));
			expect(
				screen.getByPlaceholderText("Task description (optional)"),
			).toBeInTheDocument();
			expect(screen.getByText("Start")).toBeInTheDocument();
		});

		it("invokes start_workflow with task when Start is clicked", async () => {
			render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
			});
			fireEvent.click(screen.getByText("quick-fix"));
			fireEvent.change(
				screen.getByPlaceholderText("Task description (optional)"),
				{ target: { value: "Fix the login bug" } },
			);
			fireEvent.click(screen.getByText("Start"));
			expect(mockInvoke).toHaveBeenCalledWith("start_workflow", {
				workflowName: "quick-fix",
				worktreePath: "/repo",
				task: "Fix the login bug",
				permissionMode: "readonly",
			});
		});

		it("invokes start_workflow with task: null when task input is empty", async () => {
			render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
			});
			fireEvent.click(screen.getByText("quick-fix"));
			fireEvent.click(screen.getByText("Start"));
			expect(mockInvoke).toHaveBeenCalledWith("start_workflow", {
				workflowName: "quick-fix",
				worktreePath: "/repo",
				task: null,
				permissionMode: "readonly",
			});
		});
	});

	it("does not show Complete/Approve/Reject for auto mode when running", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "running" },
					currentStepIndex: 0,
				})}
				worktreePath="/repo"
			/>,
		);
		expect(
			screen.queryByRole("button", { name: "Complete step" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("keeps past execution workflow status outside the trace scroll region", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_runs_for_worktree") {
				return Promise.resolve(makeRunSummaries(["exec-old"], "/repo"));
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(
					makeWorkflowState({
						executionId: "exec-old",
						state: { type: "completed" },
						currentStepName: "step-2",
						stepHistory: [
							{ stepName: "step-1", completedAt: 1500, result: "done" },
						],
					}),
				);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			return Promise.resolve(undefined);
		});

		render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
		fireEvent.click(screen.getByRole("button", { name: "Execution history" }));
		fireEvent.click(await screen.findByText("exec-old"));

		await waitFor(() => {
			expect(screen.getByTestId("workflow-status-summary")).toBeInTheDocument();
		});
		const status = screen.getByTestId("workflow-status-summary");
		const trace = screen.getByTestId("workflow-trace-scroll");
		expect(trace).not.toContainElement(status);
	});

	it("shows an alert instead of Loading when past execution data fails to load", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_runs_for_worktree") {
				return Promise.resolve(makeRunSummaries(["exec-old"], "/repo"));
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.reject(new Error("state failed"));
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			return Promise.resolve(undefined);
		});

		render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
		fireEvent.click(screen.getByRole("button", { name: "Execution history" }));
		fireEvent.click(await screen.findByText("exec-old"));

		expect(await screen.findByRole("alert")).toHaveAttribute(
			"data-testid",
			"workflow-history-error",
		);
		expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
	});

	it("shows an alert instead of Loading when past execution state is missing", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_runs_for_worktree") {
				return Promise.resolve(makeRunSummaries(["exec-old"], "/repo"));
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(null);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.resolve([]);
			}
			return Promise.resolve(undefined);
		});

		render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
		fireEvent.click(screen.getByRole("button", { name: "Execution history" }));
		fireEvent.click(await screen.findByText("exec-old"));

		expect(await screen.findByRole("alert")).toHaveAttribute(
			"data-testid",
			"workflow-history-error",
		);
		expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
	});

	it("shows an alert instead of Loading when past execution log fails to load", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflow_runs_for_worktree") {
				return Promise.resolve(makeRunSummaries(["exec-old"], "/repo"));
			}
			if (cmd === "get_workflow_execution_state") {
				return Promise.resolve(
					makeWorkflowState({
						executionId: "exec-old",
						state: { type: "completed" },
					}),
				);
			}
			if (cmd === "get_workflow_execution_log") {
				return Promise.reject(new Error("log failed"));
			}
			return Promise.resolve(undefined);
		});

		render(<WorkflowPanel workflowState={null} worktreePath="/repo" />);
		fireEvent.click(screen.getByRole("button", { name: "Execution history" }));
		fireEvent.click(await screen.findByText("exec-old"));

		expect(await screen.findByRole("alert")).toHaveAttribute(
			"data-testid",
			"workflow-history-error",
		);
		expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
	});
});
