import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	WorkflowExecution,
	WorkflowExecutionChangedPayload,
} from "@/types/workflow";
import { useWorkflowState } from "./useWorkflowState";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeExecution = (
	overrides: Partial<WorkflowExecution> = {},
): WorkflowExecution => ({
	id: "execution-1",
	workflowName: "test-workflow",
	status: "running",
	currentNode: "plan",
	worktreePath: "/repo",
	createdFrom: "desktop_ui",
	startedAt: 1_000,
	updatedAt: 1_000,
	completedAt: null,
	errorReason: null,
	totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
	nodeExecutions: [],
	artifacts: [],
	fanouts: [],
	approvalTarget: null,
	...overrides,
});

const mockResolveAndExecution = (
	executionId: string | null,
	execution: WorkflowExecution | null,
) => {
	mockInvoke.mockImplementation((command: string) => {
		if (command === "resolve_active_execution_by_worktree") {
			return Promise.resolve(executionId);
		}
		if (command === "get_workflow_execution_state") {
			return Promise.resolve(execution);
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
		expect(result.current.workflowExecution).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("fetches the active WorkflowExecution", async () => {
		const execution = makeExecution();
		mockResolveAndExecution("execution-1", execution);

		const { result } = renderHook(() => useWorkflowState("/repo"));

		await waitFor(() => {
			expect(result.current.workflowExecution).toEqual(execution);
		});
		expect(mockInvoke).toHaveBeenCalledWith(
			"resolve_active_execution_by_worktree",
			{ worktreePath: "/repo" },
		);
		expect(mockInvoke).toHaveBeenCalledWith("get_workflow_execution_state", {
			worktreePath: "/repo",
			executionId: "execution-1",
		});
	});

	it("updates when a matching WorkflowExecution event arrives", async () => {
		mockResolveAndExecution("execution-1", makeExecution());

		type Callback = (event: {
			payload: WorkflowExecutionChangedPayload;
		}) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-execution-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkflowState("/repo"));
		await waitFor(() => {
			expect(result.current.workflowExecution).not.toBeNull();
		});

		const updatedExecution = makeExecution({ currentNode: "implement" });
		await act(async () => {
			callback?.({
				payload: {
					worktreePath: "/repo",
					workflowExecution: updatedExecution,
				},
			});
		});

		expect(result.current.workflowExecution?.currentNode).toBe("implement");
	});

	it("ignores events for other worktrees", async () => {
		mockResolveAndExecution("execution-1", makeExecution());

		type Callback = (event: {
			payload: WorkflowExecutionChangedPayload;
		}) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-execution-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkflowState("/repo"));
		await waitFor(() => {
			expect(result.current.workflowExecution).not.toBeNull();
		});

		await act(async () => {
			callback?.({
				payload: {
					worktreePath: "/other-repo",
					workflowExecution: makeExecution({ currentNode: "other" }),
				},
			});
		});

		expect(result.current.workflowExecution?.currentNode).toBe("plan");
	});

	it("resets to null when worktreePath becomes undefined", async () => {
		mockResolveAndExecution("execution-1", makeExecution());

		const { result, rerender } = renderHook(
			({ worktreePath }: { worktreePath: string | undefined }) =>
				useWorkflowState(worktreePath),
			{ initialProps: { worktreePath: "/repo" as string | undefined } },
		);

		await waitFor(() => {
			expect(result.current.workflowExecution).not.toBeNull();
		});

		rerender({ worktreePath: undefined });
		expect(result.current.workflowExecution).toBeNull();
	});

	it("ignores stale responses after switching worktrees", async () => {
		let resolveFirst: ((value: string | null) => void) | null = null;
		const firstResolution = new Promise<string | null>((resolve) => {
			resolveFirst = resolve;
		});

		mockInvoke.mockImplementation(
			(
				command: string,
				args?: { executionId?: string; worktreePath?: string },
			) => {
				if (command === "resolve_active_execution_by_worktree") {
					if (args?.worktreePath === "/repo-a") return firstResolution;
					if (args?.worktreePath === "/repo-b") {
						return Promise.resolve("execution-b");
					}
					return Promise.resolve(null);
				}
				if (command === "get_workflow_execution_state") {
					if (args?.executionId === "execution-b") {
						return Promise.resolve(makeExecution({ currentNode: "build" }));
					}
					if (args?.executionId === "execution-a-stale") {
						return Promise.resolve(makeExecution({ currentNode: "stale" }));
					}
				}
				return Promise.resolve(null);
			},
		);

		const { result, rerender } = renderHook(
			({ worktreePath }: { worktreePath: string | undefined }) =>
				useWorkflowState(worktreePath),
			{ initialProps: { worktreePath: "/repo-a" as string | undefined } },
		);

		rerender({ worktreePath: "/repo-b" });
		await waitFor(() => {
			expect(result.current.workflowExecution?.currentNode).toBe("build");
		});

		await act(async () => {
			resolveFirst?.("execution-a-stale");
		});

		expect(result.current.workflowExecution?.currentNode).toBe("build");
	});

	it("handles invoke failure gracefully", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "resolve_active_execution_by_worktree") {
				return Promise.reject(new Error("fail"));
			}
			return Promise.resolve(null);
		});
		const consoleSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

		const { result } = renderHook(() => useWorkflowState("/repo"));

		await waitFor(() => {
			expect(consoleSpy).toHaveBeenCalledWith(
				"[useWorkflowState] get_workflow_execution_state failed",
				expect.any(Error),
			);
		});
		expect(result.current.workflowExecution).toBeNull();
		consoleSpy.mockRestore();
	});
});
