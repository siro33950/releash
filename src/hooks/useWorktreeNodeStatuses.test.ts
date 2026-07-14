import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeNodeStatusView } from "@/types/workspace-tree";
import { useWorktreeNodeStatuses } from "./useWorktreeNodeStatuses";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

function view(
	overrides: Partial<WorktreeNodeStatusView> = {},
): WorktreeNodeStatusView {
	return {
		worktreePath: "/tmp/worktree",
		version: 1,
		nodeExecutions: [
			{
				nodeExecutionId: "node-build-1",
				executionId: "execution-1",
				nodeName: "build",
				attempt: 1,
				representative: "running",
			},
		],
		workflowExecutions: [
			{ executionId: "execution-1", representative: "running" },
		],
		...overrides,
	};
}

describe("useWorktreeNodeStatuses", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns empty maps when worktreePath is null", () => {
		const { result } = renderHook(() => useWorktreeNodeStatuses(null));
		expect(result.current.nodes.size).toBe(0);
		expect(result.current.executions.size).toBe(0);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("requests an initial ordered status event for the worktree", async () => {
		mockInvoke.mockResolvedValue(undefined);

		renderHook(() => useWorktreeNodeStatuses("/tmp/worktree"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("sync_worktree_node_statuses", {
				worktreePath: "/tmp/worktree",
			});
		});
	});

	it("maps backend-owned statuses from ordered events", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Callback = (event: { payload: WorktreeNodeStatusView }) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-node-status-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() =>
			useWorktreeNodeStatuses("/tmp/worktree"),
		);
		await waitFor(() => {
			expect(callback).not.toBeNull();
		});

		await act(async () => {
			callback?.({ payload: view() });
		});

		expect(result.current.nodes.get("node-build-1")).toBe("running");
		expect(result.current.executions.get("execution-1")).toBe("running");
	});

	it("does not read sync command return values", async () => {
		mockInvoke.mockResolvedValue(
			view({
				nodeExecutions: [
					{
						nodeExecutionId: "node-ignored-1",
						executionId: "execution-command",
						nodeName: "ignored",
						attempt: 1,
						representative: "failed",
					},
				],
				workflowExecutions: [
					{ executionId: "execution-command", representative: "failed" },
				],
			}),
		);
		type Callback = (event: { payload: WorktreeNodeStatusView }) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-node-status-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() =>
			useWorktreeNodeStatuses("/tmp/worktree"),
		);
		await waitFor(() => {
			expect(callback).not.toBeNull();
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("sync_worktree_node_statuses", {
				worktreePath: "/tmp/worktree",
			});
		});

		expect(result.current.nodes.size).toBe(0);
		expect(result.current.executions.size).toBe(0);
	});

	it("replaces maps with matching ordered events in arrival order", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Callback = (event: { payload: WorktreeNodeStatusView }) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-node-status-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() =>
			useWorktreeNodeStatuses("/tmp/worktree"),
		);
		await waitFor(() => {
			expect(callback).not.toBeNull();
		});

		await act(async () => {
			callback?.({
				payload: view({
					version: 2,
					nodeExecutions: [],
					workflowExecutions: [],
				}),
			});
		});

		expect(result.current.nodes.size).toBe(0);
		expect(result.current.executions.size).toBe(0);

		await act(async () => {
			callback?.({
				payload: view({
					version: 1,
					nodeExecutions: [
						{
							nodeExecutionId: "node-test-1",
							executionId: "execution-2",
							nodeName: "test",
							attempt: 1,
							representative: "waiting",
						},
					],
					workflowExecutions: [
						{ executionId: "execution-2", representative: "waiting" },
					],
				}),
			});
		});

		expect(result.current.nodes.get("node-build-1")).toBeUndefined();
		expect(result.current.nodes.get("node-test-1")).toBe("waiting");
		expect(result.current.executions.get("execution-2")).toBe("waiting");
	});

	it("ignores live snapshots for other worktrees", async () => {
		mockInvoke.mockResolvedValue(undefined);
		type Callback = (event: { payload: WorktreeNodeStatusView }) => void;
		let callback: Callback | null = null;
		mockListen.mockImplementation((event: string, listener: Callback) => {
			if (event === "workflow-node-status-changed") callback = listener;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() =>
			useWorktreeNodeStatuses("/tmp/worktree"),
		);
		await waitFor(() => {
			expect(callback).not.toBeNull();
		});

		await act(async () => {
			callback?.({ payload: view() });
			callback?.({
				payload: view({
					worktreePath: "/tmp/other",
					nodeExecutions: [
						{
							nodeExecutionId: "node-other-1",
							executionId: "execution-other",
							nodeName: "test",
							attempt: 1,
							representative: "failed",
						},
					],
					workflowExecutions: [
						{
							executionId: "execution-other",
							representative: "failed",
						},
					],
				}),
			});
		});

		expect(result.current.nodes.size).toBe(1);
		expect(result.current.nodes.get("node-other-1")).toBeUndefined();
		expect(result.current.executions.get("execution-other")).toBeUndefined();
	});
});
