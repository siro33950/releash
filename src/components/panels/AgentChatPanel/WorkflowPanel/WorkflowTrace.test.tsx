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
				{ name: "plan", mode: "auto", prompt: "p1", rules: [] },
				{ name: "review", mode: "approval", prompt: "p2", rules: [] },
				{ name: "fix", mode: "interactive", prompt: "p3", rules: [] },
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

	it("shows Output toggle when stepHistory entry has outputText", () => {
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
							outputText: "This is the step output text",
						},
					],
				})}
			/>,
		);
		expect(screen.getByText("Output")).toBeInTheDocument();
	});

	it("expands outputText on Output toggle click", () => {
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
							outputText: "Full output text content",
						},
					],
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Output"));
		expect(screen.getByText("Full output text content")).toBeInTheDocument();
	});

	it("shows ReduceResultBadge for collect step", () => {
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
								prompt: "p1",
								rules: [],
								collect: {
									from: ["a", "b"],
									reduce: "any_needs_fix",
								},
							},
							{
								name: "review",
								mode: "approval",
								prompt: "p2",
								rules: [],
							},
							{
								name: "fix",
								mode: "interactive",
								prompt: "p3",
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
});
