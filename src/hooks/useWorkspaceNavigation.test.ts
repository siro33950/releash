import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceStatus } from "@/types/session";
import { useWorkspaceNavigation } from "./useWorkspaceNavigation";

const mockListen = vi.fn();
const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const makeStatus = (
	overrides: Partial<WorkspaceStatus> = {},
): WorkspaceStatus => ({
	worktree_id: "/repo/a",
	worktree_path: "/repo/a",
	aggregated_state: "running",
	running_count: 1,
	waiting_count: 0,
	error_count: 0,
	session_count: 1,
	last_activity_at: 1_000,
	...overrides,
});

describe("useWorkspaceNavigation", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("openWorktreeTab は同一 rootPath を再利用し異なる rootPath は追加する", () => {
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/a", "main", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/b", "feat/b", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/a", "ignored", "other");
		});

		expect(result.current.worktrees).toHaveLength(2);
		expect(result.current.worktrees[0]).toMatchObject({
			id: "/repo/a",
			rootPath: "/repo/a",
			branchName: "main",
			repoName: "repo",
		});
		expect(result.current.worktrees[1]).toMatchObject({
			id: "/repo/b",
			rootPath: "/repo/b",
			branchName: "feat/b",
		});
		expect(result.current.selectedWorktreeId).toBe("/repo/a");
	});

	it("close_quit_workspace_close_is_view_only", () => {
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/active", "main", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/other", "feature", "repo");
		});
		const retainedWorkspace = result.current.worktrees[0];

		act(() => {
			result.current.closeWorktreeTab("/repo/other");
		});

		expect(result.current.worktrees).toEqual([retainedWorkspace]);
		expect(result.current.worktrees[0]).toBe(retainedWorkspace);
		expect(result.current.selectedWorktreeId).toBe("/repo/active");
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("workspace-status-changed は一致するタブだけ agentState を更新する", async () => {
		type WorkspaceStatusCallback = (event: {
			payload: WorkspaceStatus;
		}) => void;
		let workspaceStatusCallback: WorkspaceStatusCallback | null = null;
		mockListen.mockImplementation(
			(event: string, callback: WorkspaceStatusCallback) => {
				if (event === "workspace-status-changed") {
					workspaceStatusCallback = callback;
				}
				return Promise.resolve(vi.fn());
			},
		);
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/a", "main", "repo");
			result.current.openWorktreeTab("/repo/b", "feat/b", "repo");
		});
		await waitFor(() => {
			expect(workspaceStatusCallback).not.toBeNull();
		});

		await act(async () => {
			workspaceStatusCallback?.({
				payload: makeStatus({
					worktree_id: "/repo/a",
					worktree_path: "/repo/a",
					aggregated_state: "error",
				}),
			});
		});

		expect(
			result.current.worktrees.find((tab) => tab.rootPath === "/repo/a")
				?.agentState,
		).toBe("error");
		expect(
			result.current.worktrees.find((tab) => tab.rootPath === "/repo/b")
				?.agentState,
		).toBeUndefined();

		await act(async () => {
			workspaceStatusCallback?.({
				payload: makeStatus({
					worktree_id: "/repo/missing",
					worktree_path: "/repo/missing",
					aggregated_state: "running",
				}),
			});
		});

		expect(
			result.current.worktrees.find((tab) => tab.rootPath === "/repo/b")
				?.agentState,
		).toBeUndefined();
	});

	it("workspace-status-changed の恒等更新では worktrees の同一参照を維持する", async () => {
		type WorkspaceStatusCallback = (event: {
			payload: WorkspaceStatus;
		}) => void;
		let workspaceStatusCallback: WorkspaceStatusCallback | null = null;
		mockListen.mockImplementation(
			(event: string, callback: WorkspaceStatusCallback) => {
				if (event === "workspace-status-changed") {
					workspaceStatusCallback = callback;
				}
				return Promise.resolve(vi.fn());
			},
		);
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/a", "main", "repo");
			result.current.openWorktreeTab("/repo/b", "feat/b", "repo");
		});
		await waitFor(() => {
			expect(workspaceStatusCallback).not.toBeNull();
		});

		await act(async () => {
			workspaceStatusCallback?.({
				payload: makeStatus({ aggregated_state: "running" }),
			});
		});
		const updatedWorktrees = result.current.worktrees;
		expect(
			updatedWorktrees.find((tab) => tab.rootPath === "/repo/a")?.agentState,
		).toBe("running");

		await act(async () => {
			workspaceStatusCallback?.({
				payload: makeStatus({ aggregated_state: "running" }),
			});
		});

		expect(result.current.worktrees).toBe(updatedWorktrees);
	});

	it("unmount 時に workspace-status-changed の unlisten を呼ぶ", async () => {
		const unlisten = vi.fn();
		mockListen.mockResolvedValue(unlisten);
		const { unmount } = renderHook(() => useWorkspaceNavigation());

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"workspace-status-changed",
				expect.any(Function),
			);
		});
		await act(async () => {});

		unmount();

		expect(unlisten).toHaveBeenCalledTimes(1);
	});
});
