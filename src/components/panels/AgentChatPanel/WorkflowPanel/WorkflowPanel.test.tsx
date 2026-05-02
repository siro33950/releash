import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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
				{ name: "step-1", mode: "auto", prompt: "p1", rules: [] },
				{ name: "step-2", mode: "approval", prompt: "p2", rules: [] },
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
				})}
				worktreePath="/repo"
				chatSessionId="session-1"
			/>,
		);
		expect(
			screen.getByRole("button", { name: "Approve step" }),
		).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Reject step" }),
		).toBeInTheDocument();
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
		});
	});

	it("invokes approve_workflow_step with reject when Reject is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "waiting_approval" },
				})}
				worktreePath="/repo"
				chatSessionId="session-1"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Reject step" }));
		expect(mockInvoke).toHaveBeenCalledWith("approve_workflow_step", {
			worktreePath: "/repo",
			decision: "reject",
		});
	});

	it("shows Complete button for interactive mode when running", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "running" },
					currentStepIndex: 0,
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "step-1",
								mode: "interactive",
								prompt: "p1",
								rules: [],
							},
							{ name: "step-2", mode: "auto", prompt: "p2", rules: [] },
						],
					},
				})}
				worktreePath="/repo"
				chatSessionId="session-1"
			/>,
		);
		expect(
			screen.getByRole("button", { name: "Complete step" }),
		).toBeInTheDocument();
	});

	it("invokes complete_interactive_step when Complete is clicked", () => {
		render(
			<WorkflowPanel
				workflowState={makeWorkflowState({
					state: { type: "running" },
					currentStepIndex: 0,
					workflowDefinition: {
						name: "test-workflow",
						description: "test",
						builtin: false,
						steps: [
							{
								name: "step-1",
								mode: "interactive",
								prompt: "p1",
								rules: [],
							},
							{ name: "step-2", mode: "auto", prompt: "p2", rules: [] },
						],
					},
				})}
				worktreePath="/repo"
				chatSessionId="session-1"
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Complete step" }));
		expect(mockInvoke).toHaveBeenCalledWith("complete_interactive_step", {
			worktreePath: "/repo",
			abort: false,
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
