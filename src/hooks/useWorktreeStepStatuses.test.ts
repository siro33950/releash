import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeStepStatusView } from "@/types/workspace-tree";
import {
	useWorktreeStepStatuses,
	workflowStepStatusKey,
} from "./useWorktreeStepStatuses";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

function view(
	overrides: Partial<WorktreeStepStatusView> = {},
): WorktreeStepStatusView {
	return {
		worktreePath: "/tmp/wt",
		version: 1,
		steps: [
			{
				executionId: "run-1",
				stepName: "build",
				runIndex: 1,
				representative: "running",
			},
		],
		workflows: [{ executionId: "run-1", representative: "running" }],
		...overrides,
	};
}

describe("useWorktreeStepStatuses", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns empty maps when worktreePath is null", () => {
		const { result } = renderHook(() => useWorktreeStepStatuses(null));
		expect(result.current.steps.size).toBe(0);
		expect(result.current.workflows.size).toBe(0);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("requests an initial ordered status event for the worktree", async () => {
		mockInvoke.mockResolvedValue(undefined);

		renderHook(() => useWorktreeStepStatuses("/tmp/wt"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("sync_worktree_step_statuses", {
				worktreePath: "/tmp/wt",
			});
		});
	});

	it("maps backend-owned statuses from ordered events", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Cb = (event: { payload: WorktreeStepStatusView }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(cb).not.toBeNull();
		});

		await act(async () => {
			cb?.({ payload: view() });
		});

		expect(result.current.steps.get("run-1:build:1")).toBe("running");
		expect(result.current.workflows.get("run-1")).toBe("running");
	});

	it("does not read sync command return values", async () => {
		mockInvoke.mockResolvedValue(
			view({
				steps: [
					{
						executionId: "from-command",
						stepName: "ignored",
						runIndex: 1,
						representative: "failed",
					},
				],
				workflows: [{ executionId: "from-command", representative: "failed" }],
			}),
		);
		type Cb = (event: { payload: WorktreeStepStatusView }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(cb).not.toBeNull();
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("sync_worktree_step_statuses", {
				worktreePath: "/tmp/wt",
			});
		});

		expect(result.current.steps.size).toBe(0);
		expect(result.current.workflows.size).toBe(0);
	});

	it("replaces maps with matching ordered events in arrival order", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Cb = (event: { payload: WorktreeStepStatusView }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(cb).not.toBeNull();
		});

		await act(async () => {
			cb?.({ payload: view({ version: 2, steps: [], workflows: [] }) });
		});

		expect(result.current.steps.size).toBe(0);
		expect(result.current.workflows.size).toBe(0);

		await act(async () => {
			cb?.({
				payload: view({
					version: 1,
					steps: [
						{
							executionId: "run-2",
							stepName: "test",
							runIndex: 1,
							representative: "waiting",
						},
					],
					workflows: [{ executionId: "run-2", representative: "waiting" }],
				}),
			});
		});

		expect(result.current.steps.get("run-1:build:1")).toBeUndefined();
		expect(result.current.steps.get("run-2:test:1")).toBe("waiting");
		expect(result.current.workflows.get("run-2")).toBe("waiting");
	});

	it("ignores live snapshots for other worktrees", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Cb = (event: { payload: WorktreeStepStatusView }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(cb).not.toBeNull();
		});

		await act(async () => {
			cb?.({ payload: view() });
		});

		await act(async () => {
			cb?.({
				payload: view({
					worktreePath: "/tmp/other",
					steps: [
						{
							executionId: "run-other",
							stepName: "test",
							runIndex: 1,
							representative: "failed",
						},
					],
					workflows: [{ executionId: "run-other", representative: "failed" }],
				}),
			});
		});

		expect(result.current.steps.size).toBe(1);
		expect(result.current.steps.get("run-other:test:1")).toBeUndefined();
		expect(result.current.workflows.get("run-other")).toBeUndefined();
	});

	it("builds keys with run index fallback", () => {
		expect(workflowStepStatusKey("run", "step", null)).toBe("run:step:1");
		expect(workflowStepStatusKey("run", "step", 3)).toBe("run:step:3");
	});
});
