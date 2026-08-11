import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeBranch } from "@/types/git";
import { useWorktreeList } from "./useWorktreeList";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeBranch = (
	overrides: Partial<WorktreeBranch> = {},
): WorktreeBranch => ({
	name: "feat/test",
	worktree_path: "/tmp/wt",
	is_main_worktree: false,
	is_merged: false,
	has_upstream: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	base_ahead: 0,
	dirty_count: 0,
	...overrides,
});

function setupMockInvoke(branches: WorktreeBranch[]) {
	mockInvoke.mockImplementation((cmd: string) => {
		if (cmd === "start_git_dir_watching") return Promise.resolve(42);
		if (cmd === "stop_watching") return Promise.resolve();
		if (cmd === "list_branches_with_status_snapshot")
			return Promise.resolve({
				version: 1,
				stale: false,
				loading: false,
				limited: false,
				branches,
			});
		if (cmd === "get_cached_pr_status")
			return Promise.resolve({ open_prs: {}, merged_branches: [] });
		return Promise.resolve([]);
	});
}

describe("useWorktreeList", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		setupMockInvoke([]);
	});

	it("should start git dir watcher on mount", async () => {
		renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});
	});

	it("should stop watcher on unmount", async () => {
		const { unmount } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});

		unmount();

		expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
			watcherId: 42,
		});
	});

	it("should restart watcher when repoPath changes", async () => {
		const { rerender } = renderHook(
			({ repoPath }: { repoPath: string }) => useWorktreeList(repoPath),
			{ initialProps: { repoPath: "/test/repo-a" } },
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo-a",
			});
		});

		const nextWatcherId = 99;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching")
				return Promise.resolve(nextWatcherId);
			if (cmd === "stop_watching") return Promise.resolve();
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		rerender({ repoPath: "/test/repo-b" });

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
				watcherId: 42,
			});
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo-b",
			});
		});
	});

	it("should not crash when watcher start fails", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching")
				return Promise.reject(new Error("repo not found"));
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(consoleSpy).toHaveBeenCalledWith(
				"Failed to start git dir watcher:",
				expect.any(Error),
			);
		});

		expect(result.current.branches).toEqual([]);
		consoleSpy.mockRestore();
	});

	it("should use 120s poll interval", async () => {
		vi.useFakeTimers();
		renderHook(() => useWorktreeList("/test/repo"));

		await act(async () => {
			await vi.waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"list_branches_with_status_snapshot",
					{
						repoPath: "/test/repo",
					},
				);
			});
		});

		const callCountBefore = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;

		await act(async () => {
			vi.advanceTimersByTime(30_000);
		});

		const callCountAfter30s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;
		expect(callCountAfter30s).toBe(callCountBefore);

		await act(async () => {
			vi.advanceTimersByTime(90_000);
		});

		const callCountAfter120s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;
		expect(callCountAfter120s).toBe(callCountBefore + 1);

		vi.useRealTimers();
	});

	it("should stop watcher if unmounted before watcher start resolves", async () => {
		let resolveStart: (id: number) => void = () => {};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching") {
				return new Promise<number>((resolve) => {
					resolveStart = resolve;
				});
			}
			if (cmd === "stop_watching") return Promise.resolve();
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		const { unmount } = renderHook(() => useWorktreeList("/test/repo"));
		unmount();

		await act(async () => {
			resolveStart(77);
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
				watcherId: 77,
			});
		});
	});

	it("should set loading to true on initial load then false after fetch", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		expect(result.current.loading).toBe(true);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.branches).toHaveLength(1);
		expect(result.current.branches[0].name).toBe("feat/test");
	});

	it("should not set loading to true when refresh is called with silent: true", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data so refresh triggers a state update
		const updatedBranch = makeBranch({ dirty_count: 5 });
		setupMockInvoke([updatedBranch]);

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		// loading should never have become true during silent refresh
		expect(result.current.loading).toBe(false);
		expect(result.current.branches[0].dirty_count).toBe(5);
	});

	it("should set loading to true when refresh is called without silent", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data
		const updatedBranch = makeBranch({ dirty_count: 3 });
		setupMockInvoke([updatedBranch]);

		await act(async () => {
			await result.current.refresh();
		});

		// After completion, loading is false
		expect(result.current.loading).toBe(false);
		expect(result.current.branches[0].dirty_count).toBe(3);
	});

	it("should skip setBranches when data has not changed", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(1);
		});

		const firstBranches = result.current.branches;

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		// Same reference because data didn't change
		expect(result.current.branches).toBe(firstBranches);
	});

	it("should update branches when data changes", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(1);
		});

		const firstBranches = result.current.branches;

		// Return different data
		const newBranch = makeBranch({
			name: "feat/new",
			worktree_path: "/tmp/wt2",
		});
		setupMockInvoke([branch, newBranch]);

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		expect(result.current.branches).not.toBe(firstBranches);
		expect(result.current.branches).toHaveLength(2);
	});

	it("should filter out branches without worktree_path but include main worktree branches", async () => {
		const branches = [
			makeBranch({ name: "main", is_main_worktree: true }),
			makeBranch({
				name: "feat/a",
				worktree_path: null as unknown as string,
			}),
			makeBranch({ name: "feat/b", worktree_path: "/tmp/b" }),
		];
		setupMockInvoke(branches);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(2);
		});

		const names = result.current.branches.map((b) => b.name);
		expect(names).toContain("main");
		expect(names).toContain("feat/b");
		expect(names).not.toContain("feat/a");
	});

	it("should pass is_main_worktree through to branches when main repo is on feature branch", async () => {
		const branches = [
			makeBranch({
				name: "main",
				is_main_worktree: false,
				worktree_path: null as unknown as string,
			}),
			makeBranch({
				name: "feat/current",
				is_main_worktree: true,
				worktree_path: "/repo",
			}),
			makeBranch({
				name: "feat/wt",
				is_main_worktree: false,
				worktree_path: "/tmp/wt",
			}),
		];
		setupMockInvoke(branches);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(2);
		});

		const mainWt = result.current.branches.find((b) => b.is_main_worktree);
		expect(mainWt).toBeDefined();
		expect(mainWt?.name).toBe("feat/current");
		expect(mainWt?.is_main_worktree).toBe(true);
	});

	it("should call refresh with silent: true from branch-list-sync event", async () => {
		setupMockInvoke([makeBranch()]);

		type ListenCallback = () => void;
		let branchListSyncCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "branch-list-sync") {
				branchListSyncCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data so we can verify the event triggers refresh
		const updatedBranch = makeBranch({ dirty_count: 7 });
		setupMockInvoke([updatedBranch]);

		expect(branchListSyncCallback).not.toBeNull();

		await act(async () => {
			branchListSyncCallback?.();
		});

		await waitFor(() => {
			expect(result.current.branches[0].dirty_count).toBe(7);
		});

		// loading should remain false (silent refresh)
		expect(result.current.loading).toBe(false);
	});
});
