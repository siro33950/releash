import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowStepStatusChange } from "@/types/workspace-tree";
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

function change(
	overrides: Partial<WorkflowStepStatusChange> = {},
): WorkflowStepStatusChange {
	return {
		worktreePath: "/tmp/wt",
		executionId: "run-1",
		stepName: "build",
		runIndex: 1,
		representative: "running",
		workflowRepresentative: "running",
		version: 1,
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

	it("filters initial statuses by worktreePath", async () => {
		mockInvoke.mockResolvedValue([
			change(),
			change({
				worktreePath: "/tmp/other",
				executionId: "run-other",
				stepName: "test",
			}),
		]);

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));

		await waitFor(() => {
			expect(result.current.steps.size).toBe(1);
		});
		expect(result.current.steps.get("run-1:build:1")).toBe("running");
		expect(result.current.workflows.get("run-1")).toBe("running");
	});

	it("applies matching live changes and removals", async () => {
		mockInvoke.mockResolvedValue([change({ representative: "queued" })]);
		type Cb = (event: { payload: WorkflowStepStatusChange }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.steps.get("run-1:build:1")).toBe("queued");
		});

		await act(async () => {
			cb?.({ payload: change({ representative: "waiting" }) });
		});
		expect(result.current.steps.get("run-1:build:1")).toBe("waiting");
		expect(result.current.workflows.get("run-1")).toBe("running");

		await act(async () => {
			cb?.({
				payload: change({
					representative: null,
					workflowRepresentative: null,
				}),
			});
		});
		expect(result.current.steps.get("run-1:build:1")).toBeUndefined();
		expect(result.current.workflows.get("run-1")).toBeUndefined();
	});

	it("does not restore older initial statuses after a newer removal event", async () => {
		type Cb = (event: { payload: WorkflowStepStatusChange }) => void;
		let cb: Cb | null = null;
		let resolveInitial: ((value: WorkflowStepStatusChange[]) => void) | null =
			null;
		mockInvoke.mockReturnValue(
			new Promise<WorkflowStepStatusChange[]>((resolve) => {
				resolveInitial = resolve;
			}),
		);
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(cb).not.toBeNull();
		});

		await act(async () => {
			cb?.({
				payload: change({
					representative: null,
					workflowRepresentative: null,
					version: 2,
				}),
			});
		});

		await act(async () => {
			resolveInitial?.([
				change({
					representative: "queued",
					version: 1,
				}),
			]);
		});

		expect(result.current.steps.get("run-1:build:1")).toBeUndefined();
		expect(result.current.workflows.get("run-1")).toBeUndefined();
	});

	it("ignores live changes for other worktrees", async () => {
		mockInvoke.mockResolvedValue([change()]);
		type Cb = (event: { payload: WorkflowStepStatusChange }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workflow-step-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeStepStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.steps.size).toBe(1);
		});

		await act(async () => {
			cb?.({
				payload: change({
					worktreePath: "/tmp/other",
					executionId: "run-other",
					stepName: "test",
					representative: "failed",
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
