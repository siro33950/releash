import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";
import { WorkflowTrace } from "./WorkflowTrace";

const mockInvoke = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function makeWorkflowState(
	overrides: Partial<WorkflowState> = {},
): WorkflowState {
	return {
		executionId: "exec-001",
		workflowName: "test-workflow",
		state: { type: "running" },
		currentStepIndex: 0,
		currentStepName: "plan",
		totalSteps: 3,
		stepHistory: [],
		stepExecutionCounts: {},
		stepOutputs: {},
		workflowDefinition: {
			name: "test-workflow",
			description: "test",
			builtin: false,
			steps: [
				{ name: "plan", mode: "auto", instruction: "plan", rules: [] },
				{ name: "review", mode: "approval", instruction: "review", rules: [] },
				{ name: "fix", mode: "approval", instruction: "fix", rules: [] },
			],
		},
		totalTokenUsage: { inputTokens: 100, outputTokens: 200 },
		stepStates: { plan: "running", review: "pending", fix: "pending" },
		startedAt: 1000,
		updatedAt: 2000,
		...overrides,
	};
}

describe("WorkflowTrace", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValue(undefined);
	});

	it("shows current action and total token usage", () => {
		render(<WorkflowTrace workflowState={makeWorkflowState()} />);
		expect(screen.getByText("Running plan")).toBeInTheDocument();
		expect(screen.getByText("300 tokens")).toBeInTheDocument();
	});

	it("renders the active trace item with mode and state", () => {
		render(<WorkflowTrace workflowState={makeWorkflowState()} />);
		expect(screen.getByText("plan")).toBeInTheDocument();
		expect(screen.getByText("auto")).toBeInTheDocument();
		expect(screen.getByText("Running")).toBeInTheDocument();
		expect(screen.queryByText("run 1")).not.toBeInTheDocument();
	});

	it("shows waiting approval as the required current action", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
				})}
			/>,
		);
		expect(
			screen.getByText("Waiting for approval: review"),
		).toBeInTheDocument();
		expect(screen.getByText("review")).toBeInTheDocument();
		expect(screen.getByText("approval")).toBeInTheDocument();
		expect(screen.getByText("Waiting for approval")).toBeInTheDocument();
	});

	it("renders approval actions inside the current waiting step row when context is provided", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					approvalOperations: { canReject: true },
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		const reviewRow = screen.getByTestId("trace-item-review-1");
		expect(
			within(reviewRow).getByRole("button", { name: "Approve step" }),
		).toBeInTheDocument();
		expect(
			within(reviewRow).getByRole("button", { name: "Reject step" }),
		).toBeInTheDocument();
	});

	it("does not render approval actions without approval context", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					approvalOperations: { canReject: true },
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
				})}
			/>,
		);

		expect(
			screen.queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("invokes approve_workflow_step from the current step row and clears row actions after transition", () => {
		const { rerender } = render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		fireEvent.click(screen.getByRole("button", { name: "Approve step" }));

		expect(mockInvoke).toHaveBeenCalledWith("approve_workflow_step", {
			worktreePath: "/repo",
			decision: "approve",
			executionId: "exec-001",
			stepName: "review",
		});

		rerender(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "running" },
					currentStepName: "fix",
					currentStepIndex: 2,
					stepStates: {
						plan: "completed",
						review: "completed",
						fix: "running",
					},
					stepHistory: [
						{
							stepName: "review",
							completedAt: 1001,
							result: "approve",
						},
					],
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		const reviewRow = screen.getByTestId("trace-item-review-1");
		expect(
			within(reviewRow).queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			within(reviewRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
		expect(screen.getByText("Running fix")).toBeInTheDocument();
	});

	it("keeps reject input and shows row error when reject command fails", async () => {
		mockInvoke.mockRejectedValue("validation_error: comment is too long");

		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					approvalOperations: { canReject: true },
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		const row = screen.getByTestId("trace-item-review-1");
		fireEvent.click(within(row).getByRole("button", { name: "Reject step" }));
		fireEvent.change(within(row).getByLabelText("Reject comment"), {
			target: { value: "x".repeat(8193) },
		});
		fireEvent.click(within(row).getByRole("button", { name: "Submit reject" }));

		expect(await within(row).findByRole("alert")).toHaveTextContent(
			"comment is too long",
		);
		expect(within(row).getByLabelText("Reject comment")).toHaveValue(
			"x".repeat(8193),
		);
	});

	it("renders approval actions only on the current occurrence row", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "review",
					currentStepIndex: 1,
					stepExecutionCounts: { review: 2 },
					approvalOperations: { canReject: true },
					stepStates: {
						plan: "completed",
						review: "waiting_approval",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "review",
							completedAt: 1001,
							result: "reject",
						},
					],
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		expect(
			within(screen.getByTestId("trace-item-review-1")).queryByRole("button", {
				name: "Approve step",
			}),
		).not.toBeInTheDocument();
		expect(
			within(screen.getByTestId("trace-item-review-2")).getByRole("button", {
				name: "Approve step",
			}),
		).toBeInTheDocument();
	});

	it("does not render approval actions in parallel block rows", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "parallel-review",
					currentStepIndex: 0,
					totalSteps: 2,
					approvalOperations: { canReject: true },
					stepStates: {
						"parallel-review": "waiting_approval",
						report: "pending",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "parallel-review",
								rules: [],
								parallel: [
									{ name: "arch-review", mode: "auto" },
									{ name: "security-review", mode: "auto" },
								],
							},
							{
								name: "report",
								mode: "auto",
								instruction: "report",
								rules: [],
							},
						],
					},
					activeParallelSteps: [
						{
							stepName: "arch-review",
							state: "running",
							runIndex: 0,
						},
						{
							stepName: "security-review",
							state: "running",
							runIndex: 0,
						},
					],
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		const parentRow = screen.getByTestId("trace-item-parallel-review-1");
		const childRow = screen.getByTestId("trace-child-item-arch-review-1");

		expect(
			within(parentRow).queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			within(parentRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
		expect(
			within(childRow).queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			within(childRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("does not render approval actions for a current parallel definition without active child rows", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "parallel-review",
					currentStepIndex: 0,
					totalSteps: 2,
					approvalOperations: { canReject: true },
					stepStates: {
						"parallel-review": "waiting_approval",
						report: "pending",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "parallel-review",
								rules: [],
								parallel: [
									{ name: "arch-review", mode: "auto" },
									{ name: "security-review", mode: "auto" },
								],
							},
							{
								name: "report",
								mode: "auto",
								instruction: "report",
								rules: [],
							},
						],
					},
					activeParallelSteps: [],
				})}
				approvalAction={{
					worktreePath: "/repo",
					executionId: "exec-001",
				}}
			/>,
		);

		const parentRow = screen.getByTestId("trace-item-parallel-review-1");

		expect(
			within(parentRow).queryByRole("button", { name: "Approve step" }),
		).not.toBeInTheDocument();
		expect(
			within(parentRow).queryByRole("button", { name: "Reject step" }),
		).not.toBeInTheDocument();
	});

	it("shows step history entries in chronological loop order", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "completed" },
					stepStates: {
						plan: "completed",
						review: "completed",
						fix: "pending",
					},
					currentStepName: "review",
					stepExecutionCounts: { review: 3, fix: 2 },
					stepHistory: [
						{
							stepName: "review",
							completedAt: 1001,
							result: "NEEDS_FIX",
							tokenUsage: { inputTokens: 50, outputTokens: 30 },
						},
						{
							stepName: "fix",
							completedAt: 1002,
							result: "FIX_DONE",
							tokenUsage: { inputTokens: 20, outputTokens: 10 },
						},
						{
							stepName: "review",
							completedAt: 1003,
							result: "NEEDS_FIX",
							tokenUsage: { inputTokens: 60, outputTokens: 40 },
						},
						{
							stepName: "fix",
							completedAt: 1004,
							result: "FIX_DONE",
						},
						{
							stepName: "review",
							completedAt: 1005,
							result: "LGTM",
						},
					],
				})}
			/>,
		);
		const stepNames = screen
			.getAllByText(/^(review|fix)$/)
			.map((node) => node.textContent);
		expect(stepNames).toEqual(["review", "fix", "review", "fix", "review"]);
		expect(screen.getAllByText("NEEDS_FIX")).toHaveLength(2);
		expect(screen.getAllByText("FIX_DONE")).toHaveLength(2);
		expect(screen.getByText("LGTM")).toBeInTheDocument();
		expect(screen.queryByText("Result: NEEDS_FIX")).not.toBeInTheDocument();
		expect(screen.queryByText("Result: FIX_DONE")).not.toBeInTheDocument();
		expect(screen.queryByText("Result: LGTM")).not.toBeInTheDocument();
		expect(screen.queryByText("run 1")).not.toBeInTheDocument();
		expect(screen.queryByText("run 2")).not.toBeInTheDocument();
		expect(screen.queryByText("run 3")).not.toBeInTheDocument();
		expect(screen.queryByText("#1")).not.toBeInTheDocument();
		expect(screen.queryByText("#2")).not.toBeInTheDocument();
		expect(screen.getByText("80 tokens")).toBeInTheDocument();
		expect(screen.getByText("100 tokens")).toBeInTheDocument();
	});

	it("does not render order badge for a single-step workflow", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "plan",
								mode: "auto",
								instruction: "plan",
								rules: [],
							},
						],
					},
					totalSteps: 1,
					stepStates: { plan: "running" },
				})}
			/>,
		);
		expect(screen.getByText("plan")).toBeInTheDocument();
		expect(screen.queryByText("#1")).not.toBeInTheDocument();
	});

	it("calls onSessionClick when a history entry View button is clicked", () => {
		const onSessionClick = vi.fn();
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: { plan: "completed", review: "pending", fix: "pending" },
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "done",
							sessionId: "session-1",
						},
					],
				})}
				onSessionClick={onSessionClick}
			/>,
		);
		fireEvent.click(screen.getByText("View"));
		expect(onSessionClick).toHaveBeenCalledWith("session-1");
	});

	it("shows View for the current step when a current session exists", () => {
		const onSessionClick = vi.fn();
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					currentSessionId: "session-current",
				})}
				onSessionClick={onSessionClick}
			/>,
		);
		fireEvent.click(screen.getByText("View"));
		expect(onSessionClick).toHaveBeenCalledWith("session-current");
	});

	it("renders event log entries when provided", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState()}
				events={[
					{
						event: "workflow_started",
						execution_id: "exec-001",
						workflow_name: "test-workflow",
						worktree_path: "/repo",
						timestamp: 1000,
					},
					{
						event: "step_started",
						execution_id: "exec-001",
						workflow_name: "test-workflow",
						step_name: "plan",
						execution_count: 1,
						timestamp: 1001,
					},
				]}
			/>,
		);
		expect(screen.getByText("Event log")).toBeInTheDocument();
		expect(screen.getByText("workflow_started")).toBeInTheDocument();
		expect(screen.getByText("step_started")).toBeInTheDocument();
		expect(screen.getByText("(plan)")).toBeInTheDocument();
	});

	it("shows Structured Output toggle when stepHistory entry has structuredOutput", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "completed" },
					stepStates: {
						plan: "completed",
						review: "pending",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "done",
							structuredOutput: { verdict: "LGTM" },
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("Structured Output")).toBeInTheDocument();
	});

	it("does not render default Result text for completed step without result", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "completed" },
					stepStates: {
						plan: "completed",
						review: "pending",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: null,
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("plan")).toBeInTheDocument();
		expect(screen.queryByText("Result: completed")).not.toBeInTheDocument();
	});

	it("expands structuredOutput on toggle click", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: {
						plan: "completed",
						review: "pending",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "LGTM",
							structuredOutput: { verdict: "LGTM", findings: [] },
						},
					],
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Structured Output"));
		expect(screen.getByText(/"verdict": "LGTM"/)).toBeInTheDocument();
	});

	it("shows spec_file_path only inside expanded Structured Output JSON", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: {
						plan: "completed",
						review: "pending",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "done",
							structuredOutput: {
								spec_file_path: "docs/spec/issues-909.md",
							},
						},
					],
				})}
			/>,
		);
		expect(
			screen.queryByRole("button", { name: "docs/spec/issues-909.md" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByText("docs/spec/issues-909.md"),
		).not.toBeInTheDocument();
		fireEvent.click(screen.getByText("Structured Output"));
		expect(screen.getByText(/docs\/spec\/issues-909\.md/)).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "docs/spec/issues-909.md" }),
		).not.toBeInTheDocument();
	});

	it("shows VerdictBadge for collect step reduce result", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: {
						plan: "completed",
						review: "pending",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "NEEDS_FIX",
						},
					],
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "plan",
								mode: "auto",
								instruction: "plan",
								rules: [],
								collect: {
									from: ["a", "b"],
									reduce: "any_needs_fix",
								},
							},
							{
								name: "review",
								mode: "approval",
								instruction: "review",
								rules: [],
							},
							{
								name: "fix",
								mode: "approval",
								instruction: "fix",
								rules: [],
							},
						],
					},
				})}
			/>,
		);
		expect(screen.getAllByText("NEEDS_FIX")).toHaveLength(1);
		expect(screen.queryByText("Result: NEEDS_FIX")).not.toBeInTheDocument();
	});

	it("renders parallel block with child steps", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					currentStepName: "parallel-review",
					currentStepIndex: 0,
					totalSteps: 2,
					stepStates: {
						"parallel-review": "running",
						report: "pending",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "parallel-review",
								rules: [],
								parallel: [
									{ name: "arch-review", mode: "auto" },
									{ name: "security-review", mode: "auto" },
								],
							},
							{
								name: "report",
								mode: "auto",
								instruction: "report",
								rules: [],
							},
						],
					},
					activeParallelSteps: [
						{
							stepName: "arch-review",
							state: "running",
							runIndex: 0,
						},
						{
							stepName: "security-review",
							state: "completed",
							sessionId: "sess-2",
							runIndex: 0,
							completedAt: 2001,
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("parallel-review")).toBeInTheDocument();
		expect(screen.getByText("parallel")).toBeInTheDocument();
		expect(screen.getByText("arch-review")).toBeInTheDocument();
		expect(screen.getByText("security-review")).toBeInTheDocument();
		expect(screen.getByText("1/2 completed")).toBeInTheDocument();
		expect(screen.queryByText("#1")).not.toBeInTheDocument();
		expect(screen.queryByText("run 1")).not.toBeInTheDocument();
	});

	it("renders completed parallel block from stepHistory with child steps and output", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "completed" },
					currentStepName: "report",
					currentStepIndex: 1,
					totalSteps: 2,
					stepStates: {
						"parallel-review": "completed",
						report: "completed",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "parallel-review",
								rules: [],
								parallel: [
									{ name: "arch-review", mode: "auto" },
									{ name: "security-review", mode: "auto" },
								],
								aggregate: {
									all_match: "LGTM",
									// biome-ignore lint/suspicious/noThenProperty: AggregateConfig domain type
									then: "report",
									else: "fix",
								},
							},
							{
								name: "report",
								mode: "auto",
								instruction: "report",
								rules: [],
							},
						],
					},
					stepHistory: [
						{
							stepName: "parallel-review",
							completedAt: 2001,
							result: "then",
							tokenUsage: { inputTokens: 100, outputTokens: 50 },
							structuredOutput: {
								"arch-review": { verdict: "LGTM" },
								"security-review": { verdict: "LGTM" },
							},
						},
					],
					stepOutputs: {
						"arch-review": {
							stepName: "arch-review",
							runIndex: 1,
							sessionId: "sess-arch",
							result: "LGTM",
							structuredOutput: { verdict: "LGTM" },
							tokenUsage: { inputTokens: 50, outputTokens: 25 },
							completedAt: 2000,
						},
						"security-review": {
							stepName: "security-review",
							runIndex: 1,
							sessionId: "sess-sec",
							result: "LGTM",
							structuredOutput: { verdict: "LGTM" },
							tokenUsage: { inputTokens: 50, outputTokens: 25 },
							completedAt: 2001,
						},
						"parallel-review": {
							stepName: "parallel-review",
							runIndex: 1,
							structuredOutput: {
								"arch-review": { verdict: "LGTM" },
								"security-review": { verdict: "LGTM" },
							},
							tokenUsage: { inputTokens: 100, outputTokens: 50 },
							completedAt: 2001,
						},
					},
				})}
			/>,
		);
		expect(screen.getByText("parallel-review")).toBeInTheDocument();
		expect(screen.getByText("parallel")).toBeInTheDocument();
		expect(screen.getByText("arch-review")).toBeInTheDocument();
		expect(screen.getByText("security-review")).toBeInTheDocument();
		expect(screen.getByText("2/2 completed")).toBeInTheDocument();
		expect(screen.queryByText("Result: then")).not.toBeInTheDocument();
		expect(screen.queryByText("then")).not.toBeInTheDocument();
		expect(screen.getByText("150 tokens")).toBeInTheDocument();
		expect(
			screen.getAllByText("Structured Output").length,
		).toBeGreaterThanOrEqual(1);
	});

	it("shows contract_repair_requested event with attempt and violation_reason in event log", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState()}
				events={[
					{
						event: "step_started",
						execution_id: "exec-001",
						workflow_name: "test-workflow",
						step_name: "plan",
						execution_count: 1,
						timestamp: 1001,
					},
					{
						event: "contract_repair_requested",
						execution_id: "exec-001",
						workflow_name: "test-workflow",
						step_name: "plan",
						attempt: 1,
						violation_reason: "missing verdict field",
						timestamp: 1002,
					},
					{
						event: "contract_repair_requested",
						execution_id: "exec-001",
						workflow_name: "test-workflow",
						step_name: "plan",
						attempt: 2,
						violation_reason: "invalid format",
						timestamp: 1003,
					},
				]}
			/>,
		);
		expect(screen.getByText("Event log")).toBeInTheDocument();
		expect(screen.getAllByText("contract_repair_requested")).toHaveLength(2);
		expect(
			screen.getByText("retry #1: missing verdict field"),
		).toBeInTheDocument();
		expect(screen.getByText("retry #2: invalid format")).toBeInTheDocument();
	});

	it("displays reject result and comment in trace view", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: {
						plan: "completed",
						review: "completed",
						fix: "running",
					},
					stepHistory: [
						{
							stepName: "plan",
							completedAt: 1001,
							result: "done",
						},
						{
							stepName: "review",
							completedAt: 1002,
							result: "reject",
							structuredOutput: {
								comment: "Please fix the naming convention",
							},
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("reject")).toBeInTheDocument();
		expect(screen.queryByText("Result: reject")).not.toBeInTheDocument();
		fireEvent.click(screen.getByText("Structured Output"));
		expect(
			screen.getByText(/"Please fix the naming convention"/),
		).toBeInTheDocument();
	});

	it("displays adopted fix policy structured output in step history", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					stepStates: {
						plan: "completed",
						review: "completed",
						fix: "pending",
					},
					stepHistory: [
						{
							stepName: "review",
							completedAt: 1001,
							result: "approve",
							structuredOutput: {
								policy: "Fix only the reported findings without refactoring.",
								review_step: "code_review_parallel",
							},
						},
					],
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Structured Output"));
		expect(
			screen.getByText(/"Fix only the reported findings without refactoring."/),
		).toBeInTheDocument();
		expect(screen.getByText(/"code_review_parallel"/)).toBeInTheDocument();
	});

	it("shows fix policy step as waiting for approval in trace", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
					currentStepName: "plan_fix_policy",
					currentStepIndex: 1,
					stepStates: {
						plan: "completed",
						plan_fix_policy: "waiting_approval",
						fix: "pending",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "plan",
								mode: "auto",
								instruction: "plan",
								rules: [],
							},
							{
								name: "plan_fix_policy",
								mode: "approval",
								instruction: "policy",
								rules: [],
							},
							{
								name: "fix",
								mode: "auto",
								instruction: "fix",
								rules: [],
							},
						],
					},
				})}
			/>,
		);
		expect(
			screen.getByText("Waiting for approval: plan_fix_policy"),
		).toBeInTheDocument();
		expect(screen.getByText("plan_fix_policy")).toBeInTheDocument();
		expect(screen.getByText("Waiting for approval")).toBeInTheDocument();
	});

	it("renders parallel block as completed when all children done", () => {
		render(
			<WorkflowTrace
				workflowState={makeWorkflowState({
					currentStepName: "parallel-review",
					currentStepIndex: 0,
					totalSteps: 2,
					stepStates: {
						"parallel-review": "running",
						report: "pending",
					},
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "parallel-review",
								rules: [],
								parallel: [
									{ name: "arch-review", mode: "auto" },
									{ name: "security-review", mode: "auto" },
								],
							},
							{
								name: "report",
								mode: "auto",
								instruction: "report",
								rules: [],
							},
						],
					},
					activeParallelSteps: [
						{
							stepName: "arch-review",
							state: "completed",
							sessionId: "sess-1",
							runIndex: 0,
							completedAt: 2001,
						},
						{
							stepName: "security-review",
							state: "completed",
							sessionId: "sess-2",
							runIndex: 0,
							completedAt: 2002,
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("2/2 completed")).toBeInTheDocument();
	});
});
