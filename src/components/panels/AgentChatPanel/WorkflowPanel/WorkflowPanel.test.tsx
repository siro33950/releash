import {
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";

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
			steps: [
				{ name: "step-1", mode: "auto", instruction: "implement", rules: [] },
				{ name: "step-2", mode: "approval", instruction: "review", rules: [] },
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
			/>,
		);
		expect(screen.getByText("300 tokens")).toBeInTheDocument();
	});

	it("shows Stop button when running", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({ state: { type: "running" } })}
				worktreePath="/repo"
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Stop workflow" }));
		expect(mockInvoke).toHaveBeenCalledWith("abort_workflow", {
			worktreePath: "/repo",
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Stop workflow" }));
		expect(await screen.findByRole("alert")).toHaveTextContent("abort failed");

		rerender(
			<WorkflowPanel
				workflowState={makeWorkflowState({ executionId: "exec-002" })}
				worktreePath="/repo"
				chatSessionId="session-1"
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
				chatSessionId="session-1"
			/>,
		);
		const tabs = screen.getAllByRole("tab");
		expect(tabs.length).toBeGreaterThanOrEqual(1);
		expect(tabs[0]).toHaveTextContent("test-workflow");
	});

	it("shows empty state when no workflow and no history", () => {
		render(
			<WorkflowPanel
				workflowState={null}
				worktreePath="/repo"
				chatSessionId="session-1"
			/>,
		);
		expect(screen.getByText("No workflow running")).toBeInTheDocument();
	});

	it("displays correct status badge color class for each state", () => {
		const { container: c1 } = render(
			<WorkflowPanel
				workflowState={makeWorkflowState({ state: { type: "completed" } })}
				worktreePath="/repo"
				chatSessionId="s1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Approve step" }));
		expect(mockInvoke).toHaveBeenCalledWith("approve_workflow_step", {
			worktreePath: "/repo",
			decision: "approve",
			executionId: "exec-001",
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
			steps: [
				{
					name: "review",
					mode: "approval" as const,
					instruction: "review",
					rules: [{ match: "reject", next: "fix" }],
				},
				{ name: "fix", mode: "auto" as const, instruction: "fix", rules: [] },
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
				chatSessionId="session-1"
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
			worktreePath: "/repo",
			decision: { reject: { comment: "Please fix the bug" } },
			executionId: "exec-001",
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				chatSessionId="session-1"
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
				if (cmd === "list_workflow_executions") {
					return Promise.resolve([]);
				}
				return Promise.resolve(undefined);
			});
		});

		it("shows workflow list when + button is clicked", async () => {
			render(
				<WorkflowPanel
					workflowState={null}
					worktreePath="/repo"
					chatSessionId="session-1"
				/>,
			);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
				expect(screen.getByText("plan-implement-review")).toBeInTheDocument();
			});
		});

		it("shows task input after selecting a workflow", async () => {
			render(
				<WorkflowPanel
					workflowState={null}
					worktreePath="/repo"
					chatSessionId="session-1"
				/>,
			);
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
			render(
				<WorkflowPanel
					workflowState={null}
					worktreePath="/repo"
					chatSessionId="session-1"
				/>,
			);
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
				chatSessionId: "session-1",
				task: "Fix the login bug",
			});
		});

		it("invokes start_workflow with task: null when task input is empty", async () => {
			render(
				<WorkflowPanel
					workflowState={null}
					worktreePath="/repo"
					chatSessionId="session-1"
				/>,
			);
			fireEvent.click(screen.getByRole("button", { name: "New workflow" }));
			await waitFor(() => {
				expect(screen.getByText("quick-fix")).toBeInTheDocument();
			});
			fireEvent.click(screen.getByText("quick-fix"));
			fireEvent.click(screen.getByText("Start"));
			expect(mockInvoke).toHaveBeenCalledWith("start_workflow", {
				workflowName: "quick-fix",
				chatSessionId: "session-1",
				task: null,
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
				chatSessionId="session-1"
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
});
