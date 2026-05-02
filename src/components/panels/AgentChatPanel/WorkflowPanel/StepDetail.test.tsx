import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkflowState } from "@/types/workflow";

const { StepDetail } = await import("./StepDetail");

function makeWorkflowState(
	overrides: Partial<WorkflowState> = {},
): WorkflowState {
	return {
		executionId: "exec-001",
		workflowName: "test-workflow",
		state: { type: "completed" },
		currentStepIndex: 0,
		currentStepName: "step-1",
		totalSteps: 2,
		stepHistory: [],
		stepExecutionCounts: {},
		workflowDefinition: {
			name: "test-workflow",
			description: "test",
			builtin: false,
			steps: [
				{ name: "step-1", mode: "auto", prompt: "p1", rules: [] },
				{ name: "step-2", mode: "approval", prompt: "p2", rules: [] },
			],
		},
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		stepStates: {},
		startedAt: 1000,
		updatedAt: 2000,
		...overrides,
	};
}

describe("StepDetail", () => {
	it("shows 'Not executed' when step has no history entries", () => {
		render(
			<StepDetail stepName="step-1" workflowState={makeWorkflowState()} />,
		);
		expect(screen.getByText("Not executed")).toBeInTheDocument();
	});

	it("shows execution history entries for a step", () => {
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "LGTM",
					tokenUsage: { inputTokens: 50, outputTokens: 30 },
				},
				{
					stepName: "step-1",
					completedAt: 1002,
					result: "NEEDS_FIX",
					tokenUsage: { inputTokens: 60, outputTokens: 40 },
				},
			],
		});
		render(<StepDetail stepName="step-1" workflowState={ws} />);
		expect(screen.getByText("#1")).toBeInTheDocument();
		expect(screen.getByText("#2")).toBeInTheDocument();
		expect(screen.getByText("LGTM")).toBeInTheDocument();
		expect(screen.getByText("NEEDS_FIX")).toBeInTheDocument();
	});

	it("shows token usage for each entry", () => {
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "ok",
					tokenUsage: { inputTokens: 100, outputTokens: 200 },
				},
			],
		});
		render(<StepDetail stepName="step-1" workflowState={ws} />);
		expect(screen.getByText("300 tokens")).toBeInTheDocument();
	});

	it("shows View button when entry has sessionId and onSessionClick is provided", () => {
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "ok",
					sessionId: "sess-abc",
				},
			],
		});
		render(
			<StepDetail
				stepName="step-1"
				workflowState={ws}
				onSessionClick={vi.fn()}
			/>,
		);
		expect(screen.getByText("View")).toBeInTheDocument();
	});

	it("calls onSessionClick with sessionId when View is clicked", () => {
		const onSessionClick = vi.fn();
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "ok",
					sessionId: "sess-abc",
				},
			],
		});
		render(
			<StepDetail
				stepName="step-1"
				workflowState={ws}
				onSessionClick={onSessionClick}
			/>,
		);
		fireEvent.click(screen.getByText("View"));
		expect(onSessionClick).toHaveBeenCalledWith("sess-abc");
	});

	it("does not show View button when sessionId is absent", () => {
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "ok",
				},
			],
		});
		render(
			<StepDetail
				stepName="step-1"
				workflowState={ws}
				onSessionClick={vi.fn()}
			/>,
		);
		expect(screen.queryByText("View")).not.toBeInTheDocument();
	});

	it("filters entries for the specific step only", () => {
		const ws = makeWorkflowState({
			stepHistory: [
				{
					stepName: "step-1",
					completedAt: 1001,
					result: "result-1",
				},
				{
					stepName: "step-2",
					completedAt: 1002,
					result: "result-2",
				},
			],
		});
		render(<StepDetail stepName="step-1" workflowState={ws} />);
		expect(screen.getByText("result-1")).toBeInTheDocument();
		expect(screen.queryByText("result-2")).not.toBeInTheDocument();
	});
});
