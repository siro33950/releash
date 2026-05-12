import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";
import { WorkflowTrace } from "./WorkflowTrace";

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
		expect(screen.getAllByText("Result: NEEDS_FIX")).toHaveLength(2);
		expect(screen.getAllByText("Result: FIX_DONE")).toHaveLength(2);
		expect(screen.getByText("Result: LGTM")).toBeInTheDocument();
		expect(screen.getAllByText("run 1")).toHaveLength(2);
		expect(screen.getAllByText("run 2")).toHaveLength(2);
		expect(screen.getByText("run 3")).toBeInTheDocument();
		expect(screen.getByText("80 tokens")).toBeInTheDocument();
		expect(screen.getByText("100 tokens")).toBeInTheDocument();
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

	it("renders spec_file_path as clickable link and calls onFileClick", () => {
		const onFileClick = vi.fn();
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
				onFileClick={onFileClick}
			/>,
		);
		const link = screen.getByText("docs/spec/issues-909.md");
		expect(link).toBeInTheDocument();
		fireEvent.click(link);
		expect(onFileClick).toHaveBeenCalledWith("docs/spec/issues-909.md");
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
		const badges = screen.getAllByText("NEEDS_FIX");
		expect(badges.length).toBeGreaterThanOrEqual(1);
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
		expect(screen.getByText("Result: then")).toBeInTheDocument();
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
		expect(screen.getByText("Result: reject")).toBeInTheDocument();
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
