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

/**
 * Spec issues-1011 finding 13: 二段階 invoke の契約を厳密に検証するため、
 * `resolve_active_run_by_worktree` と `get_workflow_state` の戻り値を
 * command ごとに分離して mock するヘルパー。
 * 1 つの `mockResolvedValue` で全 command を同じ戻り値にする旧 mock は、
 * 両 command が同じ shape を返してしまい契約逸脱を検知できない。
 */
const mockResolveAndState = (
	runId: string | null,
	state: WorkflowState | null,
) => {
	mockInvoke.mockImplementation((cmd: string) => {
		if (cmd === "resolve_active_run_by_worktree") {
			return Promise.resolve(runId);
		}
		if (cmd === "get_workflow_state") {
			return Promise.resolve(state);
		}
		return Promise.resolve(null);
	});
};

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
		mockResolveAndState("exec-1", state);

		const { result } = renderHook(() => useWorkflowState("/repo"));

		await waitFor(() => {
			expect(result.current.workflowState).toEqual(state);
		});
		expect(mockInvoke).toHaveBeenCalledWith("resolve_active_run_by_worktree", {
			worktreePath: "/repo",
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_workflow_state", {
			runId: "exec-1",
		});
	});

	it("updates state when matching workflow-state-changed fires", async () => {
		mockResolveAndState("exec-1", makeState());

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
		mockResolveAndState("exec-1", makeState());

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
		mockResolveAndState("exec-1", makeState());

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
		// 1 つ目の worktree への resolve_active_run_by_worktree が pending な状態を作る。
		let resolveFirst: ((v: string | null) => void) | null = null;
		const firstPromise = new Promise<string | null>((r) => {
			resolveFirst = r;
		});

		// 後続の二段階 invoke を command ごとに振り分け、stale な /repo-a 応答が
		// 入った後でも /repo-b の state が保持されることを直接検証する。
		mockInvoke.mockImplementation(
			(cmd: string, args?: { runId?: string; worktreePath?: string }) => {
				if (cmd === "resolve_active_run_by_worktree") {
					if (args?.worktreePath === "/repo-a") return firstPromise;
					if (args?.worktreePath === "/repo-b")
						return Promise.resolve("exec-b");
					return Promise.resolve(null);
				}
				if (cmd === "get_workflow_state") {
					if (args?.runId === "exec-b") {
						return Promise.resolve(makeState({ currentStepIndex: 2 }));
					}
					if (args?.runId === "exec-a-stale") {
						return Promise.resolve(makeState({ currentStepIndex: 99 }));
					}
				}
				return Promise.resolve(null);
			},
		);

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
			resolveFirst?.("exec-a-stale");
		});

		// State should still be from /repo-b, not the stale /repo-a response
		expect(result.current.workflowState?.currentStepIndex).toBe(2);
	});

	it("handles invoke failure gracefully", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "resolve_active_run_by_worktree") {
				return Promise.reject(new Error("fail"));
			}
			return Promise.resolve(null);
		});
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
