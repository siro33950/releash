import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowState, WorkflowStatePayload } from "@/types/workflow";
import { useWorkflowState } from "./useWorkflowState";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeState = (overrides: Partial<WorkflowState> = {}): WorkflowState => ({
	executionId: "exec-1",
	workflowName: "test-wf",
	state: { type: "running" },
	currentStepIndex: 0,
	currentStepName: "plan",
	totalSteps: 2,
	stepHistory: [],
	stepExecutionCounts: {},
	stepOutputs: {},
	workflowDefinition: {
		name: "test-wf",
		description: "",
		builtin: false,
		nodes: [],
	},
	totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
	stepStates: {},
	startedAt: 1000,
	updatedAt: 1000,
	...overrides,
});

describe("useWorkflowState", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns null when worktreePath is undefined", () => {
		const { result } = renderHook(() => useWorkflowState(undefined));
		expect(result.current.workflowState).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("fetches initial state via get_workflow_state", async () => {
		const state = makeState();
		mockInvoke.mockResolvedValue(state);

		const { result } = renderHook(() => useWorkflowState("/repo"));

		await waitFor(() => {
			expect(result.current.workflowState).toEqual(state);
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_workflow_state", {
			worktreePath: "/repo",
		});
	});

	it("updates state when matching workflow-state-changed fires", async () => {
		mockInvoke.mockResolvedValue(makeState());

		type Cb = (event: { payload: WorkflowStatePayload }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-state-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkflowState("/repo"));
		await waitFor(() => {
			expect(result.current.workflowState).not.toBeNull();
		});

		const updatedState = makeState({
			currentStepIndex: 1,
			currentStepName: "implement",
		});
		await act(async () => {
			cb?.({
				payload: { worktreePath: "/repo", workflowState: updatedState },
			});
		});

		expect(result.current.workflowState?.currentStepIndex).toBe(1);
	});

	it("ignores events for other worktrees", async () => {
		mockInvoke.mockResolvedValue(makeState());

		type Cb = (event: { payload: WorkflowStatePayload }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-state-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkflowState("/repo"));
		await waitFor(() => {
			expect(result.current.workflowState).not.toBeNull();
		});

		const otherState = makeState({ currentStepIndex: 99 });
		await act(async () => {
			cb?.({
				payload: { worktreePath: "/other-repo", workflowState: otherState },
			});
		});

		expect(result.current.workflowState?.currentStepIndex).toBe(0);
	});

	it("resets to null when worktreePath changes to undefined", async () => {
		mockInvoke.mockResolvedValue(makeState());

		const { result, rerender } = renderHook(
			({ wt }: { wt: string | undefined }) => useWorkflowState(wt),
			{ initialProps: { wt: "/repo" as string | undefined } },
		);

		await waitFor(() => {
			expect(result.current.workflowState).not.toBeNull();
		});

		rerender({ wt: undefined });
		expect(result.current.workflowState).toBeNull();
	});

	it("ignores stale response when worktreePath changes", async () => {
		let resolveFirst: ((v: WorkflowState) => void) | null = null;
		const firstPromise = new Promise<WorkflowState>((r) => {
			resolveFirst = r;
		});

		mockInvoke
			.mockImplementationOnce(() => firstPromise)
			.mockResolvedValue(makeState({ currentStepIndex: 2 }));

		const { result, rerender } = renderHook(
			({ wt }: { wt: string | undefined }) => useWorkflowState(wt),
			{ initialProps: { wt: "/repo-a" as string | undefined } },
		);

		// Change worktreePath before the first invoke resolves
		rerender({ wt: "/repo-b" });

		await waitFor(() => {
			expect(result.current.workflowState?.currentStepIndex).toBe(2);
		});

		// Now resolve the stale first request — should be ignored
		await act(async () => {
			resolveFirst?.(makeState({ currentStepIndex: 99 }));
		});

		// State should still be from /repo-b, not the stale /repo-a response
		expect(result.current.workflowState?.currentStepIndex).toBe(2);
	});

	it("handles invoke failure gracefully", async () => {
		mockInvoke.mockRejectedValue(new Error("fail"));
		const consoleSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

		const { result } = renderHook(() => useWorkflowState("/repo"));

		await waitFor(() => {
			expect(consoleSpy).toHaveBeenCalledWith(
				"[useWorkflowState] get_workflow_state failed",
				expect.any(Error),
			);
		});
		expect(result.current.workflowState).toBeNull();
		consoleSpy.mockRestore();
	});
});
