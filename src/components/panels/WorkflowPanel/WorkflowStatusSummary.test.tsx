import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkflowState } from "@/types/workflow";
import { WorkflowStatusSummary } from "./WorkflowStatusSummary";

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
			nodes: [
				{ name: "plan", type: "agent", instruction: "plan", rules: [] },
				{ name: "review", type: "approval", instruction: "review", rules: [] },
				{
					name: "parallel-review",
					rules: [],
					type: "parallel",
					parallel_children: [
						{ name: "arch-review", type: "agent" },
						{ name: "security-review", type: "agent" },
					],
				},
			],
		},
		totalTokenUsage: { inputTokens: 100, outputTokens: 200 },
		stepStates: {
			plan: "running",
			review: "pending",
			"parallel-review": "pending",
		},
		startedAt: 1000,
		updatedAt: 2000,
		...overrides,
	};
}

describe("WorkflowStatusSummary", () => {
	it.each([
		{
			name: "running",
			workflowState: makeWorkflowState({
				state: { type: "running" },
				currentStepName: "plan",
			}),
			label: "Running plan",
			details: "agent step",
		},
		{
			name: "waiting for approval",
			workflowState: makeWorkflowState({
				state: { type: "waiting_approval" },
				currentStepName: "review",
			}),
			label: "Waiting for approval: review",
			details: "approval step",
		},
		{
			name: "completed",
			workflowState: makeWorkflowState({
				state: { type: "completed" },
				currentStepName: "",
				stepHistory: [
					{ stepName: "plan", completedAt: 1001, result: "done" },
					{ stepName: "review", completedAt: 1002, result: "approved" },
				],
			}),
			label: "Workflow completed",
			details: "2 recorded steps",
		},
		{
			name: "failed",
			workflowState: makeWorkflowState({
				state: { type: "failed", reason: "review rejected the change" },
				currentStepName: "review",
			}),
			label: "Workflow failed",
			details: "review rejected the change",
		},
		{
			name: "aborted",
			workflowState: makeWorkflowState({
				state: { type: "aborted" },
				currentStepName: "parallel-review",
			}),
			label: "Workflow aborted",
			details: "parallel step",
		},
	])("renders $name status label, details, and token usage", ({
		workflowState,
		label,
		details,
	}) => {
		render(<WorkflowStatusSummary workflowState={workflowState} />);

		expect(screen.getByTestId("workflow-status-summary")).toBeInTheDocument();
		expect(screen.getByText(label)).toBeInTheDocument();
		expect(screen.getByText(details)).toBeInTheDocument();
		expect(screen.getByText("300 tokens")).toBeInTheDocument();
	});
});
