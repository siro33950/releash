import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorktreeList } from "./useWorktreeList";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

describe("useWorktreeList", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching") return Promise.resolve(42);
			if (cmd === "stop_watching") return Promise.resolve();
			if (cmd === "list_branches_with_status") return Promise.resolve([]);
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			if (cmd === "get_agent_states") return Promise.resolve({});
			return Promise.resolve();
		});
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
			if (cmd === "list_branches_with_status") return Promise.resolve([]);
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			if (cmd === "get_agent_states") return Promise.resolve({});
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
			if (cmd === "list_branches_with_status") return Promise.resolve([]);
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			if (cmd === "get_agent_states") return Promise.resolve({});
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
				expect(mockInvoke).toHaveBeenCalledWith("list_branches_with_status", {
					repoPath: "/test/repo",
				});
			});
		});

		const callCountBefore = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status",
		).length;

		await act(async () => {
			vi.advanceTimersByTime(30_000);
		});

		const callCountAfter30s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status",
		).length;
		expect(callCountAfter30s).toBe(callCountBefore);

		await act(async () => {
			vi.advanceTimersByTime(90_000);
		});

		const callCountAfter120s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status",
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
			if (cmd === "list_branches_with_status") return Promise.resolve([]);
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			if (cmd === "get_agent_states") return Promise.resolve({});
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
});
